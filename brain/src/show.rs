//! The show: the Director, which chooses the show state every frame, and what each state asks
//! of the sound and the face.
//!
//! Test mode first, then the calendar, then, inside a show, presence and the timers. Outside a
//! show, and inside one until civil dusk, the state is `blackout`: dark and silent. Test mode
//! runs the show whatever the calendar says, and turned on in `blackout` it starts in `idle`, so
//! the heart can be looked at without a scare first.
//!
//! `idle` is the heart at the pace presence sets, and an owl now and then. The scare is one
//! sequence: `cut` and `buildup`, dark and silent, then `lightning`, a flash that leads the
//! thunderclap in, then `thunderclap`, as long as the recording, then `respite` until the yard
//! is empty, and back to `idle`.
//!
//! For now the face is the only fixture the show drives: its left lamp is what the rose would
//! do, and its right lamp says test mode is on. The heads and the door wash join once their
//! presets are aimed.
//!
//! In test mode the right button fires the scare from any state, restarting one in progress,
//! and the left button steps presence through a few distances. Outside it the buttons do
//! nothing but the chord that enters it. Test mode ends itself after `TEST_MODE_TIMEOUT`
//! without a chord, because a rig left in it would ignore the calendar unattended.
//!
//! The show state is the brain's own: `brain/show` and `brain/testmode`, retained, published
//! on every change, and logged with what caused it.

use crate::face::FaceState;
use crate::face_show;
use crate::heart::Heart;
use crate::mouth_show::MouthShow;
use crate::presence::{Presence, Reading};
use cortex::calendar::{self, Calendar};
use cortex::config::FRAME_RATE_HZ;
use cortex::draw::Draw;
use cortex::telemetry::Publisher;
use std::time::{Duration, Instant};

/// The resting heart, and the heart at the trigger line.
pub const REST_BPM: f64 = 55.0;
pub const SCARE_BPM: f64 = 130.0;
/// The distance, in paths, over which the heart goes from rest to the scare rate: the whole
/// walk from the yard entry. Less leaves the first steps at rest.
pub const HEART_RANGE: f64 = 1.0;
/// How fast the heart changes pace: the glide closes 63 % of the gap to its target in this
/// time, and 95 % in three times it. A heart changes pace gradually, and the glide swallows a
/// detector that loses someone for a frame.
pub const RATE_TAU: f64 = 1.5;
/// The rose at the quietest point of the heartbeat; full at its loudest.
pub const ROSE_FLOOR: f64 = 0.25;
/// The gap between owls, drawn uniformly. Far apart, so an owl stays an event.
const OWL_MIN_SECS: f64 = 45.0;
const OWL_MAX_SECS: f64 = 120.0;
/// Any constant: what varies the owls from one `idle` to the next is the time it began.
const OWL_SEED: u64 = 0x6f77_6c;
/// How long the distance must hold at or past the trigger line before the scare: a couple of
/// the eyeball's frames rather than one bad box.
pub const SCARE_CONFIRM_SECS: f64 = 0.2;
/// How long the yard must be empty before `respite` ends, so a detector that loses the group
/// for a frame does not reset the show on them.
pub const LEAVE_SECS: f64 = 3.0;
/// The darkness and silence of the cut, hard.
const CUT_SECS: f64 = 1.0;
/// Still dark and silent while the heads move to "Thunderclap": the longest move plus a margin,
/// measured on the rig. Also how long the rose stays dark while the heads travel back.
pub const BUILDUP_SECS: f64 = 1.0;
/// The flash before the thunderclap: two frames, chosen at the bench. The recording starts on
/// the frame after, so the light leads the crack.
const LIGHTNING_SECS: f64 = 0.05;
/// How long test mode lasts without a chord.
const TEST_MODE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// `secs` as frames: the nearest whole number, and at least one, so a time shorter than a
/// frame still happens.
pub fn frames(secs: f64) -> u32 {
    ((secs * FRAME_RATE_HZ as f64).round() as u32).max(1)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Blackout,
    Idle,
    Cut,
    Buildup,
    Lightning,
    Thunderclap,
    Respite,
}

impl State {
    /// The state as it is logged and published on `brain/show`.
    fn word(self) -> &'static str {
        match self {
            State::Blackout => "blackout",
            State::Idle => "idle",
            State::Cut => "cut",
            State::Buildup => "buildup",
            State::Lightning => "lightning",
            State::Thunderclap => "thunderclap",
            State::Respite => "respite",
        }
    }
}

/// The owl's calls in one stretch of `idle`, each at a random gap after the last.
struct Owl {
    /// What the stretch's draws are keyed on: when it began.
    key: i64,
    calls: i64,
    frames_left: u32,
}

impl Owl {
    fn new(key: i64) -> Self {
        let mut owl = Owl {
            key,
            calls: 0,
            frames_left: 0,
        };
        owl.frames_left = owl.gap();
        owl
    }

    fn gap(&self) -> u32 {
        frames(Draw::new(OWL_SEED, &[self.key, self.calls]).between(OWL_MIN_SECS, OWL_MAX_SECS))
    }

    /// Steps one frame; whether the owl calls on it.
    fn step(&mut self) -> bool {
        if self.frames_left > 0 {
            self.frames_left -= 1;
            return false;
        }
        self.calls += 1;
        self.frames_left = self.gap();
        true
    }
}

pub struct Show {
    telemetry: Publisher,
    calendar: &'static Calendar,
    /// The face's counts as last read; the face counts from zero, and so does this.
    seen: FaceState,
    /// When test mode was last turned on, while it is on.
    testing_since: Option<Instant>,
    state: State,
    /// Frames already played in the current state.
    frames: u32,
    presence: Presence,
    heart: Heart,
    owl: Owl,
    mouth: MouthShow,
}

impl Show {
    pub fn new(telemetry: Publisher, calendar: &'static Calendar, mouth: MouthShow) -> Self {
        let state = State::Blackout;
        eprintln!("show: {}", state.word());
        telemetry.publish("show", state.word(), true);
        eprintln!("show: test mode off");
        telemetry.publish("testmode", stem::boolean(false), true);
        Self {
            presence: Presence::new(telemetry.clone()),
            telemetry,
            calendar,
            seen: FaceState {
                estop: false,
                chords: 0,
                left_presses: 0,
                right_presses: 0,
            },
            testing_since: None,
            state,
            frames: 0,
            heart: Heart::new(MouthShow::heartbeat_envelope()),
            owl: Owl::new(0),
            mouth,
        }
    }

    /// Fills this frame's slots, given what the door says.
    pub fn fill(&mut self, slots: &mut [u8], face: &FaceState) {
        let chords = face.chords.wrapping_sub(self.seen.chords);
        let left_presses = face.left_presses.wrapping_sub(self.seen.left_presses);
        let right_presses = face.right_presses.wrapping_sub(self.seen.right_presses);
        self.seen = *face;

        if chords % 2 == 1 {
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
        if testing {
            for _ in 0..left_presses {
                self.presence.cycle();
            }
        }

        let reading = self.presence.step(testing);
        if let Some((next, why)) = self.next(testing, testing && right_presses > 0, &reading) {
            self.enter(next, why);
        }
        let rose = self.play(&reading);
        self.frames = self.frames.saturating_add(1);

        // The e-stop is a show-stop, so it silences the sound with the light.
        let silent = matches!(self.state, State::Blackout | State::Cut | State::Buildup);
        self.mouth.mute(face.estop || silent);
        face_show::fill(slots, rose, testing);
    }

    /// The state this frame moves to, if it moves, and why, where that is worth saying.
    fn next(
        &self,
        testing: bool,
        scare: bool,
        reading: &Reading,
    ) -> Option<(State, Option<String>)> {
        let now = calendar::now();
        let dark = self
            .calendar
            .current(now)
            .is_some_and(|show| show.dark_stretch(now).is_some());
        if !testing && !dark {
            return (self.state != State::Blackout).then(|| {
                (
                    State::Blackout,
                    Some("outside a show's dark hours".to_string()),
                )
            });
        }
        if scare {
            return Some((State::Cut, Some("the right button".to_string())));
        }

        let played = self.frames;
        match self.state {
            State::Blackout => {
                let why = if testing {
                    "test mode"
                } else {
                    "a show's dark hours have begun"
                };
                Some((State::Idle, Some(why.to_string())))
            }
            State::Idle if reading.scare => Some((
                State::Cut,
                Some(format!(
                    "the furthest at {:.2}, past the trigger line",
                    reading.distance
                )),
            )),
            State::Cut if played >= frames(CUT_SECS) => Some((State::Buildup, None)),
            State::Buildup if played >= frames(BUILDUP_SECS) => Some((State::Lightning, None)),
            State::Lightning if played >= frames(LIGHTNING_SECS) => {
                Some((State::Thunderclap, None))
            }
            State::Thunderclap if played as usize >= MouthShow::thunderclap_envelope().len() => {
                Some((State::Respite, Some("the recording ended".to_string())))
            }
            State::Respite if reading.vacated => Some((
                State::Idle,
                Some(format!("the yard has been empty for {LEAVE_SECS:.1} s")),
            )),
            _ => None,
        }
    }

    fn enter(&mut self, next: State, why: Option<String>) {
        match why {
            Some(why) => eprintln!("show: {} → {} — {why}", self.state.word(), next.word()),
            None => eprintln!("show: {} → {}", self.state.word(), next.word()),
        }
        self.telemetry.publish("show", next.word(), true);
        self.state = next;
        self.frames = 0;
        match next {
            State::Idle => {
                self.heart.restart();
                self.owl = Owl::new(calendar::now() as i64);
            }
            State::Thunderclap => self.mouth.thunderclap(),
            State::Blackout | State::Cut | State::Buildup | State::Lightning | State::Respite => {}
        }
    }

    /// Plays this frame of the current state, and returns what the rose does: its level, 0..1.
    fn play(&mut self, reading: &Reading) -> f64 {
        match self.state {
            State::Idle => {
                let (beat, rose) = self.heart.step(reading.distance);
                if beat {
                    self.mouth.heartbeat();
                }
                if self.owl.step() {
                    self.mouth.owl();
                }
                rose
            }
            State::Lightning => 1.0,
            State::Thunderclap => MouthShow::thunderclap_envelope()
                .get(self.frames as usize)
                .map_or(0.0, |&level| f64::from(level) / f64::from(u8::MAX)),
            State::Blackout | State::Cut | State::Buildup | State::Respite => 0.0,
        }
    }

    fn set_testing(&mut self, on: bool) {
        self.testing_since = on.then(Instant::now);
        if !on {
            self.presence.reset();
        }
        eprintln!("show: test mode {}", if on { "on" } else { "off" });
        self.telemetry.publish("testmode", stem::boolean(on), true);
    }
}
