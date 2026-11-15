//! Everything under the show, and the loop that runs it.
//!
//! The show and the patch generated from its QLC+ workspace live in the rig's own crate;
//! what does not depend on which fixtures are patched lives here.
//!
//! A rig hands over a [`Rig`] and gets the frame loop, both transports, the takeover
//! arbitration and the e-stop for it.

pub mod calendar;
pub mod clock;
pub mod config;
pub mod dmx;
pub mod estop;
pub mod latest;
pub mod osc;
pub mod qlc_plus;
pub mod report;
pub mod sched;
pub mod system;

mod orchestrator;
mod sacn_in;
mod sacn_out;

use config::FRAME_RATE_HZ;
use embassy_executor::{Executor, Spawner};
use qlc_plus::{PatchEntry, Sacn};
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;
use signal_hook::low_level::signal_name;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// One rig: what its QLC+ workspace turned into, plus the show that drives it.
///
/// The workspace itself is not here and never crosses into the running program. It is a
/// build artifact: the ingest consumes it and the patch below is everything that survives.
///
/// `show` is called once per frame with the frame period in seconds and the full 512-slot
/// buffer to fill. Whatever the show senses it holds itself, as the readers of the seams its
/// sensors publish on. It is a closure rather than a trait
/// implementation because the boundary carries no meaning beyond "the rig fills the slot
/// buffer".
pub struct Rig {
    /// The binary's own name, which prefixes every log line. The rig passes
    /// `env!("CARGO_PKG_NAME")`, so it cannot drift from the binary that is installed.
    pub name: &'static str,
    /// The universe and port the workspace puts the rig on.
    pub sacn: Sacn,
    /// Every fixture the brain sends its universe to by unicast, generated from the rig's
    /// `clockwork.dot`. The wired fixtures are not here: they get the same frame from the HAT.
    pub sacn_recipients: &'static [SocketAddrV4],
    /// The generated patch, as plain data, so the startup log can name what this binary was
    /// built against — and so the frame width can be derived from it.
    pub patch: &'static [PatchEntry],
    /// The generated stop mask: what the e-stop overrides in the held frame.
    pub stop: &'static [estop::StopOverride],
    /// Whether the e-stop is pressed, asked once per frame. The stop acts on the frame that
    /// goes out, after the takeover, so a console cannot drive the rig past it.
    pub estop: Box<dyn FnMut() -> bool>,
    pub show: Box<dyn FnMut(f64, &mut [u8])>,
    /// The rig's own tasks, beside the frame loop. Called once, with the executor's spawner,
    /// after the frame task is spawned. A closure so the rig can move hardware it claimed
    /// before the brain came up into the tasks that drive it.
    pub tasks: Box<dyn FnOnce(Spawner)>,
    /// Where every frame goes once it is final — after the takeover has had its say — for
    /// the rig's fixtures that live inside the brain process and read their own channels
    /// out of it rather than off the wire.
    pub frames: latest::LatestTx<dmx::Frame>,
    /// Where the frame loop says what it has decided, on every change, for the rig to report.
    pub decisions: latest::LatestTx<Decision>,
}

/// What the frame loop decided: the rig state, as the rig reports it on `/brain/rig`, and the
/// console that has taken the universe over, by its address and priority, if one has. Before
/// the first frame, [`Decision::BOOT`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub rig: &'static str,
    pub takeover: Option<(Ipv4Addr, u8)>,
}

/// Slots the sACN frame spans: 1 through the last slot any patched fixture occupies.
///
/// Derived rather than carried beside the patch. The two would have to agree and nothing
/// could check that they did — a wrong constant is a panic on the first frame if it is too
/// large, and a permanently short network frame, invisible on the padded wire, if it is too
/// small.
pub(crate) fn frame_width(patch: &[PatchEntry]) -> usize {
    patch
        .iter()
        .flat_map(|fixture| fixture.channels)
        .map(|channel| channel.slot())
        .max()
        .map_or(0, |slot| slot + 1)
}

/// Brings the cabinet up, then runs `rig`'s frame loop forever.
///
/// The executor lives here rather than in the rig, so a rig binary is its patch and its show,
/// and nothing else — `fn main` is one call.
pub fn run(mut rig: Rig) -> ! {
    // This thread becomes the frame loop. The clock is read straight after pinning it, because
    // the first read starts embassy-time's timer thread, which wakes the frame loop: started
    // here, it inherits the frame loop's core and priority. Every task that reads the clock
    // runs on the executor below, so none can start it earlier.
    sched::realtime();
    embassy_time::Instant::now();

    let socket = UdpSocket::bind("0.0.0.0:0").expect("socket bind failed");
    let cid = sacn_out::new_cid();

    // The external-takeover receiver: an OS thread of its own, because a blocking recv_from
    // would stall the executor's single task. It publishes candidates;
    // the frame loop decides once per frame whether one is actually driving.
    let (takeover_tx, takeover_rx) = latest::latest(sacn_in::Takeover::idle());
    let sacn = rig.sacn;
    sacn_in::spawn_receiver(cid, sacn, takeover_tx);

    let name = rig.name;
    assert!(
        rig.sacn_recipients.len() <= sacn_out::MAX_RECIPIENTS,
        "{name}: {} sACN recipients, and the brain sends to at most {}",
        rig.sacn_recipients.len(),
        sacn_out::MAX_RECIPIENTS
    );
    eprintln!(
        "{name}: universe {} @ {FRAME_RATE_HZ} Hz → the wire and {} by unicast",
        sacn.universe,
        rig.sacn_recipients.len()
    );
    for recipient in rig.sacn_recipients {
        eprintln!("{name}:   {recipient}");
    }

    // The patch the build compiled in. Logged so a running binary can be checked against the
    // workspace that produced it: a fixture at the wrong address means the build never saw
    // the save.
    eprintln!("{name}: {} fixtures", rig.patch.len());
    for fixture in rig.patch {
        // The slot span, not the channel count: the mode name already carries the count,
        // and the last slot is what you check a patch against to see nothing collides.
        eprintln!(
            "{name}:   {} @ {}–{} — {}",
            fixture.name,
            fixture.address,
            fixture.address as usize + fixture.channels.len() - 1,
            fixture.profile,
        );
    }
    eprintln!("{name}: {} e-stop overrides", rig.stop.len());

    // systemd stops the brain with SIGTERM, and a brain started by hand is stopped with
    // SIGINT. Catching both only asks the frame loop to shut down: it releases the sACN source
    // on its own send path, so there is never a second sender.
    // signal_hook's iterator runs on its own thread, outside async-signal context, so the log
    // line here is safe.
    let shutdown = Arc::new(AtomicBool::new(false));
    let asked = shutdown.clone();
    let mut signals = Signals::new([SIGINT, SIGTERM]).expect("signal registration failed");
    std::thread::Builder::new()
        .name("signals".into())
        .spawn(move || {
            sched::background();
            if let Some(signal) = signals.forever().next() {
                let signal = signal_name(signal).expect("a signal registered above");
                eprintln!("{name}: {signal} — shutting down");
                asked.store(true, Ordering::Relaxed);
            }
        })
        .expect("failed to spawn the signal thread");

    // `Executor::run` needs `&'static mut`, and never returns, so leaking one allocation buys
    // that lifetime honestly — this is what the `#[embassy_executor::main]` macro does with a
    // transmute instead.
    let executor: &'static mut Executor = Box::leak(Box::new(Executor::new()));
    // The frame task takes the rig whole, so the one-shot hook comes out of it first.
    let tasks = std::mem::replace(&mut rig.tasks, Box::new(|_| {}));
    executor.run(|spawner| {
        spawner.spawn(orchestrator::frame_task(socket, cid, takeover_rx, shutdown, rig).unwrap());
        tasks(spawner);
    })
}
