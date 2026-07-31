//! The claw: the tensegrity sculpture's rig, driven by the sound in the room.
//!
//! Everything below the show is `cortex`. What lives here is this rig's patch, its scenes,
//! and the per-frame fill that turns audio features into slots — the sparkle engine on the
//! pinspot, with the three Yara pars pinned to hard R/G/B primaries for bring-up.

use cortex::Rig;
use cortex::audio_features::AudioFeatures;
use cortex::config as cfg;
use cortex::qlc_plus::{Rgb, White};
use cortex::sparkle::{SparkleMapping, SparkleOut};

// `patch` and `scenes`, generated from claw.qxw by the ingest. The workspace and the `.qxf`
// definitions are the source of truth; nothing about the addressing is hand-maintained, and
// there is no second copy of the patch to drift out of step with it. Generating them into
// this crate rather than the shared one is also what keeps the rigs apart: the mage's
// fixtures do not exist in this binary to be reached for.
include!(concat!(env!("OUT_DIR"), "/rig.rs"));

fn main() {
    // The sparkle engine: silence breathing under a colour drift, glinting on musical
    // onsets, with its own slow white-mode gate. One instance per fixture, so several
    // fixtures sparkle independently rather than in lock-step.
    let mut pinspot_map = SparkleMapping::new(
        cfg::SEEDS[0],
        cfg::SEEDS[1],
        cfg::SEEDS[2],
        cfg::WHITE_MODE_PERLIN_SEED,
    );

    cortex::run(Rig {
        name: env!("CARGO_PKG_NAME"),
        patch: &patch::PATCH,
        scenes: &scenes::SCENES,
        // Every fixture named here comes from the QLC+ patch, so this closure is also the
        // record of which patched fixtures the daemon actually drives.
        show: Box::new(move |features: &AudioFeatures, dt: f64, slots: &mut [u8]| {
            fill_sparkle(slots, &patch::PINSPOT, &pinspot_map.frame(features, dt));
            // The pinspot's other two channels are its own, not the engine's: Effect below
            // 64 is what hands the emitters to the colour channels at all, and Speed only
            // feeds the internal programs that Effect would start.
            patch::PINSPOT.effect.set(slots, 0);
            patch::PINSPOT.speed.set(slots, 0);

            fill_yara(slots, &patch::YARA_LEG_A, [255, 0, 0]); // red
            fill_yara(slots, &patch::YARA_LEG_B, [0, 255, 0]); // green
            fill_yara(slots, &patch::YARA_LEG_C, [0, 0, 255]); // blue
        }),
    })
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

/// Pin one Yara par to a hard primary for bring-up. Needs both colour and a white emitter,
/// so the white can be held off rather than left wherever it was.
fn fill_yara<F: Rgb + White>(slots: &mut [u8], fixture: &F, rgb: [u8; 3]) {
    fixture.red().set(slots, rgb[0]);
    fixture.green().set(slots, rgb[1]);
    fixture.blue().set(slots, rgb[2]);
    fixture.white().set(slots, 0);
}
