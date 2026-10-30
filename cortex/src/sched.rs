//! Where each of the brain's threads runs, and at what priority.
//!
//! The frame loop owns a core of its own at real-time priority, so the kernel can't preempt it
//! mid-frame and split the back-to-back slot stream: the classic Pi-UART → DMX corruption. The
//! audio thread is the only other one with a deadline, so it is real-time too, on one of the
//! background cores, where it preempts whatever else runs there. Everything else drops to
//! `SCHED_OTHER` on the background cores. A CPU-bound part, such as a detector, then slows
//! itself and never a frame or a sound period. The kernel's real-time throttle
//! (`sched_rt_runtime_us`, 95% by default) still guarantees ordinary tasks make progress, so a
//! busy-spin in the break code cannot lock the box.
//!
//! Set by each thread on itself, from inside the process, rather than in the unit file. A unit
//! file can only say one thing about the whole process, and here the frame loop and the audio
//! thread need opposite things from the threads beside them.
//!
//! The policy and the mask are inherited by any thread a thread spawns after setting them. That
//! is what puts embassy-time's timer thread on the frame loop's core, and an inference
//! runtime's worker pool on the background cores, without either of them asking.
//!
//! Neither call can fail for a reason that goes away by itself. The brain runs as root on a
//! four-core Pi, so a failure is a deploy error, and it panics rather than letting the show
//! run unpinned.

use crate::config as cfg;
#[cfg(target_os = "linux")]
use std::{io, mem};

/// Puts the calling thread on `cores` under `SCHED_FIFO`, at the brain's real-time priority.
pub fn realtime(cores: &[usize]) {
    pin(cores, libc::SCHED_FIFO, cfg::REALTIME_PRIORITY);
}

/// Puts the calling thread on the background cores under `SCHED_OTHER`.
pub fn background() {
    pin(cfg::BACKGROUND_CORES, libc::SCHED_OTHER, 0);
}

/// The brain only runs on the Pi; elsewhere it builds, for checking, and stops at the first pin.
#[cfg(not(target_os = "linux"))]
fn pin(cores: &[usize], _policy: libc::c_int, _priority: libc::c_int) {
    panic!("sched: pinning to cores {cores:?} needs Linux");
}

#[cfg(target_os = "linux")]
fn pin(cores: &[usize], policy: libc::c_int, priority: libc::c_int) {
    let thread = std::thread::current();
    let name = thread.name().unwrap_or("unnamed");

    // SAFETY: `cpu_set_t` is a plain bitmask, for which all zeroes is the empty set.
    let mut set: libc::cpu_set_t = unsafe { mem::zeroed() };
    for &core in cores {
        // SAFETY: `core` indexes a bit inside `set`, which `CPU_SET` bounds-checks itself.
        unsafe { libc::CPU_SET(core, &mut set) };
    }
    // SAFETY: `pthread_self` is always valid for the calling thread, and `set` outlives the
    // call.
    let failed =
        unsafe { libc::pthread_setaffinity_np(libc::pthread_self(), mem::size_of_val(&set), &set) };
    if failed != 0 {
        panic!(
            "sched: {name}: cores {cores:?}: {}",
            io::Error::from_raw_os_error(failed)
        );
    }

    let param = libc::sched_param {
        sched_priority: priority,
    };
    // SAFETY: as above, and `param` outlives the call.
    let failed = unsafe { libc::pthread_setschedparam(libc::pthread_self(), policy, &param) };
    if failed != 0 {
        panic!(
            "sched: {name}: policy {policy} priority {priority}: {}",
            io::Error::from_raw_os_error(failed)
        );
    }

    let policy = if policy == libc::SCHED_FIFO {
        format!("fifo {priority}")
    } else {
        "other".to_string()
    };
    eprintln!("sched: {name} on cores {cores:?}, {policy}");
}
