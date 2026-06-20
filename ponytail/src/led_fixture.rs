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

        // The Telink fixture is modal: the white LED and the RGB emitters cannot light
        // together, and White > 0 wins. Mirror that interlock here so the PWM reference
        // path stays faithful to the BLE fixture instead of co-lighting RGB and W.
        let (r, g, b) = if val.white() > 0 {
            (0, 0, 0)
        } else {
            (val.red(), val.green(), val.blue())
        };

        if let Err(e) = onboard.set_duty_cycle(max - duty(intensity)) {
            rprintln!("onboard set_duty_cycle error: {:?}", e);
        }
        if let Err(e) = red.set_duty_cycle(dimmed(r)) {
            rprintln!("red set_duty_cycle error: {:?}", e);
        }
        if let Err(e) = green.set_duty_cycle(dimmed(g)) {
            rprintln!("green set_duty_cycle error: {:?}", e);
        }
        if let Err(e) = blue.set_duty_cycle(dimmed(b)) {
            rprintln!("blue set_duty_cycle error: {:?}", e);
        }
        if let Err(e) = white.set_duty_cycle(dimmed(val.white())) {
            rprintln!("white set_duty_cycle error: {:?}", e);
        }
    }
}
