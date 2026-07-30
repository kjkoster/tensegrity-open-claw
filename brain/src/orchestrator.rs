//! Orchestrator stage (SPARKLE.md §0.3, §6): the per-frame DMX loop. Reads the
//! latest `AudioFeatures`, runs a `PonytailMapping` for each Ponytail and a `LaserMapping`
//! for the laser, fills the slot array, and emits one sACN packet at the frame rate. The
//! three Yara pars are pinned to hard R/G/B primaries for bring-up (see fill_yara).

use crate::audio_features::AudioFeatures;
use crate::config as cfg;
use crate::dmx;
use crate::fixture::Fixture;
use crate::laser::{LaserMapping, LaserOut};
use crate::latest::LatestRx;
use crate::sparkle::{PonytailMapping, PonytailOut};
use embassy_time::{Duration, Ticker};
use std::net::UdpSocket;
use zihatec_rs_485_dmx::{DmxHat, DmxTiming};

#[embassy_executor::task]
pub async fn noise_task(socket: UdpSocket, cid: [u8; 16], features: LatestRx<AudioFeatures>) -> ! {
    let ponytail_a = Fixture { start_address: 1 };
    let ponytail_b = Fixture { start_address: 7 };
    let ponytail_c = Fixture { start_address: 13 };
    let ponytail_d = Fixture { start_address: 19 };
    let laser = Fixture { start_address: cfg::LASER_ADDRESS };
    let mut laser_map = LaserMapping::default();

    // Three CLF Yara pars, patched high in the same universe (@100, @107, @113). For bring-up
    // they are pinned to hard, full-intensity primaries (red/green/blue) so the three are
    // trivially distinguishable and their DMX addressing is verifiable (see fill_yara).
    let yara_a = Fixture { start_address: cfg::YARA_ADDRESSES[0] };
    let yara_b = Fixture { start_address: cfg::YARA_ADDRESSES[1] };
    let yara_c = Fixture { start_address: cfg::YARA_ADDRESSES[2] };

    // Independent instances: own breath clock, colour seeds, and white-mode gate per
    // fixture, so the Ponytails sparkle and flip to white independently rather than in
    // lock-step (SPARKLE.md §6). Each fixture's white gate is seeded distinctly.
    let mut map_a = PonytailMapping::new(
        cfg::SEEDS[0],
        cfg::SEEDS[1],
        cfg::SEEDS[2],
        cfg::WHITE_MODE_PERLIN_SEED,
    );
    let mut map_b = PonytailMapping::new(
        cfg::SEEDS[4],
        cfg::SEEDS[5],
        cfg::SEEDS[6],
        cfg::WHITE_MODE_PERLIN_SEED ^ cfg::SEEDS[3],
    );
    let mut map_c = PonytailMapping::new(
        cfg::SEEDS[8],
        cfg::SEEDS[9],
        cfg::SEEDS[10],
        cfg::WHITE_MODE_PERLIN_SEED ^ cfg::SEEDS[11],
    );
    let mut map_d = PonytailMapping::new(
        cfg::SEEDS[12],
        cfg::SEEDS[13],
        cfg::SEEDS[14],
        cfg::WHITE_MODE_PERLIN_SEED ^ cfg::SEEDS[15],
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
        let a = map_a.frame(&snapshot, dt);
        let b = map_b.frame(&snapshot, dt);
        let c = map_c.frame(&snapshot, dt);
        let d = map_d.frame(&snapshot, dt);

        // One universe: the four Ponytails (A@1, B@7, C@13, D@19), the laser (@25), then the
        // three Yara pars (@100, @107, @113). Intervening slots stay zero. The buffer is a
        // full 512-slot universe for the wire; the sACN frame sends only the live head.
        let mut slots = [0u8; zihatec_rs_485_dmx::SLOTS];
        fill(&mut slots, &ponytail_a, &a);
        fill(&mut slots, &ponytail_b, &b);
        fill(&mut slots, &ponytail_c, &c);
        fill(&mut slots, &ponytail_d, &d);
        fill_laser(&mut slots, &laser, &laser_map.frame(dt));
        fill_yara(&mut slots, &yara_a, [255, 0, 0]); // @100 red
        fill_yara(&mut slots, &yara_b, [0, 255, 0]); // @107 green
        fill_yara(&mut slots, &yara_c, [0, 0, 255]); // @113 blue

        let packet = dmx::encode(
            cfg::UNIVERSE,
            sequence,
            cfg::SACN_PRIORITY,
            0,
            &cid,
            &slots[..cfg::DMX_SLOTS],
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

/// Fill one Ponytail's six DMX slots. Intensity is gamma-corrected onto the fixture's
/// linear ~100-level native brightness scale — spending those few levels perceptually
/// (rather than bunched at the dark end) is what keeps the breathing from banding. The
/// colour, white, and gobo channels are already perceptual 0..1 and scale linearly.
fn fill(slots: &mut [u8], fixture: &Fixture, out: &PonytailOut) {
    slots[fixture.slot(0)] = unit_to_byte(out.intensity.powf(1.0 / cfg::GAMMA));
    slots[fixture.slot(1)] = unit_to_byte(out.r);
    slots[fixture.slot(2)] = unit_to_byte(out.g);
    slots[fixture.slot(3)] = unit_to_byte(out.b);
    slots[fixture.slot(4)] = unit_to_byte(out.white);
    slots[fixture.slot(5)] = unit_to_byte(out.gobo);
}

/// Round a 0..1 value to a DMX byte. Round, not truncate, to match the fixture-side
/// scaling.
fn unit_to_byte(x: f64) -> u8 {
    (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// Fill the laser's eight DMX slots from its CH1..CH8 values, starting at its address.
fn fill_laser(slots: &mut [u8], laser: &Fixture, out: &LaserOut) {
    for (offset, &value) in out.channels.iter().enumerate() {
        slots[laser.slot(offset as u16)] = value;
    }
}

/// Fill one Yara par's four DMX slots with a hard primary. The Yaras run in 4-channel mode
/// (R, G, B, White), so the colour maps straight onto the first three slots with white held
/// off. There is no dimmer channel, so a full colour channel is full output.
fn fill_yara(slots: &mut [u8], fixture: &Fixture, rgb: [u8; 3]) {
    slots[fixture.slot(0)] = rgb[0]; // red
    slots[fixture.slot(1)] = rgb[1]; // green
    slots[fixture.slot(2)] = rgb[2]; // blue
    slots[fixture.slot(3)] = 0; // white off
}
