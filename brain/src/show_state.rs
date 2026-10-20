//! The show state machine: which show state the show is in, frame by frame.
//!
//! The show states, and what moves the show between them. Each frame the rule highest in this
//! list that applies decides, so test mode beats the calendar:
//!
//! | from             | when            | to                                               |
//! |------------------|-----------------|--------------------------------------------------|
//! | any              | test mode is on | test mode's show state; the left button steps it |
//! | blackout         | a show starts   | daylight, or pulse or strobe if it is dark       |
//! | daylight         | civil dusk      | pulse or strobe                                  |
//! | pulse            | music starts    | strobe                                           |
//! | strobe           | music stops     | pulse                                            |
//! | any but blackout | the show ends   | blackout                                         |
//!
//! The e-stop is not among them. It is a rig state, in the frame loop's rig state machine, a
//! layer above this one, and the show runs on underneath it unaware.
//!
//! The show state is the brain's own: `brain/state/show` and `brain/state/testmode`, retained,
//! published on every change, and logged alongside.

use crate::face::FaceState;
use crate::test_mode::TestMode;
use cortex::ear::AudioFeatures;
use cortex::calendar::{Calendar, Show};
use cortex::telemetry::Publisher;

/// The motor runs this long, then rests, on a beat counted from the start of the show, so the
/// show opens with a run.
const MOTOR_RUN_S: f64 = 1.0;
const MOTOR_REST_S: f64 = 240.0;
/// How late into its window a run may still begin: the frame loop's own jitter, and not
/// enough for what starts to be the remainder of a run rather than a run. Past it, the window
/// is skipped — after test mode hands back, or after NTP steps the clock into one.
const MOTOR_START_GRACE_S: f64 = 0.1;
/// How long before a show the ball is no longer asked to sleep: its five-minute stretch of
/// sleep, a wake, and a margin, so it is up when the show starts.
const BALL_WAKE_AHEAD_S: f64 = 10.0 * 60.0;

#[derive(Clone, Copy, PartialEq)]
pub enum ShowState {
    /// No show: dark but for the ball's status LED, and the motor still.
    Blackout,
    /// A show, with the sun up: the motor on its beat, and the ball's power LEDs dark.
    Daylight,
    /// A show after dusk, in silence: the motor, and the power LEDs pulsing.
    Pulse,
    /// A show after dusk, with music: the motor, and the power LEDs strobing to it.
    Strobe,
}

impl ShowState {
    /// The show state after this one, for stepping through them by hand.
    pub fn next(self) -> ShowState {
        match self {
            ShowState::Blackout => ShowState::Daylight,
            ShowState::Daylight => ShowState::Pulse,
            ShowState::Pulse => ShowState::Strobe,
            ShowState::Strobe => ShowState::Blackout,
        }
    }

    /// The show state as it is published on `brain/state/show`.
    fn word(self) -> &'static str {
        match self {
            ShowState::Blackout => "blackout",
            ShowState::Daylight => "daylight",
            ShowState::Pulse => "pulse",
            ShowState::Strobe => "strobe",
        }
    }
}

/// What the show does this frame.
#[derive(Clone, Copy)]
pub struct Cue {
    pub state: ShowState,
    pub motor: bool,
    /// Whether test mode is on, which the face's lamps show.
    pub testing: bool,
    /// Whether the ball may sleep: no show now, and none starting soon.
    pub sleep: bool,
}

pub struct Director {
    calendar: &'static Calendar,
    telemetry: Publisher,
    test: TestMode,
    /// Whether the motor's run in the current beat is underway.
    running: bool,
    /// What was last reported, so only changes are.
    reported: Option<(ShowState, bool)>,
}

impl Director {
    pub fn new(calendar: &'static Calendar, telemetry: Publisher) -> Self {
        Self {
            calendar,
            telemetry,
            test: TestMode::new(),
            running: false,
            reported: None,
        }
    }

    /// The cue for the moment `now`, in Unix seconds, given what the room sounds like and
    /// what the door's buttons say.
    pub fn cue(&mut self, now: f64, features: &AudioFeatures, face: &FaceState) -> Cue {
        // Test mode starts from the show state the show is in.
        let current = self.reported.map_or(ShowState::Blackout, |(state, _)| state);
        let cue = if let Some(cue) = self.test.frame(current, face) {
            self.running = false;
            cue
        } else {
            self.from_calendar(now, features)
        };
        self.report(cue.state, cue.testing);
        cue
    }

    fn from_calendar(&mut self, now: f64, features: &AudioFeatures) -> Cue {
        let show = self.calendar.current(now);
        let state = match show {
            None => ShowState::Blackout,
            Some(show) if !show.is_dark(now) => ShowState::Daylight,
            Some(_) if features.hears_music() => ShowState::Strobe,
            Some(_) => ShowState::Pulse,
        };
        self.running = show.is_some_and(|show| self.on_beat(show, now));
        let show_soon = self.calendar.shows.iter().any(|show| {
            let start = show.span.start.unix as f64;
            start > now && start - now < BALL_WAKE_AHEAD_S
        });
        Cue {
            state,
            motor: self.running,
            testing: false,
            sleep: show.is_none() && !show_soon,
        }
    }

    /// Whether the motor runs now: inside a run window, and only if the run began at its
    /// head.
    fn on_beat(&self, show: &Show, now: f64) -> bool {
        let phase = (now - show.span.start.unix as f64).rem_euclid(MOTOR_RUN_S + MOTOR_REST_S);
        phase < MOTOR_RUN_S && (self.running || phase < MOTOR_START_GRACE_S)
    }

    fn report(&mut self, state: ShowState, test: bool) {
        if self.reported == Some((state, test)) {
            return;
        }
        if self.reported.is_none_or(|(_, was)| was != test) {
            eprintln!("show: test mode {}", if test { "on" } else { "off" });
            self.telemetry.publish("state/testmode", rig_mqtt::boolean(test), true);
        }
        if self.reported.is_none_or(|(was, _)| was != state) {
            match self.reported {
                Some((was, _)) => eprintln!("show: {} → {}", was.word(), state.word()),
                None => eprintln!("show: {}", state.word()),
            }
            self.telemetry.publish("state/show", state.word(), true);
        }
        self.reported = Some((state, test));
    }
}
