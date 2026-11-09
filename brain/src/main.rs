//! The claw: the tensegrity sculpture's rig.
//!
//! Everything below the show is `cortex`. What lives here is this rig's patch, its calendar,
//! and the per-frame fill that turns the show into slots.

use cortex::Rig;
use cortex::latest::latest;
use cortex::telemetry;
use show::Show;

mod face;
mod face_show;
mod show;

// `patch`, generated from clockwork.qxw by the ingest, and `network`, from clockwork.dot. The
// workspace and the `.qxf` definitions are the source of truth for the patch, the dot for the
// addresses; nothing about either is hand-maintained, and there is no second copy to drift
// out of step.
include!(concat!(env!("OUT_DIR"), "/rig.rs"));

// `shows`, generated from `shows.rs`: every show as instants, with its dark stretches.
include!(concat!(env!("OUT_DIR"), "/shows.rs"));

fn main() {
    // The face first, so the lamps are dark and the inputs pulled up while everything else
    // starts. It publishes its own `face/…` subtree on the brain's connection, reads its lamps
    // out of the frames the brain sends, and the show reads its state.
    let (brain_publisher, brain_farewell) = telemetry::spawn(cortex::TELEMETRY_SERVICE, network::MQTT);
    let (frames, sent_frames) = latest(cortex::dmx::BLACKOUT);
    let (face, face_state) = face::claim(brain_publisher.part("face"), sent_frames);
    let estop_state = face_state.clone();
    let mut show = Show::new(brain_publisher.clone(), &shows::CALENDAR);

    cortex::run(Rig {
        name: env!("CARGO_PKG_NAME"),
        sacn: patch::SACN,
        patch: &patch::PATCH,
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
        farewell: brain_farewell,
    })
}
