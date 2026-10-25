//! The face: the cabinet door — an e-stop, and a left and a right button with a green lamp in
//! each.
//!
//! The door is a fixture both ways. Its inputs go to the show as a [`FaceState`], published
//! once per poll; its lamps are patched in the workspace like any other fixture, and are driven
//! from the frame that actually goes out. So a console that takes the rig over drives them,
//! and the e-stop darkens them with everything else.
//!
//! Holding both buttons for two seconds is a chord. A left press is counted on release, not on
//! press, because only the release tells a button pressed alone from one that became half of
//! a chord, and neither of the presses that made a chord counts on its own. Presses and chords
//! are counters rather than flags, so a show that reads the state once per frame cannot miss
//! one that came and went between two frames. The right button is read as held instead, for
//! as long as it is down on its own.
//!
//! Inputs are polled rather than interrupt-driven, because there are no capacitors on the
//! lines: a contact bounces, and a level is only believed once it has held for a few
//! consecutive samples.
//!
//! The lamps dim through the Pi's two hardware PWM channels, not a software PWM: that is a
//! busy-waiting real-time thread per pin, competing with audio and the DMX timing.
//!
//! The face is a fixture of its own on the broker, with its own client beside the brain's:
//! `health/face`, and `face/left_button`, `face/right_button` and `face/estop`, retained.

use crate::patch;
use cortex::dmx::Frame;
use cortex::latest::{LatestRx, LatestTx, latest};
use cortex::qlc_plus::Channel;
use cortex::telemetry::Publisher;
use embassy_executor::Spawner;
use embassy_time::{Duration, Ticker};
use rppal::gpio::{Gpio, InputPin, Level};
use rppal::pwm::{self, Polarity, Pwm};

// BCM numbers. The RS-485 HAT owns 14, 15 and 18; none of those may appear here.
const ESTOP_NC: u8 = 23;
const ESTOP_NO: u8 = 24;
const LEFT_BUTTON: u8 = 27;
const RIGHT_BUTTON: u8 = 25;
// The lamps' pins, with the hardware PWM channel each is routed to.
const LEFT_LAMP: (u8, pwm::Channel) = (13, pwm::Channel::Pwm1);
const RIGHT_LAMP: (u8, pwm::Channel) = (12, pwm::Channel::Pwm0);

const POLL_PERIOD: Duration = Duration::from_millis(10);
/// Consecutive samples a new level must hold before it is accepted: 30 ms at the poll
/// period, longer than the contacts bounce and shorter than any deliberate press.
const SETTLE_SAMPLES: u8 = 3;
/// How long both buttons are held to make a chord: 2 s at the poll period, long enough that
/// leaning on the door does not do it.
const CHORD_SAMPLES: u16 = 200;
/// Far above anything the eye sees as flicker, and slow enough for the lamps' drivers.
const LAMP_PWM_HZ: f64 = 1000.0;

/// What the door says, as of the face's last poll.
#[derive(Clone, Copy)]
pub struct FaceState {
    /// Whether the rig must be stopped: the e-stop is pressed, or its wiring is at fault.
    pub estop: bool,
    /// Chords so far. Like the presses below, a count whose differences are what mean
    /// anything.
    pub chords: u32,
    /// Left presses so far, each pressed and released on its own.
    pub left_presses: u32,
    /// Whether the right button is down on its own, not as half of a chord.
    pub right_held: bool,
}

/// Tracks the chord across polls.
#[derive(Default)]
struct Chord {
    /// Consecutive polls with both buttons down.
    held: u16,
    /// Whether each button has been half of a chord since it was last pressed, which takes
    /// the left one's release out of the count and stops the right one counting as held.
    left_in_chord: bool,
    right_in_chord: bool,
}

/// The face's pins, claimed and configured, waiting for [`spawn`] to hand them to their
/// polling task.
pub struct Face {
    contacts: EstopContacts,
    left_button: Input,
    right_button: Input,
    left_lamp: Lamp,
    right_lamp: Lamp,
    /// The frames as they go out, which the lamps take their levels from.
    frames: LatestRx<Frame>,
    state: FaceState,
    states: LatestTx<FaceState>,
    telemetry: Publisher,
}

/// One lamp: its slot in the frame, and the PWM channel that dims it.
struct Lamp {
    gpio: u8,
    name: &'static str,
    channel: Channel,
    /// `None` when the PWM channel could not be opened. The lamps only tell an operator what
    /// the show is doing, so the show goes on without them.
    pwm: Option<Pwm>,
    level: u8,
}

struct Input {
    pin: InputPin,
    name: &'static str,
    /// A momentary button's topic under `face/`, where it is published as pressed or not, and
    /// annotated so in the trace. The e-stop contacts have none: what a level means depends
    /// on the other contact, and the derived e-stop says it.
    button: Option<&'static str>,
    level: Level,
    differing: u8,
}

/// The e-stop's two contacts, normally closed and normally open.
struct EstopContacts {
    nc: Input,
    no: Input,
}

/// What the two e-stop contacts say together. They must always disagree; when they agree
/// the face cannot tell a press from a fault, and either fault is treated as pressed.
#[derive(Clone, Copy)]
enum Estop {
    Released,
    Pressed,
    /// Both contacts open: the cable is broken or unplugged.
    CableFault,
    /// Both contacts closed.
    ContactFault,
}

/// Claims the face: the lamps dark, the inputs pulled up, and the e-stop read once. Call this
/// before anything else, so the lamps are dark, no input floats, and the very first frame
/// already knows the e-stop. Returns the face, and the reader of its state for the show and
/// the frame loop.
///
/// The first read is not debounced: at power-on nobody is mid-press, and the debouncing is
/// for contacts on the move.
///
/// Both buttons and the e-stop are published once here, so the tree holds the face's state
/// before anybody presses anything.
///
/// Panics if the GPIO cannot be opened. The brain only runs on the Pi, and a brain that
/// cannot read its e-stop must not light the rig.
pub fn claim(telemetry: Publisher, frames: LatestRx<Frame>) -> (Face, LatestRx<FaceState>) {
    let gpio = Gpio::new().unwrap_or_else(|e| panic!("face: gpio: {e}"));
    let pin = |number: u8| {
        gpio.get(number)
            .unwrap_or_else(|e| panic!("face: GPIO{number}: {e}"))
    };

    let left_lamp = Lamp::open(LEFT_LAMP, "left-lamp", patch::FACE.left_lamp);
    let right_lamp = Lamp::open(RIGHT_LAMP, "right-lamp", patch::FACE.right_lamp);
    let contacts = EstopContacts {
        nc: Input::new(pin(ESTOP_NC).into_input_pullup(), "estop-nc", None),
        no: Input::new(pin(ESTOP_NO).into_input_pullup(), "estop-no", None),
    };
    let button = |number, name, topic| Input::new(pin(number).into_input_pullup(), name, Some(topic));
    let left_button = button(LEFT_BUTTON, "left", "left_button");
    let right_button = button(RIGHT_BUTTON, "right", "right_button");
    for input in [&contacts.nc, &contacts.no, &left_button, &right_button] {
        input.trace();
        input.publish(&telemetry);
    }
    // Where the lamps sit on the wire, under the same topics any fixture uses, read off the patch
    // so they follow the workspace. The face has no address field: its first channel is its
    // start address.
    telemetry.publish("dmx/universe", patch::SACN.universe.to_string(), true);
    telemetry.publish("dmx/address", (patch::FACE.all()[0].slot() + 1).to_string(), true);

    let state = FaceState {
        estop: contacts.decide(&telemetry),
        chords: 0,
        left_presses: 0,
        right_held: false,
    };
    let (states, state_reader) = latest(state);
    let face = Face {
        contacts,
        left_button,
        right_button,
        left_lamp,
        right_lamp,
        frames,
        state,
        states,
        telemetry,
    };
    (face, state_reader)
}

/// Starts polling the inputs and driving the lamps.
pub fn spawn(spawner: Spawner, face: Face) {
    spawner.spawn(input_task(face).unwrap());
}

#[embassy_executor::task]
async fn input_task(mut face: Face) -> ! {
    let mut ticker = Ticker::every(POLL_PERIOD);
    // The e-stop's contacts do not move together: pressing opens NC before NO closes, and
    // releasing opens NO before NC closes, so for a moment either way both read open. Read off
    // the first contact to settle, that moment is a cable fault on every press. The e-stop is
    // therefore decided only once neither contact has read anything but its settled level for
    // a full settle period, and from both levels together: once per change, on the quiet
    // sample that completes the period. It starts out decided, by the read in `claim`.
    let mut estop_quiet = SETTLE_SAMPLES;
    let mut chord = Chord::default();
    loop {
        ticker.next().await;
        let contacts = &mut face.contacts;
        // `|`, not `||`: both contacts must be sampled every tick, or a change on the NO
        // contact would be debounced late whenever the NC contact changed too.
        if contacts.nc.poll(&face.telemetry) | contacts.no.poll(&face.telemetry) {
            // Towards stop at once, on whichever contact got there first. Only the decision
            // waits, and only it can release.
            if !matches!(contacts.read(), Estop::Released) {
                face.state.estop = true;
            }
            estop_quiet = 0;
        } else if contacts.nc.differing == 0 && contacts.no.differing == 0 {
            estop_quiet = estop_quiet.saturating_add(1);
            if estop_quiet == SETTLE_SAMPLES {
                face.state.estop = contacts.decide(&face.telemetry);
            }
        } else if estop_quiet < SETTLE_SAMPLES {
            // A bounce restarts a wait still running. Once decided, one that settles back
            // changes nothing, and one that does not is a change, which restarts it anyway.
            estop_quiet = 0;
        }

        let left_changed = face.left_button.poll(&face.telemetry);
        face.right_button.poll(&face.telemetry);
        chord.observe(
            &mut face.state,
            face.left_button.pressed(),
            face.right_button.pressed(),
            left_changed,
        );
        face.states.publish(face.state);

        let frame = face.frames.snapshot();
        face.left_lamp.follow(&frame);
        face.right_lamp.follow(&frame);
    }
}

impl Chord {
    /// Counts the gestures one poll completes and notes whether the right button is held on
    /// its own, from the buttons' settled levels and whether the left one just changed.
    fn observe(&mut self, state: &mut FaceState, left: bool, right: bool, left_changed: bool) {
        if left && right {
            self.left_in_chord = true;
            self.right_in_chord = true;
            self.held = self.held.saturating_add(1);
            // Equality rather than at-least, so a chord held on counts once.
            if self.held == CHORD_SAMPLES {
                state.chords = state.chords.wrapping_add(1);
            }
        } else {
            self.held = 0;
        }
        if left_changed && !left {
            if !self.left_in_chord {
                state.left_presses = state.left_presses.wrapping_add(1);
            }
            self.left_in_chord = false;
        }
        if !right {
            self.right_in_chord = false;
        }
        // A press that turns into a chord counts as held until the left button joins it, a
        // poll or a few at most.
        state.right_held = right && !self.right_in_chord;
    }
}

impl Lamp {
    /// Opens the lamp's PWM channel, dark. A channel that will not open is reported and left
    /// out; the usual cause is the overlay that routes PWM to the lamps' pins missing from
    /// `config.txt`.
    fn open((gpio, pwm_channel): (u8, pwm::Channel), name: &'static str, channel: Channel) -> Lamp {
        let pwm = match Pwm::with_frequency(pwm_channel, LAMP_PWM_HZ, 0.0, Polarity::Normal, true)
        {
            Ok(pwm) => Some(pwm),
            Err(e) => {
                eprintln!("face: GPIO{gpio} {name}: no PWM ({e}), so the lamp stays dark");
                None
            }
        };
        trace_lamp(gpio, name, 0);
        Lamp {
            gpio,
            name,
            channel,
            pwm,
            level: 0,
        }
    }

    /// Takes the lamp's level from `frame`. Traced only as it turns on or off, so a console
    /// fading it does not fill the log.
    fn follow(&mut self, frame: &Frame) {
        let level = self.channel.get(frame);
        if level == self.level {
            return;
        }
        if (level == 0) != (self.level == 0) {
            trace_lamp(self.gpio, self.name, level);
        }
        self.level = level;
        if let Some(pwm) = &self.pwm
            && let Err(e) = pwm.set_duty_cycle(f64::from(level) / f64::from(u8::MAX))
        {
            eprintln!("face: GPIO{} {}: {e}", self.gpio, self.name);
        }
    }
}

impl EstopContacts {
    fn read(&self) -> Estop {
        match (self.nc.level, self.no.level) {
            (Level::Low, Level::High) => Estop::Released,
            (Level::High, Level::Low) => Estop::Pressed,
            (Level::High, Level::High) => Estop::CableFault,
            (Level::Low, Level::Low) => Estop::ContactFault,
        }
    }

    /// Decides the e-stop from both contacts, traces it, publishes it, and returns whether
    /// the rig must be stopped.
    fn decide(&self, telemetry: &Publisher) -> bool {
        let estop = self.read();
        // Pressed or not, like the buttons, unless the wiring says it cannot tell.
        let (state, word) = match estop {
            Estop::Released => ("RELEASED", rig_mqtt::boolean(false)),
            Estop::Pressed => ("PRESSED", rig_mqtt::boolean(true)),
            Estop::CableFault => ("CABLE FAULT (treated as pressed)", "cable_fault"),
            Estop::ContactFault => ("CONTACT FAULT (treated as pressed)", "contact_fault"),
        };
        eprintln!("face: {:<18}{state}", "ESTOP");
        telemetry.publish("estop", word, true);
        !matches!(estop, Estop::Released)
    }
}

impl Input {
    fn new(pin: InputPin, name: &'static str, button: Option<&'static str>) -> Input {
        let level = pin.read();
        Input {
            pin,
            name,
            button,
            level,
            differing: 0,
        }
    }

    /// Takes one sample. Returns whether it settled a new level, which is then traced and,
    /// on a button, published.
    fn poll(&mut self, telemetry: &Publisher) -> bool {
        if self.pin.read() == self.level {
            self.differing = 0;
            return false;
        }
        self.differing += 1;
        if self.differing < SETTLE_SAMPLES {
            return false;
        }
        self.level = !self.level;
        self.differing = 0;
        self.trace();
        self.publish(telemetry);
        true
    }

    /// A button is pulled up and closes to ground, so pressed is LOW.
    fn pressed(&self) -> bool {
        self.level == Level::Low
    }

    fn publish(&self, telemetry: &Publisher) {
        if let Some(topic) = self.button {
            telemetry.publish(topic, rig_mqtt::boolean(self.pressed()), true);
        }
    }

    fn trace(&self) {
        let meaning = match (self.button, self.level) {
            (None, _) => "",
            (Some(_), Level::Low) => " (pressed)",
            (Some(_), Level::High) => " (released)",
        };
        trace_pin(self.pin.pin(), self.name, self.level, meaning);
    }
}

fn trace_pin(number: u8, name: &str, level: Level, meaning: &str) {
    // The level is padded to four so the annotations after it line up.
    let level = match level {
        Level::Low => "LOW ",
        Level::High => "HIGH",
    };
    eprintln!("face: GPIO{number:<2} {name:<11}{level}{meaning}");
}

fn trace_lamp(gpio: u8, name: &str, level: u8) {
    let meaning = if level == 0 { "dark" } else { "lit" };
    eprintln!("face: GPIO{gpio:<2} {name:<11}{level:<4}({meaning})");
}
