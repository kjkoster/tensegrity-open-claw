//! The show state machine: which show state the show is in, frame by frame.
//!
//! The show states, and what moves the show between them. Each frame the rule highest in this
//! list that applies decides, so test mode beats the calendar:
//!
//! | from             | when                   | to                                        |
//! |------------------|------------------------|-------------------------------------------|
//! | any              | test mode is on        | test mode's show state; the left button steps it |
//! | blackout         | a show starts          | daylight, or pulse or strobe if it is dark |
//! | daylight         | civil dusk             | pulse or strobe                           |
//! | pulse or strobe  | an incident starts     | that incident                             |
//! | an incident      | its scheduled end      | pulse or strobe                           |
//! | pulse            | music starts           | strobe                                    |
//! | strobe           | music stops            | pulse                                     |
//! | any but blackout | the show ends          | blackout                                  |
//!
//! Pulse, strobe, daylight and blackout are the base states, chosen by the calendar and the
//! room. An incident is chosen by the schedule, and only in place of pulse or strobe; it
//! carries on unchanged when the music starts or stops.
//!
//! The e-stop is not among them. It is a rig state, in the frame loop's rig state machine, a
//! layer above this one, and the show runs on underneath it unaware.
//!
//! The show state is the brain's own: `brain/state/show` and `brain/state/testmode`, retained,
//! published on every change, and logged alongside.

use crate::face::FaceState;
use crate::schedule::{self, Scheduled};
use crate::test_mode::TestMode;
use cortex::calendar::{Calendar, Show, Span};
use cortex::ear::AudioFeatures;
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

/// The show states the calendar and the room choose between.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Base {
    /// No show: dark but for the ball's status LED, and the motor still.
    Blackout,
    /// A show, with the sun up: the motor on its beat, and the ball's power LEDs dark.
    Daylight,
    /// A show after dusk, in silence: the motor, and the power LEDs pulsing.
    Pulse,
    /// A show after dusk, with music: the motor, and the power LEDs strobing to it.
    Strobe,
}

/// A few minutes of something else, every half hour or so after dusk.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Incident {
    /// One head at a time snaps to a random point in its patch of sky, white.
    Searchlights,
    /// Heads, washes and ball fade together between white and each leg's colour.
    BallLighting,
    /// All red, beating a lub-dub, the heads breathing out from the centre with it.
    Heart,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ShowState {
    Base(Base),
    Incident(Incident),
}

impl ShowState {
    /// The show state after this one, for stepping through them by hand: the base states,
    /// then the incidents.
    pub fn next(self) -> ShowState {
        match self {
            ShowState::Base(Base::Blackout) => ShowState::Base(Base::Daylight),
            ShowState::Base(Base::Daylight) => ShowState::Base(Base::Pulse),
            ShowState::Base(Base::Pulse) => ShowState::Base(Base::Strobe),
            ShowState::Base(Base::Strobe) => ShowState::Incident(Incident::Searchlights),
            ShowState::Incident(Incident::Searchlights) => ShowState::Incident(Incident::BallLighting),
            ShowState::Incident(Incident::BallLighting) => ShowState::Incident(Incident::Heart),
            ShowState::Incident(Incident::Heart) => ShowState::Base(Base::Blackout),
        }
    }

    /// The show state as it is published on `brain/state/show`.
    fn word(self) -> &'static str {
        match self {
            ShowState::Base(Base::Blackout) => "blackout",
            ShowState::Base(Base::Daylight) => "daylight",
            ShowState::Base(Base::Pulse) => "pulse",
            ShowState::Base(Base::Strobe) => "strobe",
            ShowState::Incident(Incident::Searchlights) => "searchlights",
            ShowState::Incident(Incident::BallLighting) => "ball_lighting",
            ShowState::Incident(Incident::Heart) => "heart",
        }
    }
}

/// What the washes and the ball wear. Searchlights leaves them to the base show, so it has no
/// look of its own here.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Look {
    Base(Base),
    BallLighting,
    Heart,
}

/// What the show does this frame.
#[derive(Clone, Copy)]
pub struct Cue {
    pub state: ShowState,
    /// What the calendar and the room chose this frame, which an incident that runs the base
    /// show underneath it wears. Under test mode it is pulse or strobe by the room, as if it
    /// were dark, so an incident tested by day shows its night look.
    pub base: Base,
    /// Whether this is the state's first frame.
    pub entering: bool,
    /// The whole Unix second the state began — an incident's scheduled start — which is what
    /// draws inside the state are keyed on.
    pub state_key: i64,
    /// Seconds into the state. A scheduled incident counts from its scheduled start, so a
    /// restart mid-incident resumes its phase rather than starting it over; and the start
    /// stands for as long as the state does, test mode turned on or off included.
    pub state_elapsed: f64,
    /// Seconds to the state's scheduled end; `None` where it has none: the base states, and
    /// everything under test mode.
    pub state_remaining: Option<f64>,
    pub motor: bool,
    /// Whether test mode is on, which the face's lamps show.
    pub testing: bool,
    /// Whether the ball may sleep: no show now, and none starting soon.
    pub sleep: bool,
}

impl Cue {
    pub fn look(&self) -> Look {
        match self.state {
            ShowState::Base(base) => Look::Base(base),
            ShowState::Incident(Incident::Searchlights) => Look::Base(self.base),
            ShowState::Incident(Incident::BallLighting) => Look::BallLighting,
            ShowState::Incident(Incident::Heart) => Look::Heart,
        }
    }
}

pub struct Director {
    calendar: &'static Calendar,
    telemetry: Publisher,
    test: TestMode,
    /// Whether the motor's run in the current beat is underway.
    running: bool,
    /// The state the show is in and when it began.
    entered: Option<(ShowState, f64)>,
    /// The dark stretch whose schedule was last printed, by its start.
    announced: Option<i64>,
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
            entered: None,
            announced: None,
            reported: None,
        }
    }

    /// The cue for the moment `now`, in Unix seconds, given what the room sounds like and
    /// what the door's buttons say.
    pub fn cue(&mut self, now: f64, features: &AudioFeatures, face: &FaceState) -> Cue {
        let show = self.calendar.current(now);
        let stretch = show.and_then(|show| show.dark_stretch(now));
        if let Some(stretch) = stretch {
            self.announce(stretch);
        }

        // Test mode starts from the show state the show is in.
        let current = self.reported.map_or(ShowState::Base(Base::Blackout), |(state, _)| state);
        let (state, base, scheduled, motor, testing, sleep) = match self.test.frame(current, face) {
            Some(manual) => {
                self.running = false;
                let base = if features.hears_music() { Base::Strobe } else { Base::Pulse };
                (manual.state, base, None, manual.motor, true, false)
            }
            None => {
                let base = match show {
                    None => Base::Blackout,
                    Some(_) if stretch.is_none() => Base::Daylight,
                    Some(_) if features.hears_music() => Base::Strobe,
                    Some(_) => Base::Pulse,
                };
                // Pulse and strobe are exactly the base states a dark stretch gives.
                let scheduled = stretch.and_then(|stretch| schedule::running(stretch, now));
                let state = scheduled.map_or(ShowState::Base(base), |s| ShowState::Incident(s.incident));
                self.running = show.is_some_and(|show| self.on_beat(show, now));
                let show_soon = self.calendar.shows.iter().any(|show| {
                    let start = show.span.start.unix as f64;
                    start > now && start - now < BALL_WAKE_AHEAD_S
                });
                (state, base, scheduled, self.running, false, show.is_none() && !show_soon)
            }
        };

        // A scheduled incident always starts at its scheduled start, even where the show was
        // already in that state — test mode handing back into the incident it had stepped to.
        // Anything else starts when the show entered it, and keeps that start while it lasts.
        let start = scheduled.map_or(now, |scheduled| scheduled.start);
        let entering = self.entered.is_none_or(|(was, was_start)| {
            was != state || scheduled.is_some_and(|scheduled| scheduled.start != was_start)
        });
        if entering {
            self.entered = Some((state, start));
        }
        let start = self.entered.map_or(start, |(_, start)| start);

        self.report(state, testing);
        Cue {
            state,
            base,
            entering,
            state_key: start.floor() as i64,
            state_elapsed: now - start,
            state_remaining: scheduled.map(|scheduled| scheduled.end - now),
            motor,
            testing,
            sleep,
        }
    }

    /// Whether the motor runs now: inside a run window, and only if the run began at its
    /// head.
    fn on_beat(&self, show: &Show, now: f64) -> bool {
        let phase = (now - show.span.start.unix as f64).rem_euclid(MOTOR_RUN_S + MOTOR_REST_S);
        phase < MOTOR_RUN_S && (self.running || phase < MOTOR_START_GRACE_S)
    }

    /// Prints a dark stretch's whole schedule the first time the show is in it, so the log
    /// says what is to come — at dusk, and again after a restart.
    fn announce(&mut self, stretch: Span) {
        if self.announced == Some(stretch.start.unix) {
            return;
        }
        self.announced = Some(stretch.start.unix);
        let plan: Vec<Scheduled> = schedule::plan(stretch).collect();
        eprintln!(
            "show: dark {} → {}, {} incidents{}",
            stretch.start.time,
            stretch.stop.time,
            plan.len(),
            if schedule::ENABLED_INCIDENTS.is_empty() { " (none enabled)" } else { "" },
        );
        // Offsets from the stretch's start, which is dusk or the show's start, whichever is
        // later.
        for scheduled in plan {
            let offset = scheduled.start - stretch.start.unix as f64;
            eprintln!(
                "show:   #{} at +{}: {} for {}",
                scheduled.number,
                minutes(offset),
                ShowState::Incident(scheduled.incident).word(),
                minutes(scheduled.end - scheduled.start),
            );
        }
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

/// A span of seconds as minutes and seconds, for the log.
fn minutes(seconds: f64) -> String {
    let whole = seconds.round() as i64;
    format!("{}m{:02}s", whole / 60, whole % 60)
}
