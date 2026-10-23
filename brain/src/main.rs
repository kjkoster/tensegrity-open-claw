//! The claw: the tensegrity sculpture's rig, driven by the sound in the room.
//!
//! Everything below the show is `cortex`. What lives here is this rig's patch, its pointing
//! presets, its calendar, and the per-frame fill that turns audio features into slots. The
//! show picks a show state each frame, from the calendar, the incident schedule or test mode.
//! In the base show the three washes each hold their own colour, red, green or blue, with white
//! beside it, and breathe and flash to the sound in step; the heads rest dark on the ball; and
//! the ball does what the show state asks of it. Every half hour or so after dusk an incident
//! takes over for a few minutes.

use cortex::Rig;
use cortex::ear::AudioFeatures;
use cortex::config as cfg;
use cortex::latest::latest;
use cortex::sparkle::SparkleMapping;
use cortex::telemetry;
use ball_show::BallShow;
use head_show::HeadShow;
use show_state::Director;

mod ball_lighting;
mod ball_show;
mod face;
mod face_show;
mod head_show;
mod legs;
mod schedule;
mod show_state;
mod test_mode;
mod wash_show;

// `patch` and `presets`, generated from claw.qxw by the ingest. The workspace and the `.qxf`
// definitions are the source of truth; nothing about the addressing is hand-maintained, and
// there is no second copy of the patch to drift out of step with it.
include!(concat!(env!("OUT_DIR"), "/rig.rs"));

// `shows`, generated from `shows.rs`: every show as instants, with its dark stretches.
include!(concat!(env!("OUT_DIR"), "/shows.rs"));

fn main() {
    // First, so the lamps are dark and the inputs pulled up while everything else starts. Its
    // client comes up beside it, as a fixture of its own rather than a topic of the brain's.
    // It reads its lamps out of the frames the brain sends, and the show reads its state.
    let (_face_telemetry, face_publisher, face_farewell) = telemetry::spawn("face");
    let (frames, sent_frames) = latest(cortex::dmx::BLACKOUT);
    let (face, face_state) = face::claim(face_publisher, sent_frames);
    let estop_state = face_state.clone();
    let (_brain_telemetry, brain_publisher, brain_farewell) =
        telemetry::spawn(cortex::TELEMETRY_SERVICE);
    let mut director = Director::new(&shows::CALENDAR, brain_publisher.clone());

    // The sparkle engine: silence breathing, a flash on each musical onset, and nothing
    // between onsets once the music is in. One instance for all three legs, so the rig pulses
    // in step.
    let mut modulation = SparkleMapping::new();
    let mut heads = HeadShow::new();
    let mut ball_show = BallShow::new();

    cortex::run(Rig {
        name: env!("CARGO_PKG_NAME"),
        sacn: patch::SACN,
        patch: &patch::PATCH,
        presets: &presets::SUMMARIES,
        stop: patch::STOP,
        estop: Box::new(move || estop_state.snapshot().estop),
        // Every fixture named here comes from the QLC+ patch, so this closure is also the
        // record of which patched fixtures the daemon actually drives.
        show: Box::new(move |features: &AudioFeatures, dt: f64, slots: &mut [u8]| {
            let cue = director.cue(cortex::calendar::now(), &face_state.snapshot());

            // The modulation runs in every show state, so it resumes mid-stride rather than out
            // of a frozen frame.
            //
            // Gamma-corrected, because the breathing is shaped in perceived brightness.
            let sparkle = modulation.frame(features, dt).powf(1.0 / cfg::GAMMA);

            wash_show::fill(slots, cue, sparkle);
            heads.fill(slots, cue, sparkle, features, dt);
            ball_show.fill(slots, cue, dt);
            face_show::fill(slots, cue);
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
