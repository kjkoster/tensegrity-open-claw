use core::cell::RefCell;

use critical_section::Mutex;
use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};
use esp_hal::peripherals::FLASH;
use esp_storage::FlashStorage;

// First sector of the NVS partition (default IDF layout). This project does
// not use IDF NVS, so the full 4 KiB sector is available as raw storage.
const BASE: u32 = 0x9000;
const MAGIC: [u8; 4] = *b"DMX1";

static FLASH_STORAGE: Mutex<RefCell<Option<FlashStorage<'static>>>> =
    Mutex::new(RefCell::new(None));
static SAVE_SIGNAL: Signal<CriticalSectionRawMutex, u16> = Signal::new();

pub fn init() {
    // SAFETY: called once before any driver takes the FLASH peripheral; the
    // ROM SPI flash routines are safe to call before esp_hal::init().
    let flash = unsafe { FLASH::steal() };
    let storage = FlashStorage::new(flash);
    critical_section::with(|cs| {
        *FLASH_STORAGE.borrow(cs).borrow_mut() = Some(storage);
    });
}

pub fn load() -> Option<u16> {
    critical_section::with(|cs| {
        let mut guard = FLASH_STORAGE.borrow(cs).borrow_mut();
        let flash = guard.as_mut()?;
        let mut buf = [0u8; 8];
        flash.read(BASE, &mut buf).ok()?;
        if buf[..4] == MAGIC {
            Some(u16::from_le_bytes([buf[4], buf[5]]))
        } else {
            None
        }
    })
}

pub fn signal() -> &'static Signal<CriticalSectionRawMutex, u16> {
    &SAVE_SIGNAL
}

pub fn spawn(spawner: Spawner) {
    spawner.spawn(persist_task().unwrap());
}

#[embassy_executor::task]
async fn persist_task() -> ! {
    loop {
        let addr = SAVE_SIGNAL.wait().await;
        save(addr);
    }
}

fn save(addr: u16) {
    // Take ownership out of the global so the erase/write (~40 ms) happens
    // outside any critical section, allowing the embassy executor to remain
    // responsive.
    let mut opt = critical_section::with(|cs| FLASH_STORAGE.borrow(cs).borrow_mut().take());

    if let Some(flash) = opt.as_mut() {
        let mut buf = [0xFFu8; 8];
        buf[..4].copy_from_slice(&MAGIC);
        buf[4..6].copy_from_slice(&addr.to_le_bytes());
        flash.erase(BASE, BASE + 0x1000).ok();
        flash.write(BASE, &buf).ok();
    }

    critical_section::with(|cs| {
        *FLASH_STORAGE.borrow(cs).borrow_mut() = opt;
    });
}
