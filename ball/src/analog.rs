//! A hardware test for the two analog inputs: the pack voltage and whether the C terminal
//! is live, each logged once a second.
//!
//! The conversion is a straight line from the raw count, with no calibration curve. Each
//! reading logs its raw count and pin voltage beside the result, so the divider constants
//! below can be set from a meter: the divider is the meter's reading over the logged pin
//! voltage.

use embassy_time::{Duration, Ticker};
use esp_hal::{
    Async,
    analog::adc::{Adc, AdcCalBasic, AdcConfig, AdcPin, Attenuation},
    peripherals::{ADC1, GPIO3, GPIO4},
};
use rtt_target::rprintln;

/// The pack voltage, through its divider (board label D1).
pub type PackPin = GPIO3<'static>;
/// The C terminal, through its divider (board label D2).
pub type PresencePin = GPIO4<'static>;

const PERIOD: Duration = Duration::from_secs(1);
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

#[embassy_executor::task]
pub async fn sense(mut sensors: Sensors) -> ! {
    let mut ticker = Ticker::every(PERIOD);
    loop {
        ticker.next().await;

        let raw = average(&mut sensors.adc, &mut sensors.pack).await;
        let pin_v = volts(raw, PACK_FULL_SCALE_V);
        rprintln!(
            "pack {:.2} V (raw {}, pin {:.3} V)",
            pin_v * PACK_DIVIDER,
            raw,
            pin_v
        );

        let raw = average(&mut sensors.adc, &mut sensors.presence).await;
        let pin_v = volts(raw, PRESENCE_FULL_SCALE_V);
        let state = if pin_v > PRESENCE_THRESHOLD_V {
            "present"
        } else {
            "absent"
        };
        rprintln!(
            "c terminal {} (raw {}, pin {:.3} V, line {:.2} V)",
            state,
            raw,
            pin_v,
            pin_v * PRESENCE_DIVIDER
        );
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
