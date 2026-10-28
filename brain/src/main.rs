//! The claw: the tensegrity sculpture's rig.
//!
//! Everything below the show is `cortex`. What lives here is this rig's patch, its pointing
//! presets, its calendar, its recordings, and the per-frame fill that turns the show into
//! slots.

use cortex::Rig;
use cortex::latest::latest;
use cortex::telemetry;
use mouth_show::MouthShow;
use show::Show;

mod door_show;
mod face;
mod face_show;
mod heads_show;
mod heart;
mod mouth_show;
mod presence;
mod show;

// `patch` and `presets`, generated from claw.qxw by the ingest. The workspace and the `.qxf`
// definitions are the source of truth; nothing about the addressing is hand-maintained, and
// there is no second copy of the patch to drift out of step with it.
include!(concat!(env!("OUT_DIR"), "/rig.rs"));

// `shows`, generated from `shows.rs`: every show as instants, with its dark stretches.
include!(concat!(env!("OUT_DIR"), "/shows.rs"));

// `sounds`, generated from `sounds.rs` and the recordings in `sounds/`.
include!(concat!(env!("OUT_DIR"), "/sounds.rs"));

fn main() {
    // The face first, so the lamps are dark and the inputs pulled up while everything else
    // starts. It publishes its own `face/…` subtree on the brain's connection, reads its lamps
    // out of the frames the brain sends, and the show reads its state.
    let (_brain_telemetry, brain_publisher, brain_farewell) =
        telemetry::spawn(cortex::TELEMETRY_SERVICE);
    let (frames, sent_frames) = latest(cortex::dmx::BLACKOUT);
    let (face, face_state) = face::claim(brain_publisher.part("face"), sent_frames);
    let estop_state = face_state.clone();
    let mouth = mouth::spawn(&sounds::SOUNDS, brain_publisher.part("mouth"));
    let mut show = Show::new(
        brain_publisher.clone(),
        &shows::CALENDAR,
        MouthShow::new(mouth),
    );

    cortex::run(Rig {
        name: env!("CARGO_PKG_NAME"),
        sacn: patch::SACN,
        patch: &patch::PATCH,
        presets: &presets::SUMMARIES,
        stop: patch::STOP,
        estop: Box::new(move || estop_state.snapshot().estop),
        // Every fixture named here comes from the QLC+ patch, so this closure is also the
        // record of which patched fixtures the daemon actually drives.
        show: Box::new(move |_dt: f64, slots: &mut [u8]| {
            show.fill(slots, &face_state.snapshot());
        }),
        tasks: Box::new(move |spawner| {
            face::spawn(spawner, face);
            cortex::calendar::spawn(spawner, &shows::CALENDAR);
        }),
        frames,
        telemetry: brain_publisher,
        farewells: vec![brain_farewell],
    })
}
