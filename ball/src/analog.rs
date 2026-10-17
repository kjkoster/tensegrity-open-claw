//! The two analog inputs: the pack voltage, and whether the C terminal is live.
//!
//! Each is read only when the reading is about to be published, and no more often. The pack
//! drains over hours, so once a minute says everything about it. The C terminal is only
//! interesting around a run: it is watched every 100 ms while the motor runs and for half a
//! second after it stops, and not read at all otherwise.
//!
//! The conversion is a straight line from the raw count, with no calibration curve. Each
//! pack reading logs its raw count and pin voltage beside the result, as does every change of
//! the C terminal, so the divider constants below can be set from a meter: the divider is the
//! meter's reading over the logged pin voltage.

use alloc::format;
use embassy_time::{Duration, Instant, with_deadline};
use esp_hal::{
    Async,
    analog::adc::{Adc, AdcCalBasic, AdcConfig, AdcPin, Attenuation},
    peripherals::{ADC1, GPIO3, GPIO4},
};
use rtt_target::rprintln;

use crate::{motor, telemetry};

/// The pack voltage, through its divider (board label D1).
pub type PackPin = GPIO3<'static>;
/// The C terminal, through its divider (board label D2).
pub type PresencePin = GPIO4<'static>;

const PACK_PERIOD: Duration = Duration::from_secs(60);
const PRESENCE_PERIOD: Duration = Duration::from_millis(100);
/// How long the C terminal is still watched after the motor stops, for the line coming down.
const PRESENCE_LINGER: Duration = Duration::from_millis(500);
/// Samples averaged into one reading.
const SAMPLES: u32 = 8;
/// The largest raw count, at the attenuation's full scale.
const ADC_MAX: f32 = 4095.0;

// A full pack of about 21 V puts about 1.9 V on the pin, inside the 11 dB range.
const PACK_ATTENUATION: Attenuation = Attenuation::_11dB;
const PACK_FULL_SCALE_V: f32 = 2.5;
/// Pack voltage over logged pin voltage. Nominally 11.0 for 100 kΩ over 10 kΩ; this is
/// measured instead — a metered 19.78 V pack logged a 1.587 V pin — so it also absorbs the
/// ADC's own gain error, and the logged pin voltage is not what a meter on the pin reads.
const PACK_DIVIDER: f32 = 19.78 / 1.587;

// The line is 3.3–4.0 V while the grinder runs, 0.76–0.92 V on the pin, inside the
// 2.5 dB range.
const PRESENCE_ATTENUATION: Attenuation = Attenuation::_2p5dB;
const PRESENCE_FULL_SCALE_V: f32 = 1.05;
/// Line voltage over pin voltage: 1 MΩ over 300 kΩ.
const PRESENCE_DIVIDER: f32 = 1300.0 / 300.0;
/// Pin voltage above which the C terminal counts as present: well clear of an idle 0 V
/// and of a running line's lowest 0.76 V.
const PRESENCE_THRESHOLD_V: f32 = 0.3;

/// Not a calibration curve: on this chip the basic scheme only loads the ADC's zero-offset
/// code from the chip's efuses and hands back the raw count unchanged. Without it the code
/// is left at 0 and every reading sits far too high — the pack read a saturated 4095 and
/// an idle C terminal 0.39 V.
type Cal = AdcCalBasic<ADC1<'static>>;

/// The ADC and the two pins it reads. Both are on ADC1, which works with Wi-Fi active.
pub struct Sensors {
    adc: Adc<'static, ADC1<'static>, Async>,
    pack: AdcPin<PackPin, ADC1<'static>, Cal>,
    presence: AdcPin<PresencePin, ADC1<'static>, Cal>,
}

impl Sensors {
    pub fn new(adc1: ADC1<'static>, pack: PackPin, presence: PresencePin) -> Self {
        let mut config = AdcConfig::new();
        let pack = config.enable_pin_with_cal::<_, Cal>(pack, PACK_ATTENUATION);
        let presence = config.enable_pin_with_cal::<_, Cal>(presence, PRESENCE_ATTENUATION);
        Self {
            adc: Adc::new(adc1, config).into_async(),
            pack,
            presence,
        }
    }
}

/// Whether the C terminal is being read, which follows the motor.
#[derive(Clone, Copy)]
enum Watching {
    Off,
    Running,
    Stopping { until: Instant },
}

#[embassy_executor::task]
pub async fn sense(mut sensors: Sensors) -> ! {
    let mut motor = motor::RUNNING.receiver().unwrap();
    let mut watching = Watching::Off;
    let mut next_pack = Instant::now();
    let mut next_presence = Instant::now();
    // The last C terminal state, so only a change is logged: ten lines a second would bury
    // everything else on the probe.
    let mut present = None;
    loop {
        let next = match watching {
            Watching::Off => next_pack,
            Watching::Running | Watching::Stopping { .. } => next_pack.min(next_presence),
        };
        if let Ok(running) = with_deadline(next, motor.changed()).await {
            watching = match (running, watching) {
                (true, Watching::Off) => {
                    next_presence = Instant::now();
                    Watching::Running
                }
                (true, _) => Watching::Running,
                (false, Watching::Off) => Watching::Off,
                (false, _) => Watching::Stopping {
                    until: Instant::now() + PRESENCE_LINGER,
                },
            };
            continue;
        }

        let now = Instant::now();
        if now >= next_pack {
            next_pack += PACK_PERIOD;
            sensors.read_pack().await;
        }
        if !matches!(watching, Watching::Off) && now >= next_presence {
            next_presence += PRESENCE_PERIOD;
            sensors.read_presence(&mut present).await;
            if let Watching::Stopping { until } = watching
                && now >= until
            {
                watching = Watching::Off;
            }
        }
    }
}

impl Sensors {
    async fn read_pack(&mut self) {
        let raw = average(&mut self.adc, &mut self.pack).await;
        let pin_v = volts(raw, PACK_FULL_SCALE_V);
        let pack_v = pin_v * PACK_DIVIDER;
        rprintln!("pack {:.2} V (raw {}, pin {:.3} V)", pack_v, raw, pin_v);
        telemetry::publish("pack_v", format!("{:.2}", pack_v));
        // Beside the pack voltage, so that a falling pack can be read against how long the
        // ball has been running on it.
        telemetry::publish("uptime_s", format!("{}", Instant::now().as_secs()));
    }

    async fn read_presence(&mut self, last: &mut Option<bool>) {
        let raw = average(&mut self.adc, &mut self.presence).await;
        let pin_v = volts(raw, PRESENCE_FULL_SCALE_V);
        let present = pin_v > PRESENCE_THRESHOLD_V;
        if *last != Some(present) {
            *last = Some(present);
            rprintln!(
                "c terminal {} (raw {}, pin {:.3} V, line {:.2} V)",
                if present { "present" } else { "absent" },
                raw,
                pin_v,
                pin_v * PRESENCE_DIVIDER
            );
        }
        telemetry::publish("c_terminal", rig_mqtt::boolean(present));
    }
}

async fn average<PIN>(
    adc: &mut Adc<'static, ADC1<'static>, Async>,
    pin: &mut AdcPin<PIN, ADC1<'static>, Cal>,
) -> u16
where
    PIN: esp_hal::analog::adc::AdcChannel,
{
    let mut sum = 0u32;
    for _ in 0..SAMPLES {
        sum += u32::from(adc.read_oneshot(pin).await);
    }
    ((sum + SAMPLES / 2) / SAMPLES) as u16
}

fn volts(raw: u16, full_scale_v: f32) -> f32 {
    f32::from(raw) * full_scale_v / ADC_MAX
}
