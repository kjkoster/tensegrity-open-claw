use core::f32::consts::TAU;

use embassy_time::{Duration, Ticker};
use esp_hal::{
    gpio::DriveMode,
    ledc::{
        LSGlobalClkSource, Ledc, LowSpeed,
        channel::{self, Channel, ChannelHW, ChannelIFace},
        timer::{self, Timer, TimerIFace, config::Duty},
    },
    peripherals::{GPIO21, LEDC},
    time::Rate,
};
use static_cell::StaticCell;

/// The status LED's pin (board label D6). It doubles as the chip's default UART0 TX, so
/// nothing in this firmware may log over UART0: every byte would show on the LED.
pub type StatusLedPin = GPIO21<'static>;

const FADE_FREQUENCY_HZ: f32 = 3.0;
const UPDATE_RATE_HZ: u64 = 200;

// 5 kHz at 13 bits is the finest resolution the 80 MHz APB clock divides down to at this
// frequency; both are far beyond what the eye resolves as flicker or steps.
const PWM_FREQUENCY: Rate = Rate::from_khz(5);
const PWM_RESOLUTION: Duty = Duty::Duty13Bit;
const MAX_DUTY: u32 = 1 << PWM_RESOLUTION as u32;

// The channel borrows its timer for as long as it lives, and it lives in a task.
static PWM_TIMER: StaticCell<Timer<'static, LowSpeed>> = StaticCell::new();

pub struct StatusLed {
    channel: Channel<'static, LowSpeed>,
}

impl StatusLed {
    pub fn new(ledc: LEDC<'static>, pin: StatusLedPin) -> Self {
        let mut ledc = Ledc::new(ledc);
        ledc.set_global_slow_clock(LSGlobalClkSource::APBClk);

        let pwm_timer = PWM_TIMER.init(ledc.timer::<LowSpeed>(timer::Number::Timer0));
        pwm_timer
            .configure(timer::config::Config {
                duty: PWM_RESOLUTION,
                clock_source: timer::LSClockSource::APBClk,
                frequency: PWM_FREQUENCY,
            })
            .unwrap();

        let mut channel = ledc.channel(channel::Number::Channel0, pin);
        channel
            .configure(channel::config::Config {
                timer: pwm_timer,
                // Pin held high, which is off for this LED; see `set_brightness`.
                duty_pct: 100,
                drive_mode: DriveMode::PushPull,
            })
            .unwrap();

        Self { channel }
    }

    /// 0 is fully off, 255 is fully on, linear in between.
    pub fn set_brightness(&self, brightness: u8) {
        // The LED is wired from 3.3 V to the pin, so it is active-low: it lights while the
        // pin is low. The duty cycle is the fraction of time the pin is high, hence the
        // inversion. This is the only place that knows about it.
        let duty = MAX_DUTY - u32::from(brightness) * MAX_DUTY / u32::from(u8::MAX);
        // Not `set_duty`: that takes whole percents, which fade in visible steps.
        self.channel.set_duty_hw(duty);
    }
}

/// Brightness follows (1 + sin(2π · f · t)) / 2.
#[embassy_executor::task]
pub async fn sine_fade(led: StatusLed) -> ! {
    let mut ticker = Ticker::every(Duration::from_hz(UPDATE_RATE_HZ));
    let phase_step = TAU * FADE_FREQUENCY_HZ / UPDATE_RATE_HZ as f32;
    let mut phase = 0.0;
    loop {
        let level = (1.0 + libm::sinf(phase)) / 2.0;
        led.set_brightness(libm::roundf(level * f32::from(u8::MAX)) as u8);

        phase += phase_step;
        // Kept within one turn so the phase never grows past f32's precision.
        if phase >= TAU {
            phase -= TAU;
        }
        ticker.next().await;
    }
}
