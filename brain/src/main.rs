//! The claw: the tensegrity sculpture's rig, driven by the sound in the room.
//!
//! Everything below the show is `cortex`. What lives here is this rig's patch, its QLC+
//! scenes, its calendar, and the per-frame fill that turns audio features into slots. The
//! show picks a scene each frame, from the calendar or from test mode. In blackout the rig is
//! black; in the others the three washes each hold their own colour, red, green or blue,
//! with white beside it, and breathe and flash to the sound in step; and the ball does what
//! the scene asks of it.

use cortex::Rig;
use cortex::ear::AudioFeatures;
use cortex::config as cfg;
use cortex::latest::latest;
use cortex::qlc_plus::{Channel, Rgb, White};
use cortex::sparkle::SparkleMapping;
use cortex::telemetry;
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

/// Each wash's own colour, and the white beside it, at the top of the modulation.
const WASH_COLOUR_LEVEL: f64 = 0.8;
const WASH_WHITE_LEVEL: f64 = 0.5;

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

    // The sparkle engine: silence breathing, a flash on each musical onset, and nothing
    // between onsets once the music is in. One instance for all three legs, so the rig pulses
    // in step.
    let mut modulation = SparkleMapping::new(
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

            // The modulation and the status LED's sine run in every scene, so either resumes
            // mid-stride rather than out of a frozen frame. The sine is kept within one turn so
            // the phase never grows past the precision of an f64.
            //
            // Only the modulation's intensity is used: each leg keeps its own colour. It is
            // gamma-corrected, because the breathing is shaped in perceived brightness.
            let level = modulation.frame(features, dt).intensity.powf(1.0 / cfg::GAMMA);
            ball_led_phase = (ball_led_phase + TAU * BALL_LED_HZ * dt) % TAU;

            if cue.scene == Scene::Estop {
                slots.fill(0);
                return;
            }
            if cue.scene != Scene::Blackout {
                fill_wash(slots, &patch::WASH_RED_LEG, Rgb::red, level);
                fill_wash(slots, &patch::WASH_GREEN_LEG, Rgb::green, level);
                fill_wash(slots, &patch::WASH_BLUE_LEG, Rgb::blue, level);
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

/// One wash at its own colour and its white, both scaled by `level`, with its other two
/// colours held off rather than left wherever they were.
fn fill_wash<F: Rgb + White>(slots: &mut [u8], fixture: &F, colour: fn(&F) -> Channel, level: f64) {
    for channel in [fixture.red(), fixture.green(), fixture.blue()] {
        channel.set(slots, 0);
    }
    colour(fixture).set_unit(slots, WASH_COLOUR_LEVEL * level);
    fixture.white().set_unit(slots, WASH_WHITE_LEVEL * level);
}
