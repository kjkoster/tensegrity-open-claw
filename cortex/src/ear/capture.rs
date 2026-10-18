//! The ear's capture, on a dedicated OS thread: blocking ALSA reads pace the fast path, which
//! publishes a fresh AudioFeatures per period. The thread never dies — xruns recover in place,
//! a lost interface is probed for and reopened with backoff, and the sculpture keeps
//! breathing on stale snapshots in the meantime.

use super::AudioFeatures;
use super::pipeline::FeaturePipeline;
use crate::config as cfg;
use crate::latest::LatestTx;
use crate::telemetry::Publisher;
use alsa::pcm::{Access, Format, Frames, HwParams, PCM};
use alsa::{Direction, ValueOr};
use std::error::Error;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// What the ear last told the broker, so it only says what changed.
#[derive(Default)]
struct Reported {
    state: Option<u8>,
    xruns: Option<u64>,
    /// Blocks with at least one sample at full scale, since the brain started: the one level
    /// problem the AGC cannot fix and the features cannot show, because a clipped room still
    /// reads as loud music.
    clipped_blocks: u64,
    clipping: Option<u64>,
}

/// Spawns the capture on its own thread and returns immediately, so the frame loop keeps
/// running.
pub(super) fn spawn(features: LatestTx<AudioFeatures>, telemetry: Publisher) -> JoinHandle<()> {
    thread::Builder::new()
        .name("ear".into())
        .spawn(move || {
            let mut reported = Reported::default();
            let mut backoff_s = 1u64;
            loop {
                // Before every open, not just the first: an interface that dropped off the
                // bus and came back is a new address, and the broker should say so.
                probe_usb(&telemetry);
                let opened = Instant::now();
                if let Err(e) = capture(&features, &telemetry, &mut reported) {
                    eprintln!("ear: capture failed: {e}");
                    // Retained until the ear is listening again, so the broker says why it is
                    // deaf for as long as it is.
                    telemetry.publish("error", e.to_string(), true);
                }
                if opened.elapsed().as_secs() > 60 {
                    backoff_s = 1;
                }
                eprintln!("ear: reopening the interface in {backoff_s}s (sculpture keeps breathing)");
                thread::sleep(Duration::from_secs(backoff_s));
                backoff_s = (backoff_s * 2).min(cfg::DEVICE_RETRY_MAX_S);
            }
        })
        .expect("failed to spawn the ear's thread")
}

/// Finds the interface on the USB bus and publishes where it is and what it calls itself.
///
/// Best-effort: the name comes from the device's own string descriptors, which need the
/// device opened, and the IDs stand in when it cannot be. An interface that is not on the bus
/// clears both topics rather than leaving the last one standing.
fn probe_usb(telemetry: &Publisher) {
    let (vendor, product) = (cfg::EAR_USB_VENDOR, cfg::EAR_USB_PRODUCT);
    eprintln!("ear: scanning USB for {vendor:04x}:{product:04x}…");
    let list = match rusb::devices() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("ear: USB enumeration failed: {e}");
            return;
        }
    };

    for device in list.iter() {
        let desc = match device.device_descriptor() {
            Ok(d) => d,
            Err(_) => continue,
        };
        if desc.vendor_id() != vendor || desc.product_id() != product {
            continue;
        }

        let address = format!("{:03}:{:03}", device.bus_number(), device.address());
        eprintln!(
            "ear: found {vendor:04x}:{product:04x} at bus:device {address} (port {})",
            device.port_number(),
        );
        eprintln!(
            "ear:   USB {} spec, device rev {}, class {:#04x}",
            desc.usb_version(),
            desc.device_version(),
            desc.class_code(),
        );

        let name = match device.open() {
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
                eprintln!("ear:   probed name: \"{mfr} {prod}\"  serial: {serial}");
                format!("{mfr} {prod}")
            }
            Err(e) => {
                eprintln!(
                    "ear:   could not open the interface for its name ({e}); \
                     run as root or add a udev rule (capture still works)"
                );
                format!("{vendor:04x}:{product:04x}")
            }
        };
        telemetry.publish("usb/address", address, true);
        telemetry.publish("usb/device", name, true);
        return;
    }

    eprintln!("ear: WARNING — the interface is not on the USB bus; capture will likely fail");
    telemetry.publish("usb/address", "", true);
    telemetry.publish("usb/device", "", true);
}

/// Opens the interface and runs the capture/feature loop until an unrecoverable error. Only
/// returns on error; the caller reopens with backoff.
fn capture(
    features: &LatestTx<AudioFeatures>,
    telemetry: &Publisher,
    reported: &mut Reported,
) -> Result<(), Box<dyn Error>> {
    eprintln!("ear: opening ALSA device \"{}\"…", cfg::ALSA_DEVICE);
    let pcm = PCM::new(cfg::ALSA_DEVICE, Direction::Capture, false)?;

    let (rate, period) = {
        let hwp = HwParams::any(&pcm)?;
        eprintln!(
            "ear:   interface offers {}–{} Hz, {}–{} channels",
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
        "ear:   negotiated {rate} Hz, {} ch, S16_LE, period {period} frames",
        cfg::CHANNELS
    );
    // What the interface settled on, not what was asked for: another interface, or this one on
    // other firmware, can land elsewhere and shift the ear's timing without an error.
    telemetry.publish("usb/rate_hz", rate.to_string(), true);

    let io = pcm.io_i16()?;
    let mut pipeline = FeaturePipeline::new(rate as f32, period);
    let mut buf = vec![0i16; period * cfg::CHANNELS];
    let mut last_status = Instant::now();

    pcm.prepare()?;
    eprintln!("ear: listening — publishing AudioFeatures at block rate");
    // An empty retained payload deletes the topic: listening is the absence of an error.
    telemetry.publish("error", "", true);
    loop {
        match io.readi(&mut buf) {
            Ok(frames) => {
                let block = &buf[..frames * cfg::CHANNELS];
                if block.iter().any(|&s| s == i16::MAX || s == i16::MIN) {
                    reported.clipped_blocks += 1;
                }
                let state = pipeline.process_block(block);
                features.publish(state);
                report(telemetry, reported, &state);

                if last_status.elapsed().as_secs_f32() >= cfg::STATUS_INTERVAL_S {
                    last_status = Instant::now();
                    // At the status interval rather than on every change: a clipping room
                    // changes the count every block, a hundred times a second.
                    if reported.clipping != Some(reported.clipped_blocks) {
                        reported.clipping = Some(reported.clipped_blocks);
                        telemetry.publish("clipping", reported.clipped_blocks.to_string(), true);
                    }
                    eprintln!(
                        "ear: status {} music={:.2} energy={:.2} floor={:.4} agc_ref={:.4} \
                         bpm={:.0}/{:.2} onset_density={:.2} xruns={}",
                        word(state.state),
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
                eprintln!("ear:   xrun ({e}), recovering");
                pcm.try_recover(e, true)?;
            }
        }
    }
}

/// Publishes `ear/state` and `ear/xruns`, each only when it changed. The xrun count is since
/// the interface was last opened, as the pipeline keeps it.
fn report(telemetry: &Publisher, reported: &mut Reported, state: &AudioFeatures) {
    if reported.state != Some(state.state) {
        reported.state = Some(state.state);
        telemetry.publish("state", word(state.state).to_ascii_lowercase(), true);
    }
    if reported.xruns != Some(state.xrun_count) {
        reported.xruns = Some(state.xrun_count);
        telemetry.publish("xruns", state.xrun_count.to_string(), true);
    }
}

fn word(state: u8) -> &'static str {
    if state == AudioFeatures::MUSIC { "MUSIC" } else { "SILENCE" }
}
