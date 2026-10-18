//! The brain's state machine: which scene the rig is in, frame by frame.
//!
//! The scenes, and what moves the rig between them. Each frame the rule highest in this list
//! that applies decides, so the e-stop beats test mode and test mode beats the calendar:
//!
//! | from              | when                              | to                          |
//! |-------------------|-----------------------------------|-----------------------------|
//! | any               | the e-stop is pressed, or faulty  | e-stop                      |
//! | e-stop            | it is released                    | whatever the rules below say |
//! | any but e-stop    | test mode is on                   | test mode's scene; the left button steps it |
//! | blackout          | a show starts                     | daylight, or pulse or strobe if it is dark |
//! | daylight          | civil dusk                        | pulse or strobe             |
//! | pulse             | music starts                      | strobe                      |
//! | strobe            | music stops                       | pulse                       |
//! | any show scene    | the show ends                     | blackout                    |
//!
//! The e-stop scene is a scene like the others, so it is reported like them. It is the show's
//! say only: a console that takes the rig over replaces the frame, e-stop included, and what it
//! sends is the operator's responsibility.
//!
//! The state is the brain's own: `brain/scene` and `brain/testmode`, retained, published on
//! every change, and logged alongside.

use crate::face;
use crate::test_mode::TestMode;
use cortex::audio_features::AudioFeatures;
use cortex::calendar::{Calendar, Show};
use cortex::telemetry::Publisher;

/// The motor runs this long, then rests, on a beat counted from the start of the show, so the
/// show opens with a run.
const MOTOR_RUN_S: f64 = 1.0;
const MOTOR_REST_S: f64 = 29.0;
/// How late into its window a run may still begin: the frame loop's own jitter, and not
/// enough for what starts to be the remainder of a run rather than a run. Past it, the window
/// is skipped — after a release of the e-stop, after test mode hands back, or after NTP steps
/// the clock into one.
const MOTOR_START_GRACE_S: f64 = 0.1;

#[derive(Clone, Copy, PartialEq)]
pub enum Scene {
    /// The e-stop is pressed: every slot zero, at the usual rate, so nothing downstream sees a
    /// lost stream.
    Estop,
    /// No show: dark but for the ball's status LED, and the motor still.
    Blackout,
    /// A show, with the sun up: the motor on its beat, and the ball's power LEDs dark.
    Daylight,
    /// A show after dusk, in silence: the motor, and the power LEDs pulsing.
    Pulse,
    /// A show after dusk, with music: the motor, and the power LEDs strobing to it.
    Strobe,
}

impl Scene {
    /// The scene after this one, for stepping through them by hand. The e-stop is not among
    /// them: only the e-stop itself puts the rig there.
    pub fn next(self) -> Scene {
        match self {
            Scene::Estop | Scene::Blackout => Scene::Daylight,
            Scene::Daylight => Scene::Pulse,
            Scene::Pulse => Scene::Strobe,
            Scene::Strobe => Scene::Blackout,
        }
    }

    fn word(self) -> &'static str {
        match self {
            Scene::Estop => "estop",
            Scene::Blackout => "blackout",
            Scene::Daylight => "daylight",
            Scene::Pulse => "pulse",
            Scene::Strobe => "strobe",
        }
    }
}

/// What the rig does this frame.
#[derive(Clone, Copy)]
pub struct Cue {
    pub scene: Scene,
    pub motor: bool,
}

pub struct Director {
    calendar: &'static Calendar,
    telemetry: Publisher,
    test: TestMode,
    /// Whether the motor's run in the current beat is underway.
    running: bool,
    /// What was last reported, so only changes are.
    reported: Option<(Scene, bool)>,
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

    /// The cue for the moment `now`, in Unix seconds.
    ///
    /// The e-stop cancels a motor run outright rather than pausing it, so releasing it never
    /// starts the motor partway through its second.
    pub fn cue(&mut self, now: f64, features: &AudioFeatures) -> Cue {
        let estop = face::estop_pressed();
        // Test mode starts from the scene the rig is in, unless that is the e-stop, which it
        // cannot hold; it then starts from blackout.
        let current = match self.reported {
            Some((Scene::Estop, _)) | None => Scene::Blackout,
            Some((scene, _)) => scene,
        };
        // Every frame, e-stop or not, so test mode keeps up with the buttons and its timeout.
        let test = self.test.frame(current);
        let cue = if estop {
            self.running = false;
            Cue {
                scene: Scene::Estop,
                motor: false,
            }
        } else if let Some(cue) = test {
            self.running = false;
            cue
        } else {
            self.from_calendar(now, features)
        };
        self.report(cue.scene, self.test.is_on());
        cue
    }

    fn from_calendar(&mut self, now: f64, features: &AudioFeatures) -> Cue {
        let show = self.calendar.current(now);
        let scene = match show {
            None => Scene::Blackout,
            Some(show) if !show.is_dark(now) => Scene::Daylight,
            Some(_) if features.hears_music() => Scene::Strobe,
            Some(_) => Scene::Pulse,
        };
        self.running = show.is_some_and(|show| self.on_beat(show, now));
        Cue {
            scene,
            motor: self.running,
        }
    }

    /// Whether the motor runs now: inside a run window, and only if the run began at its
    /// head.
    fn on_beat(&self, show: &Show, now: f64) -> bool {
        let phase = (now - show.span.start.unix as f64).rem_euclid(MOTOR_RUN_S + MOTOR_REST_S);
        phase < MOTOR_RUN_S && (self.running || phase < MOTOR_START_GRACE_S)
    }

    fn report(&mut self, scene: Scene, test: bool) {
        if self.reported == Some((scene, test)) {
            return;
        }
        if self.reported.is_none_or(|(_, was)| was != test) {
            eprintln!("show: test mode {}", if test { "on" } else { "off" });
            self.telemetry.publish("testmode", rig_mqtt::boolean(test), true);
            face::light_lamps(test);
        }
        if self.reported.is_none_or(|(was, _)| was != scene) {
            match self.reported {
                Some((was, _)) => eprintln!("show: scene {} → {}", was.word(), scene.word()),
                None => eprintln!("show: scene {}", scene.word()),
            }
            self.telemetry.publish("scene", scene.word(), true);
        }
        self.reported = Some((scene, test));
    }
}
