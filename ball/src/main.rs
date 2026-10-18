#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

mod analog;
mod config;
mod led;
mod models;
mod motor;
mod pattern;
mod sacn;
mod telemetry;
mod wifi;

use alloc::format;
use embassy_executor::Spawner;
use embassy_net::Stack;
use embassy_time::with_deadline;
use embedded_storage::ReadStorage;
use esp_hal::{
    clock::{CpuClock, cpu_clock},
    efuse::{self, InterfaceMacAddress},
    interrupt::software::SoftwareInterruptControl,
    rng::Rng,
    system::reset_reason,
    timer::timg::TimerGroup,
};
use models::{DmxConfig, DmxState, DmxWatch, Look};
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

// The latest DMX state off the wire. See `models::DMX_CONSUMERS`.
static DMX_STATE: DmxWatch = DmxWatch::new();

#[embassy_executor::task]
async fn sacn_listener(config: DmxConfig, network_stack: Stack<'static>) -> ! {
    let mut buffers = sacn::Buffers::new();
    let dmx_state = DMX_STATE.sender();
    loop {
        network_stack.wait_config_up().await;
        // listen() returns on a universe timeout; calling it again rejoins with a
        // fresh socket.
        sacn::listen(network_stack, config, &dmx_state, &mut buffers).await;
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

    // Half the chip's top speed: the ball's work is waiting on the network and the clock, and
    // the lower clock draws less from the pack.
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::_80MHz);
    esp_alloc::heap_allocator!(size: 72 * 1024);

    let peripherals = esp_hal::init(config);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    // Before anything that can wait or panic, so the pins are driven and the LEDs and the
    // motor are held off until the rig says otherwise.
    let leds = led::Leds::new(
        peripherals.LEDC,
        peripherals.GPIO21,
        peripherals.GPIO20,
        peripherals.GPIO10,
    );
    let mut motor = motor::Motor::new(peripherals.GPIO7);
    spawner.spawn(pattern::render(leds.power).unwrap());
    let look = pattern::LOOK.sender();

    rprintln!("");
    rprintln!("ball firmware started");
    rprintln!("firmware:       {}", env!("FIRMWARE"));
    rprintln!("cpu:           {} MHz", cpu_clock().as_mhz());

    // Why this boot happened. On a ball that runs on a pack, a brownout reset is what a sagging
    // pack looks like, and nothing else would tell it apart from the ball simply being switched
    // on.
    let reset = reset_reason().map_or_else(|| "unknown".into(), |reason| format!("{:?}", reason));
    rprintln!("reset reason:   {}", reset);
    telemetry::publish("reset_reason", reset);
    telemetry::publish("firmware", env!("FIRMWARE"));

    // What this ball is, in the `sys` subtree every part of the rig publishes its own under.
    telemetry::publish("sys/board", esp_metadata_generated::chip_pretty!());
    telemetry::publish(
        "sys/cores",
        format!("{}", esp_metadata_generated::property!("cores")),
    );
    telemetry::publish("sys/cpu_frequency_mhz", format!("{}", cpu_clock().as_mhz()));
    telemetry::publish("sys/triplet", env!("TRIPLET"));
    // The SRAM the chip maps as data RAM: all of it but the part that caches the flash.
    let dram = esp_metadata_generated::memory_range!("DRAM");
    telemetry::publish("sys/ram_kb", format!("{}", (dram.end - dram.start) / 1024));
    // The flash size the bootloader's header records, which espflash writes from the chip it
    // flashed. Nothing else in the firmware touches the flash, so the peripheral is taken here
    // and let go again.
    let flash = esp_storage::FlashStorage::new(peripherals.FLASH);
    telemetry::publish("sys/flash_kb", format!("{}", flash.capacity() / 1024));
    drop(flash);

    // A hardware test, ahead of the network: it does not need it, and it runs from boot.
    let sensors = analog::Sensors::new(peripherals.ADC1, peripherals.GPIO3, peripherals.GPIO4);
    spawner.spawn(analog::sense(sensors).unwrap());

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
    telemetry::publish(
        "mac",
        format!(
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            mac_address[0],
            mac_address[1],
            mac_address[2],
            mac_address[3],
            mac_address[4],
            mac_address[5]
        ),
    );

    let dmx_config = config::dmx_config_for(mac_address);
    // Published now and sent on every connect, like everything else the ball publishes.
    telemetry::publish("dmx/universe", format!("{}", dmx_config.universe()));
    telemetry::publish("dmx/address", format!("{}", dmx_config.address()));

    let rng = Rng::new();
    let seed = (rng.random() as u64) | ((rng.random() as u64) << 32);
    let network_stack = wifi::connect(spawner, peripherals.WIFI, seed, config::WIFI_CONFIG).await;

    spawner.spawn(sacn_listener(dmx_config, network_stack).unwrap());
    spawner.spawn(telemetry::run(network_stack).unwrap());

    let mut dmx_state = DMX_STATE.receiver().unwrap();
    // Whether the universe is being heard, published on the change alone: it is what tells a
    // ball that is on the network but deaf to the multicast apart from one that is following.
    let mut live = None;
    loop {
        // A run in progress bounds the wait, because nothing arrives from a rig that simply
        // holds the motor on, and that is the case the bound is for.
        let state = match motor.deadline() {
            Some(deadline) => with_deadline(deadline, dmx_state.changed()).await.ok(),
            None => Some(dmx_state.changed().await),
        };
        if let Some(state) = state {
            let heard = matches!(state, DmxState::Live(_));
            if live != Some(heard) {
                live = Some(heard);
                telemetry::publish("dmx_live", rig_mqtt::boolean(heard));
            }
        }
        match state {
            Some(DmxState::Live(value)) => {
                leds.status.set_brightness(value.onboard_led());
                look.send(value.look());
                motor.follow(value.motor());
            }
            // The outputs that do harm when left on at the word of a sender that is gone: the
            // motor, and the power LEDs at full current. The status LED keeps its level.
            Some(DmxState::Lost) => {
                rprintln!("universe lost, stopping the motor and darkening the power leds");
                motor.trip();
                look.send(Look::DARK);
            }
            None => {
                rprintln!(
                    "motor ran for {} seconds, stopping it",
                    motor::MAX_RUN.as_secs()
                );
                motor.trip();
            }
        }
    }
}
