//! The face: the buttons on the cabinet door — an e-stop, the ball button and the music
//! button, with a lamp in each of the latter two.
//!
//! The e-stop is live: while it is pressed the show sends the rig black. The two buttons
//! are only traced, and the lamps are held dark.
//!
//! Inputs are polled rather than interrupt-driven, because there are no capacitors on the
//! lines: a contact bounces, and a level is only believed once it has held for a few
//! consecutive samples.
//!
//! The face is a fixture of its own on the broker, with its own client beside the brain's:
//! `health/face`, and `face/ball_button`, `face/music_button` and `face/estop`, retained.

use cortex::telemetry::Publisher;
use embassy_executor::Spawner;
use embassy_time::{Duration, Ticker};
use rppal::gpio::{Gpio, InputPin, Level, OutputPin};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

// BCM numbers. The RS-485 HAT owns 14, 15 and 18; none of those may appear here.
const ESTOP_NC: u8 = 23;
const ESTOP_NO: u8 = 24;
const BALL_BUTTON: u8 = 25;
const MUSIC_BUTTON: u8 = 27;
const BALL_LAMP: u8 = 12;
const MUSIC_LAMP: u8 = 13;

const POLL_PERIOD: Duration = Duration::from_millis(10);
/// Consecutive samples a new level must hold before it is accepted: 30 ms at the poll
/// period, longer than the contacts bounce and shorter than any deliberate press.
const SETTLE_SAMPLES: u8 = 3;

/// Whether the e-stop says stop, read by the show every frame. Starts pressed, so a frame
/// that somehow ran before the face was read would be black rather than lit.
static ESTOP_PRESSED: AtomicBool = AtomicBool::new(true);

/// The face's pins, claimed and configured, waiting for [`spawn`] to hand them to their
/// polling task.
pub struct Face {
    estop_nc: Input,
    estop_no: Input,
    ball_button: Input,
    music_button: Input,
    // Never written after the claim. Held so the pins stay driven LOW: rppal hands a pin
    // back to its previous mode when it is dropped.
    _ball_lamp: OutputPin,
    _music_lamp: OutputPin,
    telemetry: Publisher,
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

/// Claims the face's pins: the lamps driven LOW and the inputs pulled up, and the e-stop
/// read once. Call this before anything else, so the lamps are dark, no input floats, and
/// the very first frame already honours the e-stop.
///
/// Both buttons and the e-stop are published once here, so the tree holds the face's state
/// before anybody presses anything.
///
/// Panics if the GPIO cannot be opened. The brain only runs on the Pi, and a brain that
/// cannot read its e-stop must not light the rig.
pub fn claim(telemetry: Publisher) -> Face {
    let gpio = Gpio::new().unwrap_or_else(|e| panic!("face: gpio: {e}"));
    let pin = |number: u8| {
        gpio.get(number)
            .unwrap_or_else(|e| panic!("face: GPIO{number}: {e}"))
    };

    // rppal writes the level before switching the pin to output, so a lamp cannot flash on
    // while it is being claimed.
    let face = Face {
        _ball_lamp: pin(BALL_LAMP).into_output_low(),
        _music_lamp: pin(MUSIC_LAMP).into_output_low(),
        estop_nc: Input::new(pin(ESTOP_NC).into_input_pullup(), "estop-nc", None),
        estop_no: Input::new(pin(ESTOP_NO).into_input_pullup(), "estop-no", None),
        ball_button: Input::new(pin(BALL_BUTTON).into_input_pullup(), "ball", Some("ball_button")),
        music_button: Input::new(
            pin(MUSIC_BUTTON).into_input_pullup(),
            "music",
            Some("music_button"),
        ),
        telemetry,
    };

    for input in [&face.estop_nc, &face.estop_no, &face.ball_button, &face.music_button] {
        input.trace();
        input.publish(&face.telemetry);
    }
    trace_pin(BALL_LAMP, "ball-lamp", Level::Low, " (dark)");
    trace_pin(MUSIC_LAMP, "music-lamp", Level::Low, " (dark)");
    face.decide_estop();

    face
}

/// Starts polling the inputs.
pub fn spawn(spawner: Spawner, face: Face) {
    spawner.spawn(input_task(face).unwrap());
}

/// Whether the show must send the rig black: the e-stop is pressed, or its wiring is at
/// fault.
pub fn estop_pressed() -> bool {
    ESTOP_PRESSED.load(Ordering::Relaxed)
}

#[embassy_executor::task]
async fn input_task(mut face: Face) -> ! {
    let mut ticker = Ticker::every(POLL_PERIOD);
    // The e-stop's contacts do not move together: pressing opens NC before NO closes, and
    // releasing opens NO before NC closes, so for a moment either way both read open. Read off
    // the first contact to settle, that moment is a cable fault on every press. The e-stop is
    // therefore decided only once neither contact has read anything but its settled level for
    // a full settle period, and from both levels together.
    let mut estop_undecided = false;
    let mut estop_quiet = 0u8;
    loop {
        ticker.next().await;
        // `|`, not `||`: both contacts must be sampled every tick, or a change on the NO
        // contact would be debounced late whenever the NC contact changed too.
        let estop_changed =
            face.estop_nc.poll(&face.telemetry) | face.estop_no.poll(&face.telemetry);
        if estop_changed {
            // Towards stop at once, on whichever contact got there first. Only the decision
            // waits, and only it can release.
            if !matches!(face.estop(), Estop::Released) {
                ESTOP_PRESSED.store(true, Ordering::Relaxed);
            }
            estop_undecided = true;
            estop_quiet = 0;
        } else if face.estop_nc.differing == 0 && face.estop_no.differing == 0 {
            estop_quiet = estop_quiet.saturating_add(1);
        } else {
            estop_quiet = 0;
        }
        if estop_undecided && estop_quiet >= SETTLE_SAMPLES {
            estop_undecided = false;
            face.decide_estop();
        }

        face.ball_button.poll(&face.telemetry);
        face.music_button.poll(&face.telemetry);
    }
}

impl Face {
    fn estop(&self) -> Estop {
        match (self.estop_nc.level, self.estop_no.level) {
            (Level::Low, Level::High) => Estop::Released,
            (Level::High, Level::Low) => Estop::Pressed,
            (Level::High, Level::High) => Estop::CableFault,
            (Level::Low, Level::Low) => Estop::ContactFault,
        }
    }

    /// Hands the e-stop's state to the show, traces it, and publishes it.
    fn decide_estop(&self) {
        let estop = self.estop();
        ESTOP_PRESSED.store(!matches!(estop, Estop::Released), Ordering::Relaxed);
        // Pressed or not, like the buttons, unless the wiring says it cannot tell.
        let (state, word) = match estop {
            Estop::Released => ("RELEASED", rig_mqtt::boolean(false)),
            Estop::Pressed => ("PRESSED", rig_mqtt::boolean(true)),
            Estop::CableFault => ("CABLE FAULT (treated as pressed)", "cable_fault"),
            Estop::ContactFault => ("CONTACT FAULT (treated as pressed)", "contact_fault"),
        };
        eprintln!("face: {}  {:<18}{state}", timestamp(), "ESTOP");
        self.telemetry.publish("estop", word, true);
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
    fn publish(&self, telemetry: &Publisher) {
        if let Some(topic) = self.button {
            telemetry.publish(topic, rig_mqtt::boolean(self.level == Level::Low), true);
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
    eprintln!(
        "face: {}  GPIO{number:<2} {name:<11}{level}{meaning}",
        timestamp()
    );
}

/// Local wall-clock time to the millisecond, so a trace line can be matched against a
/// press someone remembers making.
fn timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = now.as_secs() as libc::time_t;
    // `localtime_r`, not `localtime`: `localtime` returns a buffer shared by every caller
    // in the process.
    //
    // SAFETY: `tm` is plain data, so all zeroes is a valid value, and `localtime_r` writes
    // only into the struct it is handed.
    let mut local: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&seconds, &mut local) };
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        local.tm_hour,
        local.tm_min,
        local.tm_sec,
        now.subsec_millis()
    )
}
