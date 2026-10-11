#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

mod config;
mod led;
mod models;
mod sacn;
mod wifi;

use embassy_executor::Spawner;
use embassy_net::Stack;
use esp_hal::{
    clock::{CpuClock, cpu_clock},
    efuse::{self, InterfaceMacAddress},
    interrupt::software::SoftwareInterruptControl,
    rng::Rng,
    timer::timg::TimerGroup,
};
use models::{DmxConfig, DmxWatch};
use rtt_target::rprintln;

extern crate alloc;

#[panic_handler]
fn panic(panic_info: &core::panic::PanicInfo) -> ! {
    rprintln!("{}", panic_info);
    loop {}
}

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

// The latest DMX value off the wire. See `models::DMX_CONSUMERS`.
static DMX_VALUE: DmxWatch = DmxWatch::new();

#[embassy_executor::task]
async fn sacn_listener(config: DmxConfig, network_stack: Stack<'static>) -> ! {
    let mut buffers = sacn::Buffers::new();
    let dmx_value = DMX_VALUE.sender();
    loop {
        network_stack.wait_config_up().await;
        // listen() returns on a universe timeout; calling it again rejoins with a
        // fresh socket.
        sacn::listen(network_stack, config, &dmx_value, &mut buffers).await;
    }
}

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // RTT must be initialized first so panics during startup produce visible output.
    rtt_target::rtt_init_print!();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    esp_alloc::heap_allocator!(size: 72 * 1024);

    let peripherals = esp_hal::init(config);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    // Before anything that can wait or panic, so the fade shows the firmware is alive
    // while the network is still coming up.
    let status_led = led::StatusLed::new(peripherals.LEDC, peripherals.GPIO21);
    spawner.spawn(led::sine_fade(status_led).unwrap());

    rprintln!("");
    rprintln!("ball firmware started");
    rprintln!("cpu:           {} MHz", cpu_clock().as_mhz());

    // Read this board's station MAC from efuse (before WiFi starts) and use it to
    // pick its compiled-in configuration. See config.rs for the why and the
    // first-time provisioning procedure.
    let mac_address: [u8; 6] = efuse::interface_mac_address(InterfaceMacAddress::Station)
        .as_bytes()
        .try_into()
        .unwrap();
    rprintln!(
        "mac address:    {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        mac_address[0],
        mac_address[1],
        mac_address[2],
        mac_address[3],
        mac_address[4],
        mac_address[5]
    );

    let dmx_config = config::dmx_config_for(mac_address);

    let rng = Rng::new();
    let seed = (rng.random() as u64) | ((rng.random() as u64) << 32);
    let network_stack = wifi::connect(spawner, peripherals.WIFI, seed, config::WIFI_CONFIG).await;

    spawner.spawn(sacn_listener(dmx_config, network_stack).unwrap());

    let mut dmx_value = DMX_VALUE.receiver().unwrap();
    loop {
        let value = dmx_value.changed().await;
        rprintln!("dmx:            {:?}", value);
    }
}
