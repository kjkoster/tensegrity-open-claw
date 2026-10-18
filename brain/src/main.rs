//! The claw: the tensegrity sculpture's rig, driven by the sound in the room.
//!
//! Everything below the show is `cortex`. What lives here is this rig's patch, its QLC+
//! scenes, its calendar, and the per-frame fill that turns audio features into slots. The
//! show picks a scene each frame, from the calendar or from test mode. In blackout the rig is
//! black; in the others the sparkle engine is on the pinspot, the three Yara pars are pinned to
//! hard R/G/B primaries for bring-up, and the ball does what the scene asks of it.

use cortex::Rig;
use cortex::audio_features::AudioFeatures;
use cortex::config as cfg;
use cortex::latest::latest;
use cortex::qlc_plus::{Rgb, White};
use cortex::sparkle::{SparkleMapping, SparkleOut};
use cortex::telemetry;
use patch::Vdplps36b2LedPinspotPar36;
use scene::{Director, Scene};
use std::f64::consts::TAU;

mod ball_show;
mod face;
mod scene;
mod test_mode;

// `patch` and `scenes`, generated from claw.qxw by the ingest. The workspace and the `.qxf`
// definitions are the source of truth; nothing about the addressing is hand-maintained, and
// there is no second copy of the patch to drift out of step with it.
include!(concat!(env!("OUT_DIR"), "/rig.rs"));

// `shows`, generated from `shows.rs`: every show as instants, with its dark stretches.
include!(concat!(env!("OUT_DIR"), "/shows.rs"));

/// How fast the ball's onboard LED fades up and down.
const BALL_LED_HZ: f64 = 1.0;

fn main() {
    // First, so the lamps are dark and the inputs pulled up while everything else starts. Its
    // client comes up beside it, as a fixture of its own rather than a topic of the brain's.
    // It reads its lamps out of the frames the brain sends, and the show reads its state.
    let (_face_telemetry, face_publisher, face_farewell) = telemetry::spawn("face");
    let (frames, sent_frames) = latest(cortex::DARK);
    let (face, face_state) = face::claim(face_publisher, sent_frames);
    let (_brain_telemetry, brain_publisher, brain_farewell) =
        telemetry::spawn(cortex::TELEMETRY_SERVICE);
    let mut director = Director::new(&shows::CALENDAR, brain_publisher.clone());

    // The sparkle engine: silence breathing under a colour drift, glinting on musical
    // onsets, with its own slow white-mode gate. One instance per fixture, so several
    // fixtures sparkle independently rather than in lock-step.
    let mut pinspot_map = SparkleMapping::new(
        cfg::SEEDS[0],
        cfg::SEEDS[1],
        cfg::SEEDS[2],
        cfg::WHITE_MODE_PERLIN_SEED,
    );
    let mut ball_led_phase = 0.0_f64;

    cortex::run(Rig {
        name: env!("CARGO_PKG_NAME"),
        sacn: patch::SACN,
        patch: &patch::PATCH,
        scenes: &scenes::SCENES,
        // Every fixture named here comes from the QLC+ patch, so this closure is also the
        // record of which patched fixtures the daemon actually drives.
        show: Box::new(move |features: &AudioFeatures, dt: f64, slots: &mut [u8]| {
            let cue = director.cue(cortex::calendar::now(), features, &face_state.snapshot());

            // The engine and the status LED's sine run in every scene, so either resumes
            // mid-stride rather than out of a frozen frame. The sine is kept within one turn
            // so the phase never grows past the precision of an f64.
            let sparkle = pinspot_map.frame(features, dt);
            ball_led_phase = (ball_led_phase + TAU * BALL_LED_HZ * dt) % TAU;

            if cue.scene == Scene::Estop {
                slots.fill(0);
                return;
            }
            if cue.scene != Scene::Blackout {
                fill_sparkle(slots, &patch::PINSPOT, &sparkle);
                // The pinspot's other two channels are its own, not the engine's: the Effect
                // band for RGB control is what hands the emitters to the colour channels at
                // all, and Speed only feeds the internal programs that Effect would start.
                patch::PINSPOT.effect.set(
                    slots,
                    Vdplps36b2LedPinspotPar36::EFFECT_RGB_CONTROL_VIA_THE_RED_GREEN_AND_BLUE_CHANNELS
                        .center(),
                );
                patch::PINSPOT.speed.park(slots);

                fill_yara(slots, &patch::YARA_LEG_A, [255, 0, 0]); // red
                fill_yara(slots, &patch::YARA_LEG_B, [0, 255, 0]); // green
                fill_yara(slots, &patch::YARA_LEG_C, [0, 0, 255]); // blue
            } else {
                slots.fill(0);
            }

            ball_show::fill(slots, cue, features);

            // The door's lamps say test mode is on.
            let lamps = if cue.testing { 1.0 } else { 0.0 };
            patch::FACE.left_lamp.set_unit(slots, lamps);
            patch::FACE.right_lamp.set_unit(slots, lamps);

            // A sine on the ball's onboard LED, so that a ball following the rig is told
            // apart at a glance from one that is not — which is why it breathes in blackout
            // as well. `intensity` is that LED: its definition gives the channel the
            // IntensityDimmer preset, and the field is named after the role.
            patch::BALL
                .intensity
                .set_unit(slots, (1.0 + ball_led_phase.sin()) / 2.0);
        }),
        tasks: Box::new(move |spawner| {
            face::spawn(spawner, face);
            cortex::calendar::spawn(spawner, &shows::CALENDAR);
        }),
        frames,
        telemetry: brain_publisher,
        farewells: vec![face_farewell, brain_farewell],
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
