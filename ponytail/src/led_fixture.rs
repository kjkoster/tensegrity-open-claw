use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embedded_hal::pwm::SetDutyCycle;

use crate::models::DmxValue;

pub async fn run(
    dmx_value: &Signal<CriticalSectionRawMutex, DmxValue>,
    onboard: &mut impl SetDutyCycle, // active-low
    red: &mut impl SetDutyCycle,
    green: &mut impl SetDutyCycle,
    blue: &mut impl SetDutyCycle,
    white: &mut impl SetDutyCycle,
) -> ! {
    let max = onboard.max_duty_cycle();
    let duty = |val: u8| -> u16 { (val as u32 * max as u32 / 255) as u16 };
    loop {
        let val = dmx_value.wait().await;
        onboard.set_duty_cycle(max - duty(val.intensity())).ok();
        red.set_duty_cycle(duty(val.red())).ok();
        green.set_duty_cycle(duty(val.green())).ok();
        blue.set_duty_cycle(duty(val.blue())).ok();
        white.set_duty_cycle(duty(val.white())).ok();
    }
}
