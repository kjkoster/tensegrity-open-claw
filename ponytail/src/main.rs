#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

mod http_server;
mod sacn;
mod storage;
mod wifi;

use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::gpio::DriveMode;
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::ledc::{
    LSGlobalClkSource, Ledc, LowSpeed,
    channel::{self, ChannelIFace},
    timer::{self, TimerIFace},
};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use rtt_target::rprintln;
use static_cell::StaticCell;

extern crate alloc;

#[panic_handler]
fn panic(panic_info: &core::panic::PanicInfo) -> ! {
    rprintln!("{}", panic_info);
    loop {}
}

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

static STORAGE: StaticCell<storage::Storage> = StaticCell::new();
static DMX_SAVE: Signal<CriticalSectionRawMutex, u16> = Signal::new();
static DMX_VALUE: Signal<CriticalSectionRawMutex, u8> = Signal::new();
static WIFI_SAVE: Signal<CriticalSectionRawMutex, http_server::WifiConfig> = Signal::new();

#[embassy_executor::task]
async fn persist_dmx_task(
    storage: &'static storage::Storage,
    save_signal: &'static Signal<CriticalSectionRawMutex, u16>,
) -> ! {
    loop {
        let addr = save_signal.wait().await;
        if let Err(e) = storage.write_dmx_base_address(addr) {
            // OtherCoreRunning is seen under `cargo run` (probe-rs): the debugger
            // leaves Core 1 active at start-up and the ESP32-S3 flash driver refuses
            // writes while either core fetches from flash. The condition is transient;
            // retrying would fix it. When attaching after boot the write succeeds.
            rprintln!("storage write failed: {:?}", e);
        }
    }
}

#[embassy_executor::task]
async fn persist_wifi_task(
    storage: &'static storage::Storage,
    wifi_signal: &'static Signal<CriticalSectionRawMutex, http_server::WifiConfig>,
) -> ! {
    let config = wifi_signal.wait().await;
    storage.write_ssid(&config.ssid).ok();
    storage.write_password(&config.password).ok();
    // Brief pause so the HTTP response finishes sending before we reset.
    Timer::after(Duration::from_millis(500)).await;
    esp_hal::system::software_reset()
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
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);

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

    rprintln!("Embassy initialized!");

    let rng = esp_hal::rng::Rng::new();
    let seed = (rng.random() as u64) | ((rng.random() as u64) << 32);

    let storage = STORAGE.init(storage::Storage::new(peripherals.FLASH));
    spawner.spawn(persist_dmx_task(storage, &DMX_SAVE).unwrap());
    spawner.spawn(persist_wifi_task(storage, &WIFI_SAVE).unwrap());

    let stack = wifi::connect(
        spawner,
        peripherals.WIFI,
        seed,
        storage.read_ssid(),
        storage.read_password(),
    )
    .await;

    http_server::spawn(
        spawner,
        stack,
        storage.read_dmx_base_address(),
        storage.read_ssid(),
        &DMX_SAVE,
        &WIFI_SAVE,
    );
    sacn::spawn(spawner, stack, storage, &DMX_VALUE);

    // GPIO21 is the single user-controllable yellow LED on the XIAO ESP32-S3 (active low).
    // LEDC duty 0% = GPIO low = LED on; duty 100% = GPIO high = LED off.
    // Invert DMX value: 0 → 100% duty (off), 255 → 0% duty (full brightness).
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

    let mut led_channel = ledc.channel::<LowSpeed>(channel::Number::Channel0, peripherals.GPIO21);
    led_channel
        .configure(channel::config::Config {
            timer: &lstimer,
            duty_pct: 0,
            drive_mode: DriveMode::PushPull,
        })
        .unwrap();

    loop {
        let val = DMX_VALUE.wait().await;
        let duty_pct = 100 - (val as u32 * 100 / 255) as u8;
        led_channel.set_duty(duty_pct).ok();
    }
}
