//! The show: the Director, which chooses the show state every frame, and what each state asks
//! of the fixtures.
//!
//! Test mode first, then the calendar. Outside a show the state is `blackout`: dark. Inside
//! one, or in test mode whatever the calendar says, it is `idle`: the face's left
//! lamp breathes, a slow sine from dark to full, so the rig can be seen running from the
//! cabinet door.
//!
//! The chord enters test mode. Test mode ends itself after `TEST_MODE_TIMEOUT` without a chord,
//! because a rig left in it would ignore the calendar unattended.
//!
//! Wrist is plucked from the right button, whatever the state: Wrist is sent Hold, and the
//! right lamp is lit while Wrist says a pluck can start. A press while it is lit sends Pluck
//! until Wrist no longer says it can, and Pluck goes on being sent while Wrist says the pluck
//! is busy; then Hold again, until the next press, whatever Wrist says after. A press while
//! the lamp is dark does nothing, and a press Wrist
//! still says it can after a second of is let go of: the Pluck never reached it. The firmware
//! decides nothing: the brain holds Pluck exactly as long as the pluck runs.
//!
//! The show state is the brain's own: `brain/show` and `brain/testmode`, retained, published
//! on every change, and logged with what caused it.

use crate::face::FaceState;
use crate::face_show;
use crate::wrist::{Heard, Pluck};
use crate::wrist_show::{self, Tug};
use cortex::calendar::{self, Calendar};
use cortex::config::FRAME_RATE_HZ;
use cortex::telemetry::Publisher;
use std::f64::consts::TAU;
use std::time::{Duration, Instant};

/// One breath of the left lamp in `idle`, dark to full and back.
const PULSE_SECS: f64 = 4.0;
/// How long test mode lasts without a chord.
const TEST_MODE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// How long what Wrist says is believed: ten of its state bundles.
const WRIST_STALE_AFTER: Duration = Duration::from_secs(1);
/// How many frames a press waits for Wrist to leave `can`: as long as what Wrist says is
/// believed. Wrist leaves `can` on the first Pluck frame it reads and says so in its next state
/// bundle, a tenth of a second later. Still saying it after this, it has not been sent the
/// press's Pluck — a desk holding the universe, frames lost on the way — and a press that waited
/// on would pluck whenever a frame next got through, unattended.
const PLUCK_PRESS_FRAMES: u32 = (WRIST_STALE_AFTER.as_millis() as u64 * FRAME_RATE_HZ / 1000) as u32;
/// Pluck's tug: Wrist's largest, four revolutions, at its top speed from the first step to the
/// last; clockwise, facing the shaft, which reels the wire in, by test.
const PLUCK_TUG: Tug = Tug {
    size: 1.0,
    speed: 1.0,
    clockwise: true,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Blackout,
    Idle,
}

impl State {
    /// The state as it is logged and published on `brain/show`.
    fn word(self) -> &'static str {
        match self {
            State::Blackout => "blackout",
            State::Idle => "idle",
        }
    }
}

pub struct Show {
    telemetry: Publisher,
    calendar: &'static Calendar,
    /// The face's chord count as last read; the face counts from zero, and so does this.
    seen_chords: u32,
    /// When test mode was last turned on, while it is on.
    testing_since: Option<Instant>,
    state: State,
    /// Frames already played in the current state.
    frames: u32,
    /// The face's right-button count as last read; the face counts from zero, and so does this.
    seen_right_presses: u32,
    /// A press of the lit right button, held until Wrist no longer says a pluck can start, or
    /// for `PLUCK_PRESS_FRAMES` if it goes on saying so: the frames it has been held.
    pluck_pressed: Option<u32>,
    /// Whether Wrist was sent Pluck in the last frame, so a change can be logged.
    plucking: bool,
}

impl Show {
    pub fn new(telemetry: Publisher, calendar: &'static Calendar) -> Self {
        let state = State::Blackout;
        eprintln!("show: {}", state.word());
        telemetry.publish("show", state.word(), true);
        eprintln!("show: test mode off");
        telemetry.publish("testmode", stem::boolean(false), true);
        Self {
            telemetry,
            calendar,
            seen_chords: 0,
            testing_since: None,
            state,
            frames: 0,
            seen_right_presses: 0,
            pluck_pressed: None,
            plucking: false,
        }
    }

    /// Fills this frame's slots, given what the door and Wrist say.
    pub fn fill(&mut self, slots: &mut [u8], face: &FaceState, wrist: Heard) {
        // A chord takes seconds to make, so one frame never sees two.
        let chorded = face.chords != self.seen_chords;
        self.seen_chords = face.chords;

        if chorded {
            self.set_testing(self.testing_since.is_none());
        } else if self
            .testing_since
            .is_some_and(|since| since.elapsed() >= TEST_MODE_TIMEOUT)
        {
            eprintln!(
                "show: test mode timed out after {} minutes without a chord",
                TEST_MODE_TIMEOUT.as_secs() / 60
            );
            self.set_testing(false);
        }
        let testing = self.testing_since.is_some();

        if let Some((next, why)) = self.next(testing) {
            self.enter(next, why);
        }
        let left = self.left_lamp();
        self.frames = self.frames.saturating_add(1);

        let wrist = wrist
            .filter(|(_, at)| at.elapsed() < WRIST_STALE_AFTER)
            .map(|(pluck, _)| pluck);
        let (pluck, can_pluck) = self.pluck(face, wrist);
        face_show::fill(slots, left, can_pluck);
        wrist_show::fill(slots, pluck, &PLUCK_TUG);
    }

    /// Whether Wrist is sent Pluck this frame, and whether the right lamp is lit, given what
    /// Wrist last said, if it is not stale.
    fn pluck(&mut self, face: &FaceState, wrist: Option<Pluck>) -> (bool, bool) {
        // A press takes tens of milliseconds, so one frame never sees two.
        let pressed = face.right_presses != self.seen_right_presses;
        self.seen_right_presses = face.right_presses;

        let can = wrist == Some(Pluck::Can);
        let mut expired = false;
        if let Some(held) = &mut self.pluck_pressed {
            *held += 1;
            expired = *held > PLUCK_PRESS_FRAMES;
        }
        if pressed && can && self.pluck_pressed.is_none() {
            self.pluck_pressed = Some(0);
        }
        // Until Wrist has seen the Pluck frame it goes on saying it can; after, the pluck is
        // Wrist's to say. One that never sees it is let go of.
        if !can || expired {
            self.pluck_pressed = None;
        }
        let pressing = self.pluck_pressed.is_some();
        // Busy only keeps a Pluck going, never starts one. Once Hold has gone out, a busy still
        // on its way, or arriving after a silence, is the pluck that was ended, and Wrist, in
        // Hold, would take Pluck again as a new one.
        let pluck = pressing || (self.plucking && wrist == Some(Pluck::Busy));
        if pluck != self.plucking {
            let why = if pressing {
                "right button"
            } else if expired {
                "Wrist never saw the press"
            } else if wrist.is_none() {
                "Wrist not heard for a second"
            } else {
                "pluck done"
            };
            eprintln!("show: Wrist {} — {why}", if pluck { "Pluck" } else { "Hold" });
            self.plucking = pluck;
        }
        (pluck, can && !pressing)
    }

    /// The state this frame moves to, if it moves, and why.
    fn next(&self, testing: bool) -> Option<(State, &'static str)> {
        let on = self.calendar.current(calendar::now()).is_some();
        match self.state {
            State::Idle if !testing && !on => Some((State::Blackout, "outside a show")),
            State::Blackout if testing => Some((State::Idle, "test mode")),
            State::Blackout if on => Some((State::Idle, "a show has begun")),
            _ => None,
        }
    }

    fn enter(&mut self, next: State, why: &str) {
        eprintln!("show: {} → {} — {why}", self.state.word(), next.word());
        self.telemetry.publish("show", next.word(), true);
        self.state = next;
        self.frames = 0;
    }

    /// The left lamp's level this frame, 0..1. Starting dark, so `idle` fades in.
    fn left_lamp(&self) -> f64 {
        match self.state {
            State::Blackout => 0.0,
            State::Idle => {
                let secs = f64::from(self.frames) / FRAME_RATE_HZ as f64;
                (1.0 - (TAU * secs / PULSE_SECS).cos()) / 2.0
            }
        }
    }

    fn set_testing(&mut self, on: bool) {
        self.testing_since = on.then(Instant::now);
        eprintln!("show: test mode {}", if on { "on" } else { "off" });
        self.telemetry.publish("testmode", stem::boolean(on), true);
    }
}
