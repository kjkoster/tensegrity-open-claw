use esp_hal::{
    gpio::DriveMode,
    ledc::{
        LSGlobalClkSource, Ledc, LowSpeed,
        channel::{self, Channel, ChannelHW, ChannelIFace},
        timer::{self, Timer, TimerIFace, config::Duty},
    },
    peripherals::{GPIO10, GPIO20, GPIO21, LEDC},
    time::Rate,
};
use static_cell::StaticCell;

/// The status LED's pin (board label D6). It doubles as the chip's default UART0 TX, so
/// nothing in this firmware may log over UART0: every byte would show on the LED.
pub type StatusLedPin = GPIO21<'static>;
/// The CTRL pin of the white power LED's AL8860 driver (board label D7).
pub type WhiteLedPin = GPIO20<'static>;
/// The CTRL pin of the blue power LED's AL8860 driver (board label D10).
pub type BlueLedPin = GPIO10<'static>;

// 5 kHz at 13 bits is the finest resolution the 80 MHz APB clock divides down to at this
// frequency; both are far beyond what the eye resolves as flicker or steps.
const STATUS_PWM_FREQUENCY: Rate = Rate::from_khz(5);
const STATUS_PWM_RESOLUTION: Duty = Duty::Duty13Bit;

// The AL8860 drivers dim from a PWM on their CTRL pins. 13 bits, not the 14 the timer
// offers: at its widest resolution the LEDC cannot hold a channel high for a whole period,
// and full duty wraps around to off — 255 came out dark.
const POWER_PWM_FREQUENCY: Rate = Rate::from_hz(500);
const POWER_PWM_RESOLUTION: Duty = Duty::Duty13Bit;

// A channel borrows its timer for as long as it lives, and it lives in a task.
static STATUS_PWM_TIMER: StaticCell<Timer<'static, LowSpeed>> = StaticCell::new();
static POWER_PWM_TIMER: StaticCell<Timer<'static, LowSpeed>> = StaticCell::new();

/// Which pin level lights the LED.
#[derive(Clone, Copy)]
enum Polarity {
    /// Wired from 3.3 V to the pin: lit while the pin is low.
    ActiveLow,
    /// Lit while the pin is high.
    ActiveHigh,
}

/// An LED dimmed by PWM.
pub struct Led {
    channel: Channel<'static, LowSpeed>,
    polarity: Polarity,
    max_duty: u32,
}

/// Every LED on the board. They share one LEDC peripheral, which can only be taken once, so
/// they are set up together.
pub struct Leds {
    pub status: Led,
    pub white: Led,
    pub blue: Led,
}

impl Leds {
    /// Sets up every LED dark.
    pub fn new(
        ledc: LEDC<'static>,
        status: StatusLedPin,
        white: WhiteLedPin,
        blue: BlueLedPin,
    ) -> Self {
        let mut ledc = Ledc::new(ledc);
        ledc.set_global_slow_clock(LSGlobalClkSource::APBClk);

        let status_timer = STATUS_PWM_TIMER.init(ledc.timer::<LowSpeed>(timer::Number::Timer0));
        configure_timer(status_timer, STATUS_PWM_FREQUENCY, STATUS_PWM_RESOLUTION);
        let power_timer = POWER_PWM_TIMER.init(ledc.timer::<LowSpeed>(timer::Number::Timer1));
        configure_timer(power_timer, POWER_PWM_FREQUENCY, POWER_PWM_RESOLUTION);
        // Shared from here on: both power LEDs run off the one timer.
        let power_timer: &'static Timer<'static, LowSpeed> = power_timer;

        Self {
            status: Led::new(
                ledc.channel(channel::Number::Channel0, status),
                status_timer,
                STATUS_PWM_RESOLUTION,
                Polarity::ActiveLow,
            ),
            white: Led::new(
                ledc.channel(channel::Number::Channel1, white),
                power_timer,
                POWER_PWM_RESOLUTION,
                Polarity::ActiveHigh,
            ),
            blue: Led::new(
                ledc.channel(channel::Number::Channel2, blue),
                power_timer,
                POWER_PWM_RESOLUTION,
                Polarity::ActiveHigh,
            ),
        }
    }
}

fn configure_timer(pwm_timer: &mut Timer<'static, LowSpeed>, frequency: Rate, duty: Duty) {
    pwm_timer
        .configure(timer::config::Config {
            duty,
            clock_source: timer::LSClockSource::APBClk,
            frequency,
        })
        .unwrap();
}

impl Led {
    fn new(
        mut channel: Channel<'static, LowSpeed>,
        pwm_timer: &'static Timer<'static, LowSpeed>,
        resolution: Duty,
        polarity: Polarity,
    ) -> Self {
        let dark_pct = match polarity {
            Polarity::ActiveLow => 100,
            Polarity::ActiveHigh => 0,
        };
        channel
            .configure(channel::config::Config {
                timer: pwm_timer,
                duty_pct: dark_pct,
                drive_mode: DriveMode::PushPull,
            })
            .unwrap();

        Self {
            channel,
            polarity,
            max_duty: 1 << resolution as u32,
        }
    }

    /// 0 is fully off, 255 is fully on, linear in between.
    pub fn set_brightness(&self, brightness: u8) {
        // The duty cycle is the fraction of time the pin is high, so an active-low LED
        // takes the inversion. This is the only place that knows about it.
        let lit = u32::from(brightness) * self.max_duty / u32::from(u8::MAX);
        let duty = match self.polarity {
            Polarity::ActiveLow => self.max_duty - lit,
            Polarity::ActiveHigh => lit,
        };
        // Not `set_duty`: that takes whole percents, which fade in visible steps.
        self.channel.set_duty_hw(duty);
    }
}
