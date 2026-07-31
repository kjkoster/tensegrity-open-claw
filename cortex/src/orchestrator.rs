//! The per-frame DMX loop, shared by every rig: tick, ask the rig's show to fill a slot
//! buffer, decide who owns the universe, and emit one frame to both transports.
//!
//! What the rig contributes is the fill and nothing else. That is the whole seam — no show
//! trait, no stage vocabulary — because a boundary carrying more meaning than "the rig fills
//! the slot buffer" would formalise two shows as two code bases and invite them to drift
//! apart.
//!
//! This is also where an external console takes the rig over. Ownership is decided once per
//! frame and one slot buffer goes to both outputs, so the wire and the network can never be
//! driven from different sources.

use crate::Rig;
use crate::audio_features::AudioFeatures;
use crate::clock;
use crate::config as cfg;
use crate::dmx;
use crate::latest::LatestRx;
use crate::sacn_in::Takeover;
use embassy_time::{Duration, Ticker};
use std::net::UdpSocket;
use zihatec_rs_485_dmx::{DmxHat, DmxTiming};

#[embassy_executor::task]
pub async fn frame_task(
    socket: UdpSocket,
    cid: [u8; 16],
    features: LatestRx<AudioFeatures>,
    takeover: LatestRx<Takeover>,
    mut rig: Rig,
) -> ! {
    // The wired HAT mirrors the same slot buffer as the sACN send. A broken serial setup is
    // a deploy-time gate, not a runtime hazard, so panic with the crate's remediation
    // message rather than limp along without the wire.
    let mut hat = DmxHat::open(cfg::SERIAL_DEVICE, DmxTiming::default())
        .unwrap_or_else(|e| panic!("dmx hat: {}: {e}", cfg::SERIAL_DEVICE));

    let dmx_slots = crate::frame_width(rig.patch);

    let frame_period = Duration::from_micros(1_000_000 / cfg::FRAME_RATE_HZ);
    let mut ticker = Ticker::every(frame_period);
    let mut sequence: u8 = 0;
    let dt = 1.0 / cfg::FRAME_RATE_HZ as f64;
    let mut overridden = false;

    loop {
        ticker.next().await;
        let snapshot = features.snapshot();

        // A full 512-slot universe for the wire; the sACN frame carries only the live head.
        // Slots outside a patched fixture's block stay zero, and so do the ones the rig's
        // show says nothing about.
        let mut slots = [0u8; zihatec_rs_485_dmx::SLOTS];
        (rig.show)(&snapshot, dt, &mut slots);

        // The show above runs every frame whether or not it is driving. Its noise fields,
        // breath phase and slews stay continuous, so handback resumes mid-stride instead of
        // springing out of a frozen frame — and the audio pipeline never stops feeding it
        // either way.
        //
        // One decision, one buffer: everything below ships whatever `slots` holds, to both
        // the wire and the network, so the two can never follow different sources. Takeover
        // is whole-universe — the external frame replaces every slot, never some of them.
        let external = takeover.snapshot();
        let external_driving = external.in_force(clock::now_us());
        if external_driving {
            slots = external.slots;
        }

        if external_driving != overridden {
            overridden = external_driving;
            if overridden {
                eprintln!("sacn: takeover by {}", external.describe());
            } else {
                eprintln!(
                    "sacn: released by {} — internal engine resumes",
                    external.describe()
                );
            }
        }

        let packet = dmx::encode(
            cfg::UNIVERSE,
            sequence,
            cfg::SACN_PRIORITY,
            0,
            &cid,
            &slots[..dmx_slots],
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
