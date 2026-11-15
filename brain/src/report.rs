//! The brain's OSC and the face's, as `fixtures/brain.oscquery.json` and
//! `fixtures/face.oscquery.json` have them, sent to every `osc` edge from the brain in
//! `clockwork.dot`. What goes on connect goes once, at start, which is the brain's connect:
//! its WiFi is the access point. The rest goes at its rate, a bundle per rate.
//!
//! The face is a part of the brain today, on the same Pi, in the same program; each reports the
//! shared blocks for itself all the same, so either can move.

use crate::face::FaceState;
use crate::osc::{brain, face};
use crate::{network, patch};
use cortex::Decision;
use cortex::calendar::{self, Calendar};
use cortex::config::{FRAME_RATE_HZ, SACN_PRIORITY};
use cortex::latest::LatestRx;
use cortex::osc::Value;
use cortex::report::{self, Message, Sender, When, messages};
use std::net::Ipv4Addr;
use std::thread;
use std::time::Duration;

/// The devices this program is, each reporting the shared blocks.
const DEVICES: [&str; 2] = ["brain", "face"];

/// The fast bundle's rate and the slow one's, as the files give them, and so how many fast
/// ticks there are to one slow one.
const FAST_HZ: f64 = brain::BrainShow::RATE_HZ;
const SLOW_HZ: f64 = brain::BrainNextShowS::RATE_HZ;
const FAST_PER_SLOW: u64 = (FAST_HZ / SLOW_HZ) as u64;

pub fn spawn(
    face_states: LatestRx<FaceState>,
    show_states: LatestRx<brain::BrainShow>,
    decisions: LatestRx<Decision>,
    calendar: &'static Calendar,
) {
    thread::Builder::new()
        .name("report".into())
        .spawn(move || {
            let sender = Sender::new(network::OSC_RECIPIENTS);
            sender.send(&connect());
            report::every(Duration::from_secs_f64(1.0 / FAST_HZ), |ticks| {
                let face = face_states.snapshot();
                sender.send(&fast(&face, show_states.snapshot(), &decisions.snapshot()));
                if ticks % FAST_PER_SLOW == 0 {
                    sender.send(&slow(calendar));
                }
            })
        })
        .expect("failed to spawn the report thread");
}

fn connect() -> Vec<Message> {
    let mut out = report::shared(&DEVICES, When::Connect);
    // The face has no address field: its first channel is its start address. The brain sends
    // the whole universe, so its own is the universe's first slot.
    let universe = patch::SACN.universe;
    out.extend(report::dmx("brain", universe, 1));
    out.extend(report::dmx(
        "face",
        universe,
        (patch::FACE.all()[0].slot() + 1) as u16,
    ));
    out.extend(messages([
        (
            brain::DmxBrainFrameRateHz::PATH,
            Value::Int(FRAME_RATE_HZ as i32),
        ),
        (
            brain::DmxBrainPriority::PATH,
            Value::Int(i32::from(SACN_PRIORITY)),
        ),
    ]));
    out
}

fn fast(face: &FaceState, show: brain::BrainShow, decided: &Decision) -> Vec<Message> {
    let (ip, priority) = decided.takeover.unwrap_or((Ipv4Addr::UNSPECIFIED, 0));
    messages([
        (brain::BrainShow::PATH, Value::Text(show.word().to_owned())),
        (brain::BrainRig::PATH, Value::Text(decided.rig.to_owned())),
        (brain::DmxBrainTakeoverIp::PATH, Value::Text(ip.to_string())),
        (
            brain::DmxBrainTakeoverPriority::PATH,
            Value::Int(i32::from(priority)),
        ),
        (face::FaceLeftButton::PATH, Value::Bool(face.left_down)),
        (face::FaceRightButton::PATH, Value::Bool(face.right_down)),
        (
            face::FaceEstop::PATH,
            Value::Text(face.estop.word().to_owned()),
        ),
    ])
}

fn slow(calendar: &Calendar) -> Vec<Message> {
    let now = calendar::now();
    let next_show = if calendar.plays(now) {
        0
    } else {
        calendar
            .next_play(now)
            .map_or(-1, |start| (start - now).ceil() as i32)
    };
    let dusk = calendar
        .dusk(now)
        .map_or(0, |dusk| (dusk - now).max(0.0).ceil() as i32);
    let mut out = report::shared(&DEVICES, When::Second);
    out.extend(messages([
        (brain::BrainNextShowS::PATH, Value::Int(next_show)),
        (brain::BrainDuskS::PATH, Value::Int(dusk)),
    ]));
    out
}
