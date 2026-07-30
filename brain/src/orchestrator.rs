//! Orchestrator stage (SPARKLE.md §0.3, §6): the per-frame DMX loop. Runs a
//! `LaserMapping` for the laser, fills the slot array, and emits one sACN packet at the
//! frame rate. The three Yara pars are pinned to hard R/G/B primaries for bring-up (see
//! fill_yara).

use crate::audio_features::AudioFeatures;
use crate::config as cfg;
use crate::dmx;
use crate::fixture::Fixture;
use crate::laser::{LaserMapping, LaserOut};
use crate::latest::LatestRx;
use crate::patch;
use embassy_time::{Duration, Ticker};
use std::net::UdpSocket;
use zihatec_rs_485_dmx::{DmxHat, DmxTiming};

#[embassy_executor::task]
pub async fn noise_task(socket: UdpSocket, cid: [u8; 16], features: LatestRx<AudioFeatures>) -> ! {
    let mut laser_map = LaserMapping::default();

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

        // Every fixture named here comes from the QLC+ patch (see patch.rs), so this list is
        // also the record of which patched fixtures the daemon actually drives. Slots outside
        // a fixture's block stay zero. The buffer is a full 512-slot universe for the wire;
        // the sACN frame sends only the live head.
        let mut slots = [0u8; zihatec_rs_485_dmx::SLOTS];
        fill_laser(&mut slots, &patch::LASER, &laser_map.frame(dt));
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

/// Fill the laser's eight DMX slots from its CH1..CH8 values, starting at its address.
fn fill_laser(slots: &mut [u8], laser: &Fixture, out: &LaserOut) {
    for (offset, &value) in out.channels.iter().enumerate() {
        slots[laser.slot(offset as u16)] = value;
    }
}

// This code writes the Yaras' 4-channel mode by hand, so it has an opinion about their
// patched width. Pin it: repatch a Yara to one of its other modes (6CH, 7CH, 11CH…) in QLC+
// and the build stops here rather than quietly writing colour into a dimmer and a shutter.
const _: () = assert!(patch::YARA_1.channels == 4);
const _: () = assert!(patch::YARA_2.channels == 4);
const _: () = assert!(patch::YARA_3.channels == 4);

/// Fill one Yara par's four DMX slots with a hard primary. The Yaras run in 4-channel mode
/// (R, G, B, White), so the colour maps straight onto the first three slots with white held
/// off. There is no dimmer channel, so a full colour channel is full output.
fn fill_yara(slots: &mut [u8], fixture: &Fixture, rgb: [u8; 3]) {
    slots[fixture.slot(0)] = rgb[0]; // red
    slots[fixture.slot(1)] = rgb[1]; // green
    slots[fixture.slot(2)] = rgb[2]; // blue
    slots[fixture.slot(3)] = 0; // white off
}
