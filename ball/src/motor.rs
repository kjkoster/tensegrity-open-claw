use esp_hal::{
    gpio::{Level, Output, OutputConfig},
    peripherals::GPIO7,
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, watch::Watch};
use embassy_time::{Duration, Instant};
use rtt_target::rprintln;

use crate::telemetry;

/// The motor's pin (board label D5).
pub type MotorPin = GPIO7<'static>;

/// The longest the motor runs in one go, whatever the rig asks for. The rig decides how long
/// a run is; this is the bound on a rig that has stopped deciding and holds the channel on.
pub const MAX_RUN: Duration = Duration::from_secs(2);

/// Whether the motor is running, for the sensing that only happens around a run. Written
/// here and nowhere else, so the motor stays owned by the one loop that drives it.
pub static RUNNING: Watch<CriticalSectionRawMutex, bool, 1> = Watch::new();

#[derive(Clone, Copy, PartialEq)]
enum State {
    /// Off, and free to start when the rig asks.
    Idle,
    Running { since: Instant },
    /// Off, and kept off until the rig asks for a stop. A run that was cut short must not
    /// pick up again where the universe happens to come back: what returns after a drop is
    /// the tail of a run, and the tail of a run is a partial one. Only being told to stop
    /// leaves it, because that is the rig saying the run it wanted is over.
    Tripped,
}

impl State {
    /// The state as it is published on `ball/motor` and logged.
    fn word(self) -> &'static str {
        match self {
            State::Idle => "idle",
            State::Running { .. } => "running",
            State::Tripped => "tripped",
        }
    }
}

pub struct Motor {
    pin: Output<'static>,
    state: State,
}

impl Motor {
    /// Starts with the motor off and tripped: a board that restarted mid-run comes back
    /// into the same tail of a run as one that lost the universe.
    pub fn new(pin: MotorPin) -> Self {
        let motor = Self {
            pin: Output::new(pin, Level::Low, OutputConfig::default()),
            state: State::Tripped,
        };
        RUNNING.sender().send(false);
        telemetry::publish("motor", motor.state.word());
        motor
    }

    /// When the run in progress has to end, while there is one.
    pub fn deadline(&self) -> Option<Instant> {
        match self.state {
            State::Running { since } => Some(since + MAX_RUN),
            State::Idle | State::Tripped => None,
        }
    }

    /// Does what the rig asks, unless tripped and not yet told to stop.
    pub fn follow(&mut self, running: bool) {
        let next = match (self.state, running) {
            (_, false) => State::Idle,
            (State::Idle, true) => State::Running {
                since: Instant::now(),
            },
            (state @ (State::Running { .. } | State::Tripped), true) => state,
        };
        self.enter(next);
    }

    /// Stops the motor and keeps it stopped until the rig next asks for a stop.
    pub fn trip(&mut self) {
        self.enter(State::Tripped);
    }

    fn enter(&mut self, next: State) {
        let running = matches!(next, State::Running { .. });
        // Logged on the change alone: this is called for every change on any of the ball's
        // channels, and the motor's own state moves on few of them.
        if next != self.state {
            self.state = next;
            rprintln!("motor:          {}", next.word());
            telemetry::publish("motor", next.word());
            // Only when running starts or stops: idle and tripped are both off, and a move
            // between them must not restart the watch on a run that already ended.
            RUNNING.sender().send_if_modified(|current| {
                let changed = *current != Some(running);
                *current = Some(running);
                changed
            });
        }
        // Active-high: the motor runs while the pin is high. This is the only place that
        // knows about it.
        self.pin.set_level(if running { Level::High } else { Level::Low });
    }
}
