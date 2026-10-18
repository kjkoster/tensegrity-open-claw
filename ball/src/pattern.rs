//! The power LEDs' patterns, drawn on the ball from what the rig last asked for.
//!
//! The rig sends a pattern and its levels rather than a brightness per frame: its frames are
//! 25 ms apart and arrive over WiFi in bunches, while the pulse holds its peak for 10 ms.
//! Drawn here, a pattern is timed by the ball's own clock and stepped once per PWM period.
//!
//! A new look that keeps the pattern is taken in as the pattern runs, so the strobe follows
//! the music from one flash to the next. One that changes the pattern starts it at once.

use embassy_futures::select::{Either, select};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    watch::{Receiver, Watch},
};
use embassy_time::{Instant, Timer};

use crate::config::{PULSE_DARK, PULSE_HOLD, PULSE_RISE};
use crate::led::{POWER_PWM_PERIOD, PowerLeds};
use crate::models::{Look, Pattern};

/// What the power LEDs are asked to do, written by the main loop on every packet.
pub static LOOK: Watch<CriticalSectionRawMutex, Look, 1> = Watch::new();

type Looks = Receiver<'static, CriticalSectionRawMutex, Look, 1>;

#[embassy_executor::task]
pub async fn render(power: PowerLeds) -> ! {
    let mut looks = LOOK.receiver().unwrap();
    let mut look = Look::DARK;
    loop {
        match look.pattern {
            Pattern::Direct => {
                power.set_brightness(look.white, look.blue);
                look = looks.changed().await;
            }
            Pattern::Pulse => pulse(&power, &mut looks, &mut look).await,
            Pattern::Strobe => strobe(&power, &mut looks, &mut look).await,
        }
    }
}

/// One pulse, or as much of one as runs before the pattern changes.
async fn pulse(power: &PowerLeds, looks: &mut Looks, look: &mut Look) {
    let start = Instant::now();
    let peak = start + PULSE_RISE;
    let mut at = start;
    while at < peak {
        let rise = (at - start).as_micros() as f32 / PULSE_RISE.as_micros() as f32;
        let level = ease(rise);
        power.set_brightness(scaled(look.white, level), scaled(look.blue, level));
        at += POWER_PWM_PERIOD;
        if !wait(looks, look, at).await {
            return;
        }
    }
    power.set_brightness(look.white, look.blue);
    if !wait(looks, look, peak + PULSE_HOLD).await {
        return;
    }
    power.set_brightness(0, 0);
    wait(looks, look, peak + PULSE_HOLD + PULSE_DARK).await;
}

/// One flash and the gap after it, or as much as runs before the pattern changes.
///
/// Neither phase is shorter than one PWM period. A zero length would be a loop that never
/// waits, and the executor would never get the core back.
async fn strobe(power: &PowerLeds, looks: &mut Looks, look: &mut Look) {
    power.set_brightness(look.white, look.blue);
    let dark = Instant::now() + look.strobe_on.max(POWER_PWM_PERIOD);
    if !wait(looks, look, dark).await {
        return;
    }
    power.set_brightness(0, 0);
    // The gap's length is read only now, so a look that arrived during the flash sets it.
    wait(looks, look, dark + look.strobe_off.max(POWER_PWM_PERIOD)).await;
}

/// Waits until `until`, taking in every look that arrives meanwhile. Returns false as soon as
/// one asks for a different pattern.
async fn wait(looks: &mut Looks, look: &mut Look, until: Instant) -> bool {
    loop {
        match select(Timer::at(until), looks.changed()).await {
            Either::First(()) => return true,
            Either::Second(next) => {
                let same = next.pattern == look.pattern;
                *look = next;
                if !same {
                    return false;
                }
            }
        }
    }
}

/// Smoothstep: an S-curve from 0 to 1 that leaves the dark and arrives at the peak gently,
/// the shape of a raised cosine to within a percent, without the trigonometry `core` lacks.
fn ease(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn scaled(level: u8, fraction: f32) -> u8 {
    (f32::from(level) * fraction + 0.5) as u8
}
