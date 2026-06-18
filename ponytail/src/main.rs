#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

mod ble;
mod config;
mod led_fixture;
mod models;
mod sacn;
mod wifi;

use embassy_executor::Spawner;
use embassy_net::Stack;
use esp_hal::{
    clock::{CpuClock, cpu_clock},
    efuse::{self, InterfaceMacAddress},
    interrupt::software::SoftwareInterruptControl,
    peripherals::BT,
    rng::Rng,
    timer::timg::TimerGroup,
};
use esp_hal::{
    gpio::DriveMode,
    ledc::{
        LSGlobalClkSource, Ledc, LowSpeed,
        channel::{self, ChannelIFace, Number as ChannelNumber},
        timer::{self, TimerIFace},
    },
    time::Rate,
};
use models::{DmxConfig, DmxReceiver, DmxWatch};
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

// The latest DMX value, fanned out to both consumer personalities. See
// `models::DMX_CONSUMERS`.
static DMX_VALUE: DmxWatch = DmxWatch::new();

#[embassy_executor::task]
async fn sacn_listener(config: DmxConfig, network_stack: Stack<'static>) -> ! {
    loop {
        network_stack.wait_config_up().await;
        let mut listener = sacn::Listener::new(network_stack, config, DMX_VALUE.sender());
        // run() returns on a universe timeout; drop the listener (leaving the
        // multicast group) and recreate it so it rejoins with a fresh socket.
        listener.run().await;
    }
}

/// BLE-bridge consumer, spawned as its own task so it runs in parallel with the PWM
/// `led_fixture` (which `main` drives directly). Both observe `DMX_VALUE`.
#[embassy_executor::task]
async fn ble_bridge(dmx_value: DmxReceiver, bt: BT<'static>, target: [u8; 6]) -> ! {
    ble::run(dmx_value, bt, target).await
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
    // One plain heap region, sized for WiFi + BLE coexistence with headroom to spare.
    // (We could pool an extra ~72 KB reclaimed from the ROM bootloader, but a single
    // region is far easier to reason about and we have RAM to spare.) If this ever
    // fails to link, regular DRAM is tighter than expected — drop to a smaller size.
    esp_alloc::heap_allocator!(size: 128 * 1024);

    let peripherals = esp_hal::init(config);
    // GPIO27-37 are used internally by the XIAO ESP32-S3's octal PSRAM. Consuming
    // them here prevents accidental reuse; do not reassign these pins.
    let _ = peripherals.GPIO27;
    let _ = peripherals.GPIO28;
    let _ = peripherals.GPIO29;
    let _ = peripherals.GPIO30;
    let _ = peripherals.GPIO31;
    let _ = peripherals.GPIO32;
    let _ = peripherals.GPIO33;
    let _ = peripherals.GPIO34;
    let _ = peripherals.GPIO35;
    let _ = peripherals.GPIO36;
    let _ = peripherals.GPIO37;

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    rprintln!("");
    rprintln!("cpu:            {} MHz", cpu_clock().as_mhz());

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

    // The BLE controller's public address — what a sniffer sees as the ESP's InitA in
    // its CONNECT_IND. Derived from the same efuse base MAC as the station MAC above.
    let bt_mac: [u8; 6] = efuse::interface_mac_address(InterfaceMacAddress::Bluetooth)
        .as_bytes()
        .try_into()
        .unwrap();
    rprintln!(
        "ble mac:        {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        bt_mac[0],
        bt_mac[1],
        bt_mac[2],
        bt_mac[3],
        bt_mac[4],
        bt_mac[5]
    );

    let dmx_config = config::dmx_config_for(mac_address);
    let wifi_config = config::wifi_config();

    let rng = Rng::new();
    let seed = (rng.random() as u64) | ((rng.random() as u64) << 32);
    let (network_stack, _) = wifi::connect(spawner, peripherals.WIFI, seed, &wifi_config).await;

    spawner.spawn(sacn_listener(dmx_config, network_stack).unwrap());

    // ── Consumers ───────────────────────────────────────────────────────────────
    // Both consumer personalities run at once, each observing DMX_VALUE through its
    // own Watch receiver: the BLE bridge writes the fixture's original Telink
    // controller over BLE, and the PWM led_fixture drives the RGBW array over LEDC.
    // The BLE bridge is spawned as a task; the PWM personality is awaited directly at
    // the end of main, so this function never returns.
    let ble_value = DMX_VALUE.receiver().unwrap();
    let pwm_value = DMX_VALUE.receiver().unwrap();

    let target = config::ble_target_mac_for(mac_address)
        .expect("no BLE target in config::BOARDS for this board");
    rprintln!(
        "ble address:    {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        target[0],
        target[1],
        target[2],
        target[3],
        target[4],
        target[5]
    );
    // Heap state just before BLE coexistence allocates its controller-thread stack:
    // the stack needs one contiguous block, so the largest free block matters as much
    // as the total.
    rprintln!("heap stats:\n{}", esp_alloc::HEAP.stats());
    spawner.spawn(ble_bridge(ble_value, peripherals.BT, target).unwrap());

    rprintln!("pwm fixture:    LEDC");

    // GPIO21 is the single user-controllable yellow LED on the XIAO ESP32-S3 (active low).
    let mut ledc = Ledc::new(peripherals.LEDC);
    ledc.set_global_slow_clock(LSGlobalClkSource::APBClk);

    let mut lstimer = ledc.timer::<LowSpeed>(timer::Number::Timer0);
    lstimer
        .configure(timer::config::Config {
            duty: timer::config::Duty::Duty8Bit,
            clock_source: timer::LSClockSource::APBClk,
            frequency: Rate::from_khz(20),
        })
        .unwrap();

    let ch_cfg = channel::config::Config {
        timer: &lstimer,
        duty_pct: 0,
        drive_mode: DriveMode::PushPull,
    };

    let mut onboard_channel = ledc.channel::<LowSpeed>(ChannelNumber::Channel0, peripherals.GPIO21);
    onboard_channel.configure(ch_cfg).unwrap();

    let mut red_channel = ledc.channel::<LowSpeed>(ChannelNumber::Channel1, peripherals.GPIO9);
    red_channel.configure(ch_cfg).unwrap();

    let mut green_channel = ledc.channel::<LowSpeed>(ChannelNumber::Channel2, peripherals.GPIO8);
    green_channel.configure(ch_cfg).unwrap();

    let mut blue_channel = ledc.channel::<LowSpeed>(ChannelNumber::Channel3, peripherals.GPIO7);
    blue_channel.configure(ch_cfg).unwrap();

    let mut white_channel = ledc.channel::<LowSpeed>(ChannelNumber::Channel4, peripherals.GPIO44);
    white_channel.configure(ch_cfg).unwrap();

    led_fixture::run(
        pwm_value,
        &mut onboard_channel,
        &mut red_channel,
        &mut green_channel,
        &mut blue_channel,
        &mut white_channel,
    )
    .await
}
