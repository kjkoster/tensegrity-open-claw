//! Audio capture on a dedicated OS thread (SOUND.md §3, §8.2): blocking ALSA
//! reads pace the fast path, which publishes a fresh ControlState per period.
//! The thread never dies — xruns recover in place, a lost device is reopened
//! with backoff, and the sculpture keeps breathing on stale snapshots.

use crate::config as cfg;
use crate::control::ControlPublisher;
use crate::features::FeaturePipeline;
use alsa::pcm::{Access, Format, Frames, HwParams, PCM};
use alsa::{Direction, ValueOr};
use std::error::Error;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

// ── USB identity of the Alesis io|2 ──────────────────────────────────────────
const USB_VENDOR: u16 = 0x13b2;
const USB_PRODUCT: u16 = 0x0008;

/// Spawns the capture on its own thread and returns immediately, so the embassy
/// executor / DMX loop keeps running. Detach the handle or join it as you like.
pub fn spawn_capture(publisher: ControlPublisher) -> JoinHandle<()> {
    thread::Builder::new()
        .name("audio-capture".into())
        .spawn(move || {
            probe_usb();
            let mut backoff_s = 1u64;
            loop {
                let opened = Instant::now();
                if let Err(e) = capture(&publisher) {
                    eprintln!("audio: capture failed: {e}");
                }
                if opened.elapsed().as_secs() > 60 {
                    backoff_s = 1;
                }
                eprintln!("audio: reopening device in {backoff_s}s (sculpture keeps breathing)");
                thread::sleep(Duration::from_secs(backoff_s));
                backoff_s = (backoff_s * 2).min(cfg::DEVICE_RETRY_MAX_S);
            }
        })
        .expect("failed to spawn audio-capture thread")
}

/// Best-effort USB identification. Logs bus/device/vendor/product and, if the
/// device can be opened, the on-device string descriptors (the authoritative
/// name — for the io|2 these are iManufacturer="Alesis", iProduct="io|2").
fn probe_usb() {
    eprintln!("audio: scanning USB for {USB_VENDOR:04x}:{USB_PRODUCT:04x} (Alesis io|2)…");
    let list = match rusb::devices() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("audio: USB enumeration failed: {e}");
            return;
        }
    };

    for device in list.iter() {
        let desc = match device.device_descriptor() {
            Ok(d) => d,
            Err(_) => continue,
        };
        if desc.vendor_id() != USB_VENDOR || desc.product_id() != USB_PRODUCT {
            continue;
        }

        eprintln!(
            "audio: found {:04x}:{:04x} on bus {:03} device {:03} (port {})",
            desc.vendor_id(),
            desc.product_id(),
            device.bus_number(),
            device.address(),
            device.port_number(),
        );
        eprintln!(
            "audio:   USB {} spec, device rev {}, class {:#04x}",
            desc.usb_version(),
            desc.device_version(),
            desc.class_code(),
        );

        match device.open() {
            Ok(handle) => {
                let mfr = handle
                    .read_manufacturer_string_ascii(&desc)
                    .unwrap_or_else(|_| "?".into());
                let prod = handle
                    .read_product_string_ascii(&desc)
                    .unwrap_or_else(|_| "?".into());
                let serial = handle
                    .read_serial_number_string_ascii(&desc)
                    .unwrap_or_else(|_| "(none)".into());
                eprintln!("audio:   probed name: \"{mfr} {prod}\"  serial: {serial}");
            }
            Err(e) => eprintln!(
                "audio:   could not open device for name probe ({e}); \
                 run as root or add a udev rule (capture still works)"
            ),
        }
        return;
    }

    eprintln!("audio: WARNING — io|2 not present on the USB bus; capture will likely fail");
}

/// Opens the device and runs the capture/feature loop until an unrecoverable
/// error. Only returns on error; the caller reopens with backoff.
fn capture(publisher: &ControlPublisher) -> Result<(), Box<dyn Error>> {
    eprintln!("audio: opening ALSA device \"{}\"…", cfg::ALSA_DEVICE);
    let pcm = PCM::new(cfg::ALSA_DEVICE, Direction::Capture, false)?;

    let (rate, period) = {
        let hwp = HwParams::any(&pcm)?;
        eprintln!(
            "audio:   device offers {}–{} Hz, {}–{} channels",
            hwp.get_rate_min()?,
            hwp.get_rate_max()?,
            hwp.get_channels_min()?,
            hwp.get_channels_max()?,
        );
        hwp.set_channels(cfg::CHANNELS as u32)?;
        hwp.set_rate(cfg::REQUESTED_RATE_HZ, ValueOr::Nearest)?;
        hwp.set_format(Format::s16())?; // plug layer converts native 24-bit -> 16-bit
        hwp.set_access(Access::RWInterleaved)?;
        hwp.set_period_size_near(cfg::PERIOD_FRAMES as Frames, ValueOr::Nearest)?;
        hwp.set_buffer_size_near((cfg::PERIOD_FRAMES * cfg::PERIODS_PER_BUFFER) as Frames)?;
        pcm.hw_params(&hwp)?;

        let cur = pcm.hw_params_current()?;
        (cur.get_rate()?, cur.get_period_size()? as usize)
    };
    eprintln!(
        "audio:   negotiated {rate} Hz, {} ch, S16_LE, period {period} frames",
        cfg::CHANNELS
    );

    let io = pcm.io_i16()?;
    let mut pipeline = FeaturePipeline::new(rate as f32, period);
    let mut buf = vec![0i16; period * cfg::CHANNELS];
    let mut last_status = Instant::now();

    pcm.prepare()?;
    eprintln!("audio: capture running — publishing ControlState at block rate");
    loop {
        match io.readi(&mut buf) {
            Ok(frames) => {
                let state = pipeline.process_block(&buf[..frames * cfg::CHANNELS]);
                publisher.publish(state);

                if last_status.elapsed().as_secs_f32() >= cfg::STATUS_INTERVAL_S {
                    last_status = Instant::now();
                    eprintln!(
                        "audio: status {} music={:.2} energy={:.2} floor={:.4} agc_ref={:.4} \
                         bpm={:.0}/{:.2} onset_density={:.2} xruns={}",
                        if state.state == 1 { "MUSIC" } else { "SILENCE" },
                        state.music_amount,
                        state.energy,
                        state.noise_floor,
                        state.agc_ref,
                        state.bpm,
                        state.tempo_confidence,
                        state.onset_density,
                        state.xrun_count,
                    );
                }
            }
            Err(e) => {
                pipeline.note_xrun();
                eprintln!("audio:   xrun ({e}), recovering");
                pcm.try_recover(e, true)?;
            }
        }
    }
}
