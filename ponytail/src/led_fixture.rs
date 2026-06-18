use embedded_hal::pwm::SetDutyCycle;
use rtt_target::rprintln;

use crate::models::{DMX_MAXVALUE, DmxReceiver};

pub async fn run(
    mut dmx_value: DmxReceiver,
    onboard: &mut impl SetDutyCycle, // active-low
    red: &mut impl SetDutyCycle,
    green: &mut impl SetDutyCycle,
    blue: &mut impl SetDutyCycle,
    white: &mut impl SetDutyCycle,
) -> ! {
    let max = onboard.max_duty_cycle();
    loop {
        let val = dmx_value.changed().await;
        let intensity = val.intensity();
        let duty = |v: u8| -> u16 { (v as u32 * max as u32 / DMX_MAXVALUE as u32) as u16 };
        let dimmed = |c: u8| -> u16 { duty((c as u32 * intensity as u32 / DMX_MAXVALUE as u32) as u8) };
        if let Err(e) = onboard.set_duty_cycle(max - duty(intensity)) {
            rprintln!("onboard set_duty_cycle error: {:?}", e);
        }
        if let Err(e) = red.set_duty_cycle(dimmed(val.red())) {
            rprintln!("red set_duty_cycle error: {:?}", e);
        }
        if let Err(e) = green.set_duty_cycle(dimmed(val.green())) {
            rprintln!("green set_duty_cycle error: {:?}", e);
        }
        if let Err(e) = blue.set_duty_cycle(dimmed(val.blue())) {
            rprintln!("blue set_duty_cycle error: {:?}", e);
        }
        if let Err(e) = white.set_duty_cycle(dimmed(val.white())) {
            rprintln!("white set_duty_cycle error: {:?}", e);
        }
    }
}
