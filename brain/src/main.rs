//! The claw: the tensegrity sculpture's rig.
//!
//! Everything below the show is `cortex`. What lives here is this rig's patch, its calendar,
//! and the per-frame fill that turns the show into slots.

use cortex::Rig;
use cortex::latest::latest;
use show::Show;

mod face;
mod face_show;
mod report;
mod show;
mod wrist;
mod wrist_show;

// `patch`, generated from clockwork.qxw by the ingest, and `network`, from clockwork.dot. The
// workspace and the `.qxf` definitions are the source of truth for the patch, the dot for the
// addresses; nothing about either is hand-maintained, and there is no second copy to drift
// out of step.
include!(concat!(env!("OUT_DIR"), "/rig.rs"));

// `shows`, generated from `shows.rs`: every show as instants, with its dark stretches.
include!(concat!(env!("OUT_DIR"), "/shows.rs"));

// `osc::wrist` and `osc::dendril`, generated from `fixtures/wrist.oscquery.json` and
// `fixtures/dendril.oscquery.json`: the addresses Wrist's board sends and their values, which
// the brain decodes by; `osc::brain` and `osc::face`, the addresses the brain sends.
mod osc {
    include!(concat!(env!("OUT_DIR"), "/osc_wrist.rs"));
    include!(concat!(env!("OUT_DIR"), "/osc_dendril.rs"));
    include!(concat!(env!("OUT_DIR"), "/osc_brain.rs"));
    include!(concat!(env!("OUT_DIR"), "/osc_face.rs"));
}

fn main() {
    // The face first, so the lamps are dark and the inputs pulled up while everything else
    // starts. It reads its lamps out of the frames the brain sends, and the show and the
    // brain's OSC read its state.
    let (frames, sent_frames) = latest(cortex::dmx::BLACKOUT);
    let (face, face_state) = face::claim(sent_frames);
    let estop_state = face_state.clone();
    // Wrist's OSC, which the show plucks it by and shows its phase and the sway's from.
    let wrist_reports = wrist::spawn(network::OSC_BIND);
    let (mut show, show_states) = Show::new(&shows::CALENDAR);
    // What the frame loop decides, which the brain's OSC reports with the show and the face.
    let (decisions, decided) = latest(cortex::Decision::BOOT);
    report::spawn(face_state.clone(), show_states, decided, &shows::CALENDAR);

    cortex::run(Rig {
        name: env!("CARGO_PKG_NAME"),
        sacn: patch::SACN,
        sacn_recipients: network::SACN_RECIPIENTS,
        patch: &patch::PATCH,
        stop: patch::STOP,
        estop: Box::new(move || estop_state.snapshot().stopped()),
        // Every fixture named here comes from the QLC+ patch, so this closure is also the
        // record of which patched fixtures the daemon actually drives.
        show: Box::new(move |_dt: f64, slots: &mut [u8]| {
            show.fill(slots, &face_state.snapshot(), wrist_reports.snapshot());
        }),
        tasks: Box::new(move |spawner| {
            face::spawn(spawner, face);
            cortex::calendar::spawn(spawner, &shows::CALENDAR);
        }),
        frames,
        decisions,
    })
}
