//! Deep sleep outside shows. The rig asks over DMX, and the ball sleeps in five-minute
//! stretches, waking each time to report and to ask again.
//!
//! Deep sleep ends in a reset, so a wake is a boot like any other, and every boot decides the
//! same way: [`LISTEN`] after the network is up, it sleeps again if the universe asks it to or
//! has not been heard at all. A universe nobody sends means the rig is off, and a ball that
//! stayed awake for it would drain its pack exactly when nobody is there. Once the ball is
//! running, only the universe asking puts it to sleep.

use embassy_time::Duration;
use esp_hal::{
    peripherals::LPWR,
    rtc_cntl::{
        SocResetReason,
        sleep::{LowPower, RtcSleepConfig},
    },
    system::reset_reason,
    time,
};
use rtt_target::rprintln;

use crate::{
    config::WIFI_RETRY_SILENCE,
    led::Led,
    models::Look,
    motor::Motor,
    pattern, telemetry, wifi,
};

/// How long the ball listens once the network is up before it decides: longer than the
/// universe's data-loss timeout, and plenty for the broker to take the wake's telemetry.
pub const LISTEN: Duration = Duration::from_secs(5);

/// How long the ball tries to join before it decides that there is no access point to join.
/// The access point is on the Pi, so a rig switched off has none. Long enough for four
/// retries, each after a silence, which an access point that still lists the ball from before
/// a crash needs before it lets it back in; and a margin for the attempts themselves and the
/// lease.
pub const JOIN_TIMEOUT: Duration =
    Duration::from_secs(RETRIES * WIFI_RETRY_SILENCE.as_secs() + JOIN_MARGIN_S);
const RETRIES: u64 = 4;
const JOIN_MARGIN_S: u64 = 15;

/// One stretch of sleep.
const STRETCH: time::Duration = time::Duration::from_secs(300);

/// Wakes from deep sleep in a row, kept in RTC memory, which a deep sleep keeps and anything
/// else may not.
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut WAKES: u32 = 0;

/// Counts this boot as one more wake after a deep sleep, or starts the count over after any
/// other reset, and publishes it as `ball/wakes`.
pub fn count_wake() {
    let woke = reset_reason() == Some(SocResetReason::CoreDeepSleep);
    // SAFETY: called once, at boot, before any task runs, so nothing else touches it.
    let wakes = unsafe {
        WAKES = if woke { WAKES.wrapping_add(1) } else { 0 };
        WAKES
    };
    telemetry::publish("wakes", alloc::format!("{}", wakes));
}

pub struct Sleep {
    low_power: LowPower<'static>,
}

impl Sleep {
    pub fn new(lpwr: LPWR<'static>) -> Self {
        Self {
            low_power: LowPower::new(lpwr),
        }
    }

    /// Puts the ball to sleep for one stretch. The motor goes off first, whatever it was
    /// doing, and held off: in deep sleep nothing drives its pin.
    pub async fn now(&mut self, motor: &mut Motor, status: &Led) -> ! {
        motor.hold_off();
        status.set_brightness(0);
        pattern::LOOK.sender().send(Look::DARK);
        telemetry::farewell().await;
        wifi::disconnect().await;
        rprintln!("sleeping for {} s", STRETCH.as_secs());
        self.low_power.set_wakeup_deadline(time::Instant::now() + STRETCH);
        self.low_power.sleep_deep(RtcSleepConfig::deep())
    }
}
