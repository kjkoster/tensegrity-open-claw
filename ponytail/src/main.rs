#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

mod http_server;
mod led_fixture;
mod models;
mod sacn;
mod storage;
mod wifi;

use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_net::Stack;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};
use esp_hal::{
    clock::CpuClock,
    gpio::DriveMode,
    interrupt::software::SoftwareInterruptControl,
    ledc::{
        LSGlobalClkSource, Ledc, LowSpeed,
        channel::{self, ChannelIFace, Number as ChannelNumber},
        timer::{self, TimerIFace},
    },
    rng::Rng,
    time::Rate,
    timer::timg::TimerGroup,
};
use esp_storage::FlashStorage;
use models::{DmxConfig, DmxValue, WifiConfig};
use rtt_target::rprintln;
use static_cell::StaticCell;
use storage::Storage;

extern crate alloc;

#[panic_handler]
fn panic(panic_info: &core::panic::PanicInfo) -> ! {
    rprintln!("{}", panic_info);
    loop {}
}

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

static STORAGE: StaticCell<Storage<FlashStorage<'static>>> = StaticCell::new();
static DMX_SAVE: Signal<CriticalSectionRawMutex, DmxConfig> = Signal::new();
static DMX_VALUE: Signal<CriticalSectionRawMutex, DmxValue> = Signal::new();
static WIFI_SAVE: Signal<CriticalSectionRawMutex, WifiConfig> = Signal::new();

#[embassy_executor::task]
async fn persist_dmx_config(
    storage: &'static Storage<FlashStorage<'static>>,
    dmx_save_signal: &'static Signal<CriticalSectionRawMutex, DmxConfig>,
    network_stack: Stack<'static>,
) -> ! {
    let mut listener = sacn::Listener::new(network_stack, storage.read_dmx_config(), &DMX_VALUE);
    loop {
        match select(listener.run(), dmx_save_signal.wait()).await {
            Either::First(never) => match never {},
            Either::Second(config) => {
                if let Err(e) = storage.write_dmx_config(config) {
                    rprintln!("storage write failed: {:?}", e);
                }
                drop(listener);
                listener = sacn::Listener::new(network_stack, config, &DMX_VALUE);
            }
        }
    }
}

#[embassy_executor::task]
async fn persist_wifi_config(
    storage: &'static Storage<FlashStorage<'static>>,
    wifi_save_signal: &'static Signal<CriticalSectionRawMutex, WifiConfig>,
) -> ! {
    let config = wifi_save_signal.wait().await;
    storage.write_wifi_config(&config).ok();
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

    let storage = STORAGE.init(storage::Storage::new(
        FlashStorage::new(peripherals.FLASH),
        0x9000,
    ));

    let rng = Rng::new();
    let seed = (rng.random() as u64) | ((rng.random() as u64) << 32);
    let wifi_config = storage.read_wifi_config();
    let network_stack = wifi::connect(spawner, peripherals.WIFI, seed, &wifi_config).await;

    spawner.spawn(persist_dmx_config(storage, &DMX_SAVE, network_stack).unwrap());
    spawner.spawn(persist_wifi_config(storage, &WIFI_SAVE).unwrap());

    http_server::spawn(
        spawner,
        network_stack,
        storage.read_dmx_config(),
        &wifi_config,
        &DMX_SAVE,
        &WIFI_SAVE,
    );

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
        &DMX_VALUE,
        &mut onboard_channel,
        &mut red_channel,
        &mut green_channel,
        &mut blue_channel,
        &mut white_channel,
    )
    .await
}
