use esp_hal::{
    gpio::{Level, Output, OutputConfig},
    peripherals::GPIO7,
};
use embassy_time::{Duration, Instant};
use rtt_target::rprintln;

/// The motor's pin (board label D5).
pub type MotorPin = GPIO7<'static>;

/// The longest the motor runs in one go, whatever the rig asks for. The rig decides how long
/// a run is; this is the bound on a rig that has stopped deciding and holds the channel on.
pub const MAX_RUN: Duration = Duration::from_secs(120);

pub struct Motor {
    pin: Output<'static>,
    /// Set while the motor may not start. A run that was cut short must not pick up again
    /// where the universe happens to come back: what returns after a drop is the tail of a
    /// run, and the tail of a run is a partial one. Only being told to stop clears it,
    /// because that is the rig saying the run it wanted is over.
    tripped: bool,
    /// When the run in progress began, while there is one.
    running_since: Option<Instant>,
}

impl Motor {
    /// Starts with the motor off and tripped: a board that restarted mid-run comes back
    /// into the same tail of a run as one that lost the universe.
    pub fn new(pin: MotorPin) -> Self {
        Self {
            pin: Output::new(pin, Level::Low, OutputConfig::default()),
            tripped: true,
            running_since: None,
        }
    }

    /// When the run in progress has to end, while there is one.
    pub fn deadline(&self) -> Option<Instant> {
        self.running_since.map(|since| since + MAX_RUN)
    }

    /// Does what the rig asks, unless tripped and not yet told to stop.
    pub fn follow(&mut self, running: bool) {
        if !running {
            self.tripped = false;
        }
        self.drive(running && !self.tripped);
    }

    /// Stops the motor and keeps it stopped until the rig next asks for a stop.
    pub fn trip(&mut self) {
        self.tripped = true;
        self.drive(false);
    }

    fn drive(&mut self, running: bool) {
        // Logged on the change alone: this is called for every change on any of the ball's
        // channels, and the motor's own state moves on few of them.
        if running != self.running_since.is_some() {
            if running {
                self.running_since = Some(Instant::now());
                rprintln!("motor:          on");
            } else {
                self.running_since = None;
                rprintln!("motor:          off");
            }
        }
        // Active-high: the motor runs while the pin is high. This is the only place that
        // knows about it.
        self.pin.set_level(if running { Level::High } else { Level::Low });
    }
}
