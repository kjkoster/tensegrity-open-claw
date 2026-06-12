use alsa::pcm::{Access, Format, HwParams, PCM};
use alsa::{Direction, ValueOr};
use std::error::Error;
use std::thread::{self, JoinHandle};

// ── Capture config ───────────────────────────────────────────────────────────
const ALSA_DEVICE: &str = "plughw:CARD=io2,DEV=0"; // confirm with `arecord -L`
const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: usize = 2;
const CAPTURE_SECS: u32 = 60;
const READ_FRAMES: usize = 4096;

// ── USB identity of the Alesis io|2 (from your lsusb dump) ────────────────────
const USB_VENDOR: u16 = 0x13b2;
const USB_PRODUCT: u16 = 0x0008;

/// Spawns the capture on its own thread and returns immediately, so the embassy
/// executor / DMX loop keeps running. Detach the handle or join it as you like.
pub fn spawn_capture() -> JoinHandle<()> {
    thread::Builder::new()
        .name("audio-capture".into())
        .spawn(|| {
            probe_usb();
            if let Err(e) = capture_to_tmp() {
                eprintln!("audio: capture failed: {e}");
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

fn capture_to_tmp() -> Result<(), Box<dyn Error>> {
    eprintln!("audio: opening ALSA device \"{ALSA_DEVICE}\"…");
    let pcm = PCM::new(ALSA_DEVICE, Direction::Capture, false)?;

    let (rate, period) = {
        let hwp = HwParams::any(&pcm)?;
        eprintln!(
            "audio:   device offers {}–{} Hz, {}–{} channels",
            hwp.get_rate_min()?,
            hwp.get_rate_max()?,
            hwp.get_channels_min()?,
            hwp.get_channels_max()?,
        );
        hwp.set_channels(CHANNELS as u32)?;
        hwp.set_rate(SAMPLE_RATE, ValueOr::Nearest)?;
        hwp.set_format(Format::s16())?; // plug layer converts native 24-bit -> 16-bit
        hwp.set_access(Access::RWInterleaved)?;
        pcm.hw_params(&hwp)?;

        let cur = pcm.hw_params_current()?;
        (cur.get_rate()?, cur.get_period_size()?)
    };
    eprintln!("audio:   negotiated {rate} Hz, {CHANNELS} ch, S16_LE, period {period} frames");

    let io = pcm.io_i16()?;
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writers = Vec::with_capacity(CHANNELS);
    for ch in 0..CHANNELS {
        let path = format!("/tmp/capture.{ch}.wav");
        eprintln!("audio:   writing channel {ch} -> {path}");
        writers.push(hound::WavWriter::create(&path, spec)?);
    }

    let total = (rate * CAPTURE_SECS) as usize;
    let mut buf = vec![0i16; READ_FRAMES * CHANNELS];
    let mut done = 0usize;
    let mut next_mark = rate as usize * 10; // progress log every ~10s of audio

    eprintln!("audio: capturing {CAPTURE_SECS}s…");
    pcm.prepare()?;
    while done < total {
        let frames = match io.readi(&mut buf) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("audio:   xrun ({e}), recovering");
                pcm.try_recover(e, true)?;
                continue;
            }
        };
        for f in 0..frames {
            for ch in 0..CHANNELS {
                writers[ch].write_sample(buf[f * CHANNELS + ch])?;
            }
        }
        done += frames;
        if done >= next_mark {
            eprintln!(
                "audio:   {}s / {CAPTURE_SECS}s ({done} frames)",
                done / rate as usize
            );
            next_mark += rate as usize * 10;
        }
    }

    for (ch, w) in writers.into_iter().enumerate() {
        w.finalize()?;
        eprintln!("audio: finalized /tmp/capture.{ch}.wav");
    }
    eprintln!("audio: done — {total} frames per channel at {rate} Hz");
    Ok(())
}
