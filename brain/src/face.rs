//! The face: the buttons on the cabinet door — an e-stop, the ball button and the music
//! button, with a lamp in each of the latter two.
//!
//! The e-stop is live: while it is pressed the rig is sent black, whoever is driving it.
//! The two buttons are only traced for now, and the lamps are held dark.
//!
//! Inputs are polled rather than interrupt-driven, because there are no capacitors on the
//! lines: a contact bounces, and a level is only believed once it has held for a few
//! consecutive samples.

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

/// Whether the e-stop says stop, read by the frame loop every frame. Starts pressed, so a
/// frame that somehow ran before the face was read would be black rather than lit.
static ESTOP_PRESSED: AtomicBool = AtomicBool::new(true);

/// The face's pins, claimed and configured, waiting for [`spawn`] to hand them to their
/// polling task.
pub struct Face {
    estop_nc: Input,
    estop_no: Input,
    ball: Input,
    music: Input,
    // Never written after the claim. Held so the pins stay driven LOW: rppal hands a pin
    // back to its previous mode when it is dropped.
    _ball_lamp: OutputPin,
    _music_lamp: OutputPin,
}

struct Input {
    pin: InputPin,
    name: &'static str,
    /// A momentary button is annotated pressed/released in the trace. The e-stop contacts
    /// are not: what a level means depends on the other contact, and the derived e-stop
    /// line says it.
    momentary: bool,
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
/// Panics if the GPIO cannot be opened. The brain only runs on the Pi, and a brain that
/// cannot read its e-stop must not light the rig.
pub fn claim() -> Face {
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
        estop_nc: Input::new(pin(ESTOP_NC).into_input_pullup(), "estop-nc", false),
        estop_no: Input::new(pin(ESTOP_NO).into_input_pullup(), "estop-no", false),
        ball: Input::new(pin(BALL_BUTTON).into_input_pullup(), "ball", true),
        music: Input::new(pin(MUSIC_BUTTON).into_input_pullup(), "music", true),
    };

    for input in [&face.estop_nc, &face.estop_no, &face.ball, &face.music] {
        input.trace();
    }
    trace_lamp(BALL_LAMP, "ball-lamp");
    trace_lamp(MUSIC_LAMP, "music-lamp");
    face.update_estop();

    face
}

/// Starts polling the inputs.
pub fn spawn(spawner: Spawner, face: Face) {
    spawner.spawn(input_task(face).unwrap());
}

/// Whether the rig must be black: the e-stop is pressed, or its wiring is at fault.
pub fn estop_pressed() -> bool {
    ESTOP_PRESSED.load(Ordering::Relaxed)
}

#[embassy_executor::task]
async fn input_task(mut face: Face) -> ! {
    let mut ticker = Ticker::every(POLL_PERIOD);
    loop {
        ticker.next().await;
        // `|`, not `||`: both contacts must be sampled every tick, or a change on the NO
        // contact would be debounced late whenever the NC contact changed too.
        let estop_changed = face.estop_nc.poll() | face.estop_no.poll();
        if estop_changed {
            face.update_estop();
        }
        face.ball.poll();
        face.music.poll();
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

    /// Hands the e-stop's state to the frame loop, and traces it.
    fn update_estop(&self) {
        let estop = self.estop();
        ESTOP_PRESSED.store(!matches!(estop, Estop::Released), Ordering::Relaxed);
        let state = match estop {
            Estop::Released => "RELEASED",
            Estop::Pressed => "PRESSED",
            Estop::CableFault => "CABLE FAULT (treated as pressed)",
            Estop::ContactFault => "CONTACT FAULT (treated as pressed)",
        };
        eprintln!("face: {}  {:<18}{state}", timestamp(), "ESTOP");
    }
}

impl Input {
    fn new(pin: InputPin, name: &'static str, momentary: bool) -> Input {
        let level = pin.read();
        Input {
            pin,
            name,
            momentary,
            level,
            differing: 0,
        }
    }

    /// Takes one sample. Returns whether it settled a new level, which is then traced.
    fn poll(&mut self) -> bool {
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
        true
    }

    fn trace(&self) {
        let meaning = match (self.momentary, self.level) {
            (false, _) => "",
            (true, Level::Low) => " (pressed)",
            (true, Level::High) => " (released)",
        };
        eprintln!(
            "face: {}  GPIO{:<2} {:<11}{}{meaning}",
            timestamp(),
            self.pin.pin(),
            self.name,
            level_name(self.level),
        );
    }
}

fn trace_lamp(number: u8, name: &str) {
    eprintln!(
        "face: {}  GPIO{number:<2} {name:<11}{} (dark)",
        timestamp(),
        level_name(Level::Low),
    );
}

/// Upper case, and padded so the annotations after it line up.
fn level_name(level: Level) -> &'static str {
    match level {
        Level::Low => "LOW ",
        Level::High => "HIGH",
    }
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
