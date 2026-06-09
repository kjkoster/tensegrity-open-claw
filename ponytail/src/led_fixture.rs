use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embedded_hal::pwm::SetDutyCycle;

pub async fn run(
    dmx_value: &Signal<CriticalSectionRawMutex, u8>,
    led: &mut impl SetDutyCycle,
) -> ! {
    loop {
        let val = dmx_value.wait().await;
        let max = led.max_duty_cycle();
        // Active-low LED: DMX 0 → full brightness (duty 0); DMX 255 → off (duty max).
        let duty = max - (val as u32 * max as u32 / 255) as u16;
        led.set_duty_cycle(duty).ok();
    }
}
