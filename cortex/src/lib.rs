//! Everything under the show, and the loop that runs it.
//!
//! The show and the patch generated from its QLC+ workspace live in the rig's own crate;
//! what does not depend on which fixtures are patched lives here.
//!
//! A rig hands over a [`Rig`] and gets the frame loop, the ear, both transports, the
//! takeover arbitration and the e-stop for it.

pub mod calendar;
pub mod clock;
pub mod config;
pub mod dmx;
pub mod draw;
pub mod ear;
pub mod envelope;
pub mod estop;
pub mod latest;
pub mod moving_head;
pub mod qlc_plus;
pub mod sparkle;
pub mod telemetry;

mod orchestrator;
mod sacn_in;
mod sacn_out;

use ear::AudioFeatures;
use config::FRAME_RATE_HZ;
use embassy_executor::{Executor, Spawner};
use qlc_plus::{PatchEntry, Sacn};
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;
use signal_hook::low_level::signal_name;
use std::net::UdpSocket;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// The name the brain wears on the broker: its health topic and the prefix on everything else
/// it publishes. The rig spawns the client under it, so its show can publish there as well.
pub const TELEMETRY_SERVICE: &str = "brain";

/// One rig: what its QLC+ workspace turned into, plus the show that drives it.
///
/// The workspace itself is not here and never crosses into the running program. It is a
/// build artifact: the ingest consumes it and the patch below is everything that survives.
///
/// `show` is called once per frame with the latest audio features, the frame period in
/// seconds, and the full 512-slot buffer to fill. It is a closure rather than a trait
/// implementation because the boundary carries no meaning beyond "the rig fills the slot
/// buffer" — when the intent frame lands it replaces the closure and the rig's content does
/// not move.
pub struct Rig {
    /// The binary's own name, which prefixes every log line. The rig passes
    /// `env!("CARGO_PKG_NAME")`, so it cannot drift from the binary that is installed.
    pub name: &'static str,
    /// The universe and port the workspace puts the rig on.
    pub sacn: Sacn,
    /// The generated patch, as plain data, so the startup log can name what this binary was
    /// built against — and so the frame width can be derived from it.
    pub patch: &'static [PatchEntry],
    /// The generated presets, likewise, one line each: its name and every head's pose. Lines
    /// rather than the presets themselves, because a preset's shape is the rig's own — its
    /// fields are the rig's heads.
    pub presets: &'static [&'static str],
    /// The generated stop mask: what the e-stop overrides in the held frame.
    pub stop: &'static [estop::StopOverride],
    /// Whether the e-stop is pressed, asked once per frame. The stop acts on the frame that
    /// goes out, after the takeover, so a console cannot drive the rig past it.
    pub estop: Box<dyn FnMut() -> bool>,
    pub show: Box<dyn FnMut(&AudioFeatures, f64, &mut [u8])>,
    /// The rig's own tasks, beside the frame loop. Called once, with the executor's spawner,
    /// after the frame task is spawned. A closure so the rig can move hardware it claimed
    /// before the brain came up into the tasks that drive it.
    pub tasks: Box<dyn FnOnce(Spawner)>,
    /// Where every frame goes once it is final — after the takeover has had its say — for
    /// the rig's fixtures that live inside the brain process and read their own channels
    /// out of it rather than off the wire.
    pub frames: latest::LatestTx<dmx::Frame>,
    /// The brain's own client, spawned by the rig under [`TELEMETRY_SERVICE`] so that both the
    /// rig's show and everything here can publish under `brain/`: the show is built before
    /// the brain comes up, and one service cannot have two clients.
    pub telemetry: telemetry::Publisher,
    /// The goodbyes of every telemetry client the rig spawned, the brain's included, said by
    /// the frame loop on shutdown. The rig spawns those clients itself, but only the brain
    /// knows when it is stopping.
    pub farewells: Vec<telemetry::Farewell>,
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

/// What the brain runs on, in the `sys` subtree every part of the rig publishes its own under:
/// the board as its device tree names it, the cores, the top CPU frequency in MHz, the
/// installed RAM in KB, and the target triplet the brain was compiled for. A reading the
/// board does not offer is left out rather than published as a guess.
fn publish_system(publisher: &telemetry::Publisher) {
    if let Ok(model) = std::fs::read_to_string("/proc/device-tree/model") {
        publisher.publish("sys/board", model.trim_end_matches('\0').trim(), true);
    }
    if let Some(kb) = installed_ram_kb() {
        publisher.publish("sys/ram_kb", kb.to_string(), true);
    }
    if let Ok(cores) = std::thread::available_parallelism() {
        publisher.publish("sys/cores", cores.to_string(), true);
    }
    let max_khz = "/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq";
    if let Some(khz) = std::fs::read_to_string(max_khz)
        .ok()
        .and_then(|text| text.trim().parse::<u64>().ok())
    {
        publisher.publish("sys/cpu_frequency_mhz", (khz / 1000).to_string(), true);
    }
    publisher.publish("sys/triplet", env!("TRIPLET"), true);
}

/// The RAM fitted to a Raspberry Pi, decoded from the revision code in `/proc/cpuinfo`.
///
/// Not the kernel's `MemTotal`, which is what is left once the GPU and the kernel have taken
/// their share. A new-style code (bit 23) carries the size in bits 20–22, as 256 MB doubled
/// that many times; an old-style code does not say, and is left out.
fn installed_ram_kb() -> Option<u64> {
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    let revision = cpuinfo
        .lines()
        .find_map(|line| line.strip_prefix("Revision")?.split(':').nth(1))?;
    let code = u32::from_str_radix(revision.trim(), 16).ok()?;
    let new_style = code & (1 << 23) != 0;
    new_style.then(|| (256 * 1024) << ((code >> 20) & 0x7))
}

/// Brings the cabinet up, then runs `rig`'s frame loop forever.
///
/// The executor lives here rather than in the rig, so a rig binary is its patch, its presets
/// and its show, and nothing else — `fn main` is one call. The ear always listens, on every
/// rig: a rig with no use for it simply never reads the features it is handed, and an ear
/// with nothing listening to it is cheaper than two configurations that can disagree about
/// which interface the cabinet has.
pub fn run(mut rig: Rig) -> ! {
    // The ear runs on its own OS thread, alongside Embassy, and reports on its own client:
    // a part of the rig on the broker like the face, not a topic of the brain's.
    let (_ear_telemetry, ear_publisher, ear_farewell) = telemetry::spawn("ear");
    let (_ear, rx) = ear::spawn(ear_publisher);

    let socket = UdpSocket::bind("0.0.0.0:0").expect("socket bind failed");
    socket
        .set_multicast_ttl_v4(1)
        .expect("set multicast TTL failed");
    let cid = sacn_out::new_cid();

    // The external-takeover receiver: a second OS thread alongside the ear, because a
    // blocking recv_from would stall the executor's single task. It publishes candidates;
    // the frame loop decides once per frame whether one is actually driving.
    let (takeover_tx, takeover_rx) = latest::latest(sacn_in::Takeover::idle());
    let sacn = rig.sacn;
    let _sacn_in = sacn_in::spawn_receiver(cid, sacn, takeover_tx);

    let group = sacn_out::multicast_addr(sacn.universe);
    let name = rig.name;
    eprintln!(
        "{name}: universe {} → {group}:{}  @ {FRAME_RATE_HZ} Hz",
        sacn.universe, sacn.port
    );

    // The patch and presets the build compiled in. Logged so a running binary can be checked
    // against the workspace that produced it: a fixture at the wrong address, or a preset still
    // at its old pose, means the build never saw the save.
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
    eprintln!("{name}: {} presets", rig.presets.len());
    for preset in rig.presets {
        eprintln!("{name}:   {preset}");
    }

    // What the brain was built as and against goes under `brain/`, retained, so a subscriber
    // arriving later still gets it.
    rig.farewells.push(ear_farewell);
    let publisher = &rig.telemetry;
    publisher.publish("dmx/universe", sacn.universe.to_string(), true);
    publisher.publish("frame_rate_hz", FRAME_RATE_HZ.to_string(), true);
    publish_system(publisher);

    // systemd stops the brain with SIGTERM, and a brain started by hand is stopped with
    // SIGINT. Catching both only asks the frame loop to shut down: it releases the sACN source
    // on its own send path, so there is never a second sender, and says the goodbyes.
    // signal_hook's iterator runs on its own thread, outside async-signal context, so the log
    // line here is safe.
    let shutdown = Arc::new(AtomicBool::new(false));
    let asked = shutdown.clone();
    let mut signals = Signals::new([SIGINT, SIGTERM]).expect("signal registration failed");
    std::thread::spawn(move || {
        if let Some(signal) = signals.forever().next() {
            let signal = signal_name(signal).unwrap_or("signal");
            eprintln!("{name}: {signal} — shutting down");
            asked.store(true, Ordering::Relaxed);
        }
    });

    // `Executor::run` needs `&'static mut`, and never returns, so leaking one allocation buys
    // that lifetime honestly — this is what the `#[embassy_executor::main]` macro does with a
    // transmute instead.
    let executor: &'static mut Executor = Box::leak(Box::new(Executor::new()));
    // The frame task takes the rig whole, so the one-shot hook comes out of it first.
    let tasks = std::mem::replace(&mut rig.tasks, Box::new(|_| {}));
    executor.run(|spawner| {
        spawner.spawn(orchestrator::frame_task(socket, cid, rx, takeover_rx, shutdown, rig).unwrap());
        tasks(spawner);
    })
}
