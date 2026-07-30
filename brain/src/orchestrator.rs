//! Orchestrator stage (SPARKLE.md §0.3, §6): the per-frame DMX loop. Reads the latest
//! `AudioFeatures`, runs the sparkle mapping for the pinspot and a `LaserMapping` for the
//! laser, fills the slot array, and emits one sACN packet at the frame rate. The three Yara
//! pars are pinned to hard R/G/B primaries for bring-up (see fill_yara).

use crate::audio_features::AudioFeatures;
use crate::config as cfg;
use crate::dmx;
use crate::laser::{LaserMapping, LaserOut};
use crate::latest::LatestRx;
use crate::patch;
use crate::qlc_plus::{Rgb, White};
use crate::sparkle::{SparkleMapping, SparkleOut};
use embassy_time::{Duration, Ticker};
use std::net::UdpSocket;
use zihatec_rs_485_dmx::{DmxHat, DmxTiming};

#[embassy_executor::task]
pub async fn noise_task(socket: UdpSocket, cid: [u8; 16], features: LatestRx<AudioFeatures>) -> ! {
    let mut laser_map = LaserMapping::default();

    // The sparkle engine: silence breathing under a colour drift, glinting on musical
    // onsets, with its own slow white-mode gate. One instance per fixture, so several
    // fixtures sparkle independently rather than in lock-step (SPARKLE.md §6).
    let mut pinspot_map = SparkleMapping::new(
        cfg::SEEDS[0],
        cfg::SEEDS[1],
        cfg::SEEDS[2],
        cfg::WHITE_MODE_PERLIN_SEED,
    );

    // The wired HAT mirrors the same slot buffer as the sACN send (HARDWARE-DMX.md).
    // A broken serial setup is a deploy-time gate, not a runtime hazard, so panic
    // with the crate's remediation message rather than limp along without the wire.
    let mut hat = DmxHat::open(cfg::SERIAL_DEVICE, DmxTiming::default())
        .unwrap_or_else(|e| panic!("dmx hat: {}: {e}", cfg::SERIAL_DEVICE));

    let frame_period = Duration::from_micros(1_000_000 / cfg::FRAME_RATE_HZ);
    let mut ticker = Ticker::every(frame_period);
    let mut sequence: u8 = 0;
    let dt = 1.0 / cfg::FRAME_RATE_HZ as f64;

    loop {
        ticker.next().await;
        let snapshot = features.snapshot();

        // Every fixture named here comes from the QLC+ patch (see patch.rs), so this list is
        // also the record of which patched fixtures the daemon actually drives. Slots outside
        // a fixture's block stay zero. The buffer is a full 512-slot universe for the wire;
        // the sACN frame sends only the live head.
        let mut slots = [0u8; zihatec_rs_485_dmx::SLOTS];
        fill_sparkle(&mut slots, &patch::PINSPOT, &pinspot_map.frame(&snapshot, dt));
        // The pinspot's other two channels are its own, not the engine's: Effect below 64
        // is what hands the emitters to the colour channels at all, and Speed only feeds
        // the internal programs that Effect would start.
        patch::PINSPOT.effect.set(&mut slots, 0);
        patch::PINSPOT.speed.set(&mut slots, 0);

        fill_laser(&mut slots, &laser_map.frame(dt));
        fill_yara(&mut slots, &patch::YARA_1, [255, 0, 0]); // red
        fill_yara(&mut slots, &patch::YARA_2, [0, 255, 0]); // green
        fill_yara(&mut slots, &patch::YARA_3, [0, 0, 255]); // blue

        let packet = dmx::encode(
            cfg::UNIVERSE,
            sequence,
            cfg::SACN_PRIORITY,
            0,
            &cid,
            &slots[..patch::DMX_SLOTS],
        );
        dmx::send_multicast(&socket, cfg::UNIVERSE, cfg::SACN_PORT, &packet);
        // A wire write error is logged, not fatal — a yanked cable must not take the
        // sculpture down.
        if let Err(e) = hat.send(&slots) {
            eprintln!("dmx hat: write error: {e}");
        }
        sequence = sequence.wrapping_add(1);
    }
}

/// Drive any colour-mixing fixture from the sparkle engine.
///
/// Generic over [`Rgb`], so it accepts exactly those fixtures whose patched mode carries
/// red, green and blue — and the compiler refuses the rest rather than letting the engine
/// write colour into whatever channel happens to sit at that offset.
///
/// Intensity is folded into the colours rather than sent to a dimmer, because the fixtures
/// this drives have none. It is gamma-corrected first: that curve is about perceived
/// brightness, which is what the breathing is shaped in, and the colours ride on it.
///
/// The engine's white mode deliberately darkens RGB to hand the light to a white emitter.
/// On a fixture without one that would just black it out for the length of every
/// white-sparkle phrase, so it is rendered as equal parts red, green and blue instead.
fn fill_sparkle<F: Rgb>(slots: &mut [u8], fixture: &F, out: &SparkleOut) {
    let level = out.intensity.powf(1.0 / cfg::GAMMA);
    let (r, g, b) = if out.white > 0.0 {
        (out.white, out.white, out.white)
    } else {
        (out.r, out.g, out.b)
    };
    fixture.red().set_unit(slots, r * level);
    fixture.green().set_unit(slots, g * level);
    fixture.blue().set_unit(slots, b * level);
}

/// Fill the laser's channels from its CH1..CH8 values. The laser's map is positional — it
/// has no colour to name — so this walks the mode's channels in order.
fn fill_laser(slots: &mut [u8], out: &LaserOut) {
    for (channel, &value) in patch::LASER.all().iter().zip(out.channels.iter()) {
        channel.set(slots, value);
    }
}

/// Pin one Yara par to a hard primary for bring-up. Needs both colour and a white emitter,
/// so the white can be held off rather than left wherever it was.
fn fill_yara<F: Rgb + White>(slots: &mut [u8], fixture: &F, rgb: [u8; 3]) {
    fixture.red().set(slots, rgb[0]);
    fixture.green().set(slots, rgb[1]);
    fixture.blue().set(slots, rgb[2]);
    fixture.white().set(slots, 0);
}
