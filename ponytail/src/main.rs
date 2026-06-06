#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

mod http_server;
mod storage;

use core::sync::atomic::{AtomicU16, Ordering};

use embassy_executor::Spawner;
use embassy_net::StackResources;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::wifi::{Interface, sta::StationConfig};
use rtt_target::rprintln;
use static_cell::StaticCell;

extern crate alloc;

pub(crate) static DMX_BASE_ADDRESS: AtomicU16 = AtomicU16::new(333);

#[panic_handler]
fn panic(panic_info: &core::panic::PanicInfo) -> ! {
    rprintln!("{}", panic_info);
    loop {}
}

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

static STACK_RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();

#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, Interface<'static>>) -> ! {
    runner.run().await
}

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // RTT must be initialized first so panics during startup produce visible output.
    rtt_target::rtt_init_print!();

    storage::init();
    if let Some(addr) = storage::load() {
        DMX_BASE_ADDRESS.store(addr, Ordering::Relaxed);
        rprintln!("dmx base address from nvs: {}", addr);
    } else {
        rprintln!(
            "default dmx base address: {}",
            DMX_BASE_ADDRESS.load(Ordering::Relaxed)
        );
    }

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
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

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    rprintln!("Embassy initialized!");

    let rng = esp_hal::rng::Rng::new();
    let seed = (rng.random() as u64) | ((rng.random() as u64) << 32);

    let (mut wifi_controller, interfaces) =
        esp_radio::wifi::new(peripherals.WIFI, Default::default())
            .unwrap_or_else(|e| panic!("Failed to initialize Wi-Fi controller: {:?}", e));

    let (stack, runner) = embassy_net::new(
        interfaces.station,
        embassy_net::Config::dhcpv4(Default::default()),
        STACK_RESOURCES.init(StackResources::new()),
        seed,
    );
    spawner.spawn(net_task(runner).unwrap());
    storage::spawn(spawner);

    wifi_controller
        .set_config(&esp_radio::wifi::Config::Station(
            StationConfig::default()
                .with_ssid("radiowaves")
                .with_password("IkWilInternetten!!".into()),
        ))
        .unwrap();

    rprintln!("connecting to radiowaves...");
    wifi_controller.connect_async().await.unwrap();
    rprintln!("wifi connected, waiting for dhcp...");
    stack.wait_config_up().await;

    if let Some(cfg) = stack.config_v4() {
        rprintln!("ip address: {}", cfg.address.address());
        rprintln!("netmask:    {}", cfg.address.netmask());
        rprintln!("dmx config: http://{}/", cfg.address.address());
        for dns in cfg.dns_servers.iter() {
            rprintln!("nameserver: {}", dns);
        }
    }

    http_server::spawn(spawner, stack, &DMX_BASE_ADDRESS, storage::signal());

    // GPIO21 is the single user-controllable yellow LED on the XIAO ESP32-S3 (active low).
    let mut led = Output::new(peripherals.GPIO21, Level::High, OutputConfig::default());

    loop {
        led.set_low();
        Timer::after(Duration::from_millis(15)).await;
        led.set_high();
        Timer::after(Duration::from_millis(985)).await;
    }
}
