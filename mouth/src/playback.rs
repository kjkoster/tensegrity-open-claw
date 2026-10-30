//! The mouth's playback, on a dedicated OS thread: blocking ALSA writes pace it, one period at
//! a time, and every period is mixed from the latest cue. An underrun recovers in place, and a
//! lost interface is probed for and reopened with backoff, the show running on silent in the
//! meantime. An interface that is there but cannot play the recordings stops the brain.

use crate::config as cfg;
use crate::{Cue, Sounds};
use alsa::pcm::{Access, Format, Frames, HwParams, PCM};
use alsa::{Direction, ValueOr};
use cortex::latest::LatestRx;
use cortex::telemetry::Publisher;
use rusb::{Device, GlobalContext};
use std::error::Error;
use std::thread;
use std::time::{Duration, Instant};

const CHANNELS: usize = 2;

/// What the mouth last told the broker, so it only says what changed.
#[derive(Default)]
struct Reported {
    state: Option<&'static str>,
    /// Underruns since the brain started.
    xruns: u64,
}

pub(crate) fn spawn<const N: usize>(
    sounds: &'static Sounds<N>,
    cues: LatestRx<Cue<N>>,
    telemetry: Publisher,
) {
    thread::Builder::new()
        .name("mouth".into())
        .spawn(move || {
            cortex::sched::realtime(cortex::config::AUDIO_CORES);
            let mut mixer = Mixer::new(sounds);
            let mut reported = Reported::default();
            telemetry.publish("xruns", "0", true);
            let mut backoff_s = 1u64;
            loop {
                // Before every open, not just the first: an interface that dropped off the
                // bus and came back is at a new address, and the broker should say so.
                probe_usb(&telemetry);
                let opened = Instant::now();
                if let Err(e) = play(sounds, &cues, &telemetry, &mut mixer, &mut reported) {
                    eprintln!("mouth: playback failed: {e}");
                    report_state(&telemetry, &mut reported, "down");
                    // Retained until the mouth plays again, so the broker says why it is
                    // silent for as long as it is.
                    telemetry.publish("error", e.to_string(), true);
                }
                if opened.elapsed().as_secs() > 60 {
                    backoff_s = 1;
                }
                eprintln!(
                    "mouth: reopening the interface in {backoff_s}s (the show runs on silent)"
                );
                thread::sleep(Duration::from_secs(backoff_s));
                backoff_s = (backoff_s * 2).min(cfg::DEVICE_RETRY_MAX_S);
            }
        })
        .expect("failed to spawn the mouth's thread");
}

/// Opens the interface and plays until it goes away, returning why; the caller reopens with
/// backoff. An interface that opens but will not play the recordings as they were built panics:
/// reopening it would only refuse them again.
fn play<const N: usize>(
    sounds: &'static Sounds<N>,
    cues: &LatestRx<Cue<N>>,
    telemetry: &Publisher,
    mixer: &mut Mixer<N>,
    reported: &mut Reported,
) -> Result<(), Box<dyn Error>> {
    eprintln!("mouth: opening ALSA device \"{}\"…", cfg::ALSA_DEVICE);
    let pcm = PCM::new(cfg::ALSA_DEVICE, Direction::Playback, false)?;
    let period = configure(&pcm, sounds, telemetry)
        .unwrap_or_else(|e| panic!("mouth: ALSA device \"{}\": {e}", cfg::ALSA_DEVICE));

    let io = pcm
        .io_i16()
        .unwrap_or_else(|e| panic!("mouth: ALSA device \"{}\": {e}", cfg::ALSA_DEVICE));
    let mut out = vec![0i16; period * CHANNELS];
    mixer.open(period, &cues.snapshot());
    pcm.prepare()?;
    // An empty retained payload deletes the topic: playing is the absence of an error.
    telemetry.publish("error", "", true);
    loop {
        let cue = cues.snapshot();
        mixer.fill(&cue, &mut out);
        report_state(
            telemetry,
            reported,
            if cue.mute { "muted" } else { "playing" },
        );

        let mut written = 0;
        while written < period {
            match io.writei(&out[written * CHANNELS..]) {
                Ok(frames) => written += frames,
                Err(e) => {
                    // An underrun recovers in place; anything else is returned, and the
                    // interface reopened.
                    pcm.try_recover(e, true)?;
                    reported.xruns += 1;
                    eprintln!("mouth:   xrun ({e}), recovered");
                    telemetry.publish("xruns", reported.xruns.to_string(), true);
                    break;
                }
            }
        }
    }
}

/// Sets the interface up for the recordings as they were built, and returns the period it
/// settled on.
fn configure<const N: usize>(
    pcm: &PCM,
    sounds: &'static Sounds<N>,
    telemetry: &Publisher,
) -> Result<usize, Box<dyn Error>> {
    {
        let hwp = HwParams::any(pcm)?;
        hwp.set_channels(CHANNELS as u32)?;
        // The recordings were resampled to the interface's rate at build time. Without this
        // the plug layer would resample again, silently, for an interface that cannot run at
        // it; with it, that interface opens at another rate, and the check below refuses it.
        hwp.set_rate_resample(false)?;
        hwp.set_rate(sounds.rate_hz, ValueOr::Nearest)?;
        hwp.set_format(Format::s16())?;
        hwp.set_access(Access::RWInterleaved)?;
        hwp.set_period_size_near(cfg::PERIOD_FRAMES as Frames, ValueOr::Nearest)?;
        hwp.set_buffer_size_near((cfg::PERIOD_FRAMES * cfg::PERIODS_PER_BUFFER) as Frames)?;
        pcm.hw_params(&hwp)?;
    }
    // What the interface settled on, not what was asked for: the sizes are rounded to what it
    // can do, and the latency with them.
    let (rate, period, buffer) = {
        let current = pcm.hw_params_current()?;
        (
            current.get_rate()?,
            current.get_period_size()? as usize,
            current.get_buffer_size()?,
        )
    };
    let card = pcm.info()?.get_card();
    eprintln!(
        "mouth:   card {card}, {rate} Hz, {CHANNELS} ch, S16_LE, period {period} frames, buffer {buffer}"
    );
    telemetry.publish("alsa/card", card.to_string(), true);
    telemetry.publish("alsa/rate_hz", rate.to_string(), true);
    telemetry.publish("alsa/period", period.to_string(), true);
    telemetry.publish("alsa/buffer", buffer.to_string(), true);
    if rate != sounds.rate_hz {
        return Err(format!(
            "the interface opened at {rate} Hz, and the recordings are at {} Hz",
            sounds.rate_hz
        )
        .into());
    }

    // Start only once the buffer is full, so the first period is not already late.
    let swp = pcm.sw_params_current()?;
    swp.set_start_threshold(buffer)?;
    pcm.sw_params(&swp)?;
    Ok(period)
}

fn report_state(telemetry: &Publisher, reported: &mut Reported, state: &'static str) {
    if reported.state != Some(state) {
        reported.state = Some(state);
        eprintln!("mouth: {state}");
        telemetry.publish("state", state, true);
    }
}

/// One recording playing.
#[derive(Clone, Copy)]
struct Voice {
    sound: usize,
    frame: usize,
}

/// Mixes the voices playing into each period.
struct Mixer<const N: usize> {
    sounds: &'static Sounds<N>,
    voices: [Option<Voice>; cfg::VOICES],
    /// The cue's trigger counts as last read.
    seen: [u32; N],
    /// One period of the mix, wide enough that voices summing past full scale do not wrap.
    mix: Vec<[i32; CHANNELS]>,
}

impl<const N: usize> Mixer<N> {
    fn new(sounds: &'static Sounds<N>) -> Self {
        Self {
            sounds,
            voices: [None; cfg::VOICES],
            seen: [0; N],
            mix: Vec::new(),
        }
    }

    /// Readies the mixer for a freshly opened interface. A sound cued while the interface was
    /// down is forgotten rather than started late.
    fn open(&mut self, period: usize, cue: &Cue<N>) {
        self.mix = vec![[0; CHANNELS]; period];
        self.voices = [None; cfg::VOICES];
        self.seen = cue.triggers;
    }

    /// Mixes the next period into `out`, interleaved.
    fn fill(&mut self, cue: &Cue<N>, out: &mut [i16]) {
        let seen = std::mem::replace(&mut self.seen, cue.triggers);
        if cue.mute {
            // A sound cued while muted is dropped, not held for the release.
            self.voices = [None; cfg::VOICES];
            out.fill(0);
            return;
        }
        for sound in 0..N {
            // However often it was cued since the last period, a sound starts once: a period
            // is far shorter than a frame, so more than one is a show asking twice at once.
            if cue.triggers[sound] != seen[sound] {
                self.start(sound);
            }
        }

        self.mix.fill([0; CHANNELS]);
        for slot in &mut self.voices {
            let Some(voice) = slot else { continue };
            let sound = self.sounds.all[voice.sound];
            let end = sound.frames.len().min(voice.frame + self.mix.len());
            for (mixed, frame) in self.mix.iter_mut().zip(voice.frame..end) {
                let [left, right] = sound.sample(frame);
                mixed[0] += i32::from(left);
                mixed[1] += i32::from(right);
            }
            voice.frame = end;
            if end == sound.frames.len() {
                *slot = None;
            }
        }
        for (frame, mixed) in out.chunks_exact_mut(CHANNELS).zip(&self.mix) {
            for (sample, &level) in frame.iter_mut().zip(mixed) {
                // Voices that sum past full scale clip rather than wrap.
                *sample = level.clamp(i16::MIN.into(), i16::MAX.into()) as i16;
            }
        }
    }

    /// Starts `sound` in a free voice, or in place of the one furthest into its recording.
    fn start(&mut self, sound: usize) {
        let slot = match self.voices.iter().position(Option::is_none) {
            Some(free) => free,
            None => (0..cfg::VOICES)
                .max_by_key(|&slot| self.voices[slot].map_or(0, |voice| voice.frame))
                .unwrap(),
        };
        self.voices[slot] = Some(Voice { sound, frame: 0 });
    }
}

/// The interface as the USB bus sees it, one topic each under `mouth/usb/`.
const USB_TOPICS: [&str; 7] = [
    "vid_pid", "bus", "address", "speed", "sysfs", "serial", "product",
];

/// Finds the interface on the USB bus and publishes what and where it is. An interface that
/// is not on the bus blanks every topic rather than leaving the last one standing.
fn probe_usb(telemetry: &Publisher) {
    let wanted = format!("{:04x}:{:04x}", cfg::USB_VENDOR, cfg::USB_PRODUCT);
    let values = match rusb::devices() {
        Ok(list) => match list.iter().find(|device| {
            device.device_descriptor().is_ok_and(|descriptor| {
                descriptor.vendor_id() == cfg::USB_VENDOR
                    && descriptor.product_id() == cfg::USB_PRODUCT
            })
        }) {
            Some(device) => describe(&device, wanted),
            None => {
                eprintln!("mouth: {wanted} is not on the USB bus; playback will likely fail");
                Default::default()
            }
        },
        // libusb itself, not the interface: it will not work on the next probe either.
        Err(e) => panic!("mouth: USB enumeration: {e}"),
    };
    for (topic, value) in USB_TOPICS.iter().zip(values) {
        telemetry.publish(format!("usb/{topic}"), value, true);
    }
}

/// The interface's readings, in the order of [`USB_TOPICS`].
fn describe(device: &Device<GlobalContext>, vid_pid: String) -> [String; 7] {
    let bus = device.bus_number().to_string();
    let address = device.address().to_string();
    let speed = format!("{:?}", device.speed()).to_ascii_lowercase();
    // The kernel names a device by its bus and the chain of hub ports down to it; blank when
    // the chain cannot be read, rather than a path to nothing.
    let sysfs = device
        .port_numbers()
        .map(|ports| {
            let ports: Vec<String> = ports.iter().map(u8::to_string).collect();
            format!("/sys/bus/usb/devices/{bus}-{}", ports.join("."))
        })
        .unwrap_or_default();
    // The strings need the device opened, which needs root or a udev rule; blank without.
    let (serial, product) = match (device.open(), device.device_descriptor()) {
        (Ok(handle), Ok(descriptor)) => (
            handle
                .read_serial_number_string_ascii(&descriptor)
                .unwrap_or_default(),
            handle
                .read_product_string_ascii(&descriptor)
                .unwrap_or_default(),
        ),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("mouth: could not read the interface's strings ({e})");
            Default::default()
        }
    };
    eprintln!(
        "mouth: found {vid_pid} \"{product}\" at {sysfs}, bus {bus} address {address}, {speed} speed"
    );
    [vid_pid, bus, address, speed, sysfs, serial, product]
}
