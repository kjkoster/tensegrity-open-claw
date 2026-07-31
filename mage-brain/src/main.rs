//! The mage rig: four moving heads, a pinspot and a laser, driven by pose detection from a
//! camera rather than by sound. Nothing in this show listens to the room.
//!
//! Day one is the pinspot alone, breathing purple, because the pinspot is the bench and
//! bring-up fixture and there is something to drive while the rest is being built. It reuses
//! the claw's breath period, floor and ceiling — one set of numbers in `cortex`, so the two
//! rigs cannot end up breathing differently by accident.

use cortex::Rig;
use cortex::audio_features::AudioFeatures;
use cortex::config as cfg;
use std::f64::consts::TAU;

// `patch` and `scenes`, generated from mage.qxw by the ingest. The workspace and the `.qxf`
// definitions are the source of truth; nothing about the addressing is hand-maintained.
// Generating them into this crate rather than the shared one is also what keeps the rigs
// apart: the claw's fixtures do not exist in this binary to be reached for. The camera is not
// a DMX device and appears in neither.
include!(concat!(env!("OUT_DIR"), "/rig.rs"));

fn main() {
    // Phase carried across frames rather than read off the wall clock, so the breath is
    // continuous through anything that stalls a frame, and wrapped to one period so it stays
    // exact however many days the rig runs.
    let mut phase_s = 0.0f64;

    cortex::run(Rig {
        name: env!("CARGO_PKG_NAME"),
        patch: &patch::PATCH,
        scenes: &scenes::SCENES,
        // The audio features are ignored, deliberately: the mage show is pose-driven and
        // there is no pose stage yet. The capture thread runs anyway, because it belongs to
        // the cabinet rather than to either rig.
        show: Box::new(move |_features: &AudioFeatures, dt: f64, slots: &mut [u8]| {
            phase_s = (phase_s + dt) % cfg::SPARKLE_BREATH_PERIOD_S;

            // A cosine breath between the floor and the ceiling. It never reaches zero: a
            // light that goes fully dark reads as broken, while one that keeps an ember
            // reads as alive.
            let breath = 0.5 - 0.5 * (TAU * phase_s / cfg::SPARKLE_BREATH_PERIOD_S).cos();
            let level = cfg::SPARKLE_BREATH_FLOOR
                + breath * (cfg::SPARKLE_BREATH_CEIL - cfg::SPARKLE_BREATH_FLOOR);
            // Gamma applies to the level, not to the mix: the breath is shaped in perceived
            // brightness, and the colour rides on it.
            let level = level.powf(1.0 / cfg::GAMMA);

            // Purple is red and blue together, with the green emitter held off rather than
            // left wherever it was.
            patch::PINSPOT.red.set_unit(slots, level);
            patch::PINSPOT.green.set(slots, 0);
            patch::PINSPOT.blue.set_unit(slots, level);
            // Effect below 64 is what hands the emitters to the colour channels at all, and
            // Speed only feeds the internal programs that Effect would start.
            patch::PINSPOT.effect.set(slots, 0);
            patch::PINSPOT.speed.set(slots, 0);
        }),
    })
}
