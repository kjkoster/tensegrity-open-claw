use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embedded_hal::pwm::SetDutyCycle;

use crate::models::{DMX_MAXVALUE, DmxValue};

pub async fn run(
    dmx_value: &Signal<CriticalSectionRawMutex, DmxValue>,
    onboard: &mut impl SetDutyCycle, // active-low
    red: &mut impl SetDutyCycle,
    green: &mut impl SetDutyCycle,
    blue: &mut impl SetDutyCycle,
    white: &mut impl SetDutyCycle,
) -> ! {
    let max = onboard.max_duty_cycle();
    loop {
        let val = dmx_value.wait().await;
        let intensity = val.intensity();
        let duty = |v: u8| -> u16 { (v as u32 * max as u32 / DMX_MAXVALUE as u32) as u16 };
        let dimmed = |c: u8| -> u16 { duty((c as u32 * intensity as u32 / DMX_MAXVALUE as u32) as u8) };
        onboard.set_duty_cycle(max - duty(intensity)).ok();
        red.set_duty_cycle(dimmed(val.red())).ok();
        green.set_duty_cycle(dimmed(val.green())).ok();
        blue.set_duty_cycle(dimmed(val.blue())).ok();
        white.set_duty_cycle(dimmed(val.white())).ok();
    }
}
