//! Where each of the brain's threads runs, and at what priority.
//!
//! The frame loop owns a core of its own at real-time priority, so the kernel can't preempt it
//! mid-frame and split the back-to-back slot stream: the classic Pi-UART → DMX corruption.
//! Everything else drops to `SCHED_OTHER` on the background cores, so a CPU-bound part slows
//! itself and never a frame. The kernel's real-time throttle (`sched_rt_runtime_us`, 95% by
//! default) still guarantees ordinary tasks make progress, so a busy-spin in the break code
//! cannot lock the box.
//!
//! Set by each thread on itself, from inside the process, rather than in the unit file. A unit
//! file can only say one thing about the whole process, and here the frame loop needs the
//! opposite of the threads beside it.
//!
//! The policy and the mask are inherited by any thread a thread spawns after setting them. That
//! is what puts embassy-time's timer thread on the frame loop's core without it asking.
//!
//! Neither call can fail for a reason that goes away by itself. The brain runs as root on a
//! four-core Pi, so a failure is a deploy error, and it panics rather than letting the show
//! run unpinned.

use crate::config as cfg;
#[cfg(target_os = "linux")]
use std::{io, mem};

/// Puts the calling thread on the frame loop's core under `SCHED_FIFO`, at the brain's
/// real-time priority.
pub fn realtime() {
    pin(cfg::FRAME_LOOP_CORES, libc::SCHED_FIFO, cfg::REALTIME_PRIORITY);
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

    // Zeroed and then set, rather than written out: musl's `sched_param` carries sporadic-
    // server fields glibc's lacks, which `SCHED_FIFO` and `SCHED_OTHER` ignore.
    //
    // SAFETY: `sched_param` is plain data, for which all zeroes is a valid value.
    let mut param: libc::sched_param = unsafe { mem::zeroed() };
    param.sched_priority = priority;
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
