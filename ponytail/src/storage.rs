extern crate alloc;

use alloc::string::String;
use core::cell::RefCell;
use critical_section::Mutex;
use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};
use esp_hal::peripherals::FLASH;
use esp_storage::{FlashStorage, FlashStorageError};

// Flash layout — first sector of the NVS partition (default IDF layout).
// This project does not use IDF NVS, so the full 4 KiB sector is available
// as raw storage.
//
//   [0..4]    magic = b"DMX1"
//   [4..6]    dmx_base_address  (u16 LE)
//   [6]       ssid_len          (u8, 0..=32)
//   [7..39]   ssid              (32 bytes, zero-padded)
//   [39]      password_len      (u8, 0..=64)
//   [40..104] password          (64 bytes, zero-padded)
//
// Total 104 bytes — a multiple of WORD_SIZE (4), so reads and writes are
// always aligned.
const BASE: u32 = 0x9000;
const MAGIC: [u8; 4] = *b"DMX1";
const SSID_MAX: usize = 32;
const PASSWORD_MAX: usize = 64;
const RECORD_SIZE: usize = 104;

const DEFAULT_DMX_BASE_ADDRESS: u16 = 333;
const DEFAULT_SSID: &str = "radiowaves";
const DEFAULT_PASSWORD: &str = "IkWilInternetten!!";

const fn str_to_fixed<const N: usize>(s: &str) -> ([u8; N], u8) {
    let b = s.as_bytes();
    assert!(b.len() <= N, "string too long for storage slot");
    let mut arr = [0u8; N];
    let mut i = 0;
    while i < b.len() {
        arr[i] = b[i];
        i += 1;
    }
    (arr, b.len() as u8)
}

const SSID_INIT: ([u8; SSID_MAX], u8) = str_to_fixed(DEFAULT_SSID);
const PWD_INIT: ([u8; PASSWORD_MAX], u8) = str_to_fixed(DEFAULT_PASSWORD);

struct Settings {
    dmx_base_address: u16,
    ssid: [u8; SSID_MAX],
    ssid_len: u8,
    password: [u8; PASSWORD_MAX],
    password_len: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dmx_base_address: DEFAULT_DMX_BASE_ADDRESS,
            ssid: SSID_INIT.0,
            ssid_len: SSID_INIT.1,
            password: PWD_INIT.0,
            password_len: PWD_INIT.1,
        }
    }
}

// 4-byte aligned buffer for flash reads and writes (WORD_SIZE = 4).
#[repr(C, align(4))]
struct AlignedRecord([u8; RECORD_SIZE]);

pub struct Storage {
    settings: Mutex<RefCell<Settings>>,
    flash: Mutex<RefCell<Option<FlashStorage<'static>>>>,
}

impl Storage {
    /// Load persisted settings from flash, falling back to compiled-in
    /// defaults when no valid record is found (first boot or erased flash).
    #[allow(clippy::large_stack_frames)] // called once at boot; Clippy over-counts inlined Mutex/RefCell temporaries
    pub fn new(flash: FLASH<'static>) -> Self {
        let mut fs = FlashStorage::new(flash);
        let mut rec = AlignedRecord([0xFF; RECORD_SIZE]);
        let mut settings = Settings::default();

        if fs.read(BASE, &mut rec.0).is_ok() && rec.0[..4] == MAGIC {
            let dmx = u16::from_le_bytes([rec.0[4], rec.0[5]]);
            let ssid_len = rec.0[6] as usize;
            let pwd_len = rec.0[39] as usize;

            if (1..=512).contains(&dmx) && ssid_len <= SSID_MAX && pwd_len <= PASSWORD_MAX {
                settings.dmx_base_address = dmx;
                settings.ssid_len = ssid_len as u8;
                settings.ssid[..ssid_len].copy_from_slice(&rec.0[7..7 + ssid_len]);
                settings.password_len = pwd_len as u8;
                settings.password[..pwd_len].copy_from_slice(&rec.0[40..40 + pwd_len]);
            }
        }

        Self {
            settings: Mutex::new(RefCell::new(settings)),
            flash: Mutex::new(RefCell::new(Some(fs))),
        }
    }

    pub fn read_dmx_base_address(&self) -> u16 {
        critical_section::with(|cs| self.settings.borrow(cs).borrow().dmx_base_address)
    }

    pub fn read_ssid(&self) -> String {
        critical_section::with(|cs| {
            let s = self.settings.borrow(cs).borrow();
            let len = s.ssid_len as usize;
            core::str::from_utf8(&s.ssid[..len])
                .unwrap_or(DEFAULT_SSID)
                .into()
        })
    }

    pub fn read_password(&self) -> String {
        critical_section::with(|cs| {
            let s = self.settings.borrow(cs).borrow();
            let len = s.password_len as usize;
            core::str::from_utf8(&s.password[..len])
                .unwrap_or(DEFAULT_PASSWORD)
                .into()
        })
    }

    pub fn write_dmx_base_address(&self, addr: u16) -> Result<(), FlashStorageError> {
        if !(1..=512).contains(&addr) {
            return Err(FlashStorageError::OutOfBounds);
        }
        critical_section::with(|cs| {
            self.settings.borrow(cs).borrow_mut().dmx_base_address = addr;
        });
        self.flush()
    }

    pub fn write_ssid(&self, ssid: &str) -> Result<(), FlashStorageError> {
        if ssid.len() > SSID_MAX {
            return Err(FlashStorageError::OutOfBounds);
        }
        critical_section::with(|cs| {
            let mut s = self.settings.borrow(cs).borrow_mut();
            s.ssid_len = ssid.len() as u8;
            s.ssid[..ssid.len()].copy_from_slice(ssid.as_bytes());
        });
        self.flush()
    }

    pub fn write_password(&self, password: &str) -> Result<(), FlashStorageError> {
        if password.len() > PASSWORD_MAX {
            return Err(FlashStorageError::OutOfBounds);
        }
        critical_section::with(|cs| {
            let mut s = self.settings.borrow(cs).borrow_mut();
            s.password_len = password.len() as u8;
            s.password[..password.len()].copy_from_slice(password.as_bytes());
        });
        self.flush()
    }

    fn flush(&self) -> Result<(), FlashStorageError> {
        let mut rec = AlignedRecord([0xFF; RECORD_SIZE]);

        critical_section::with(|cs| {
            let s = self.settings.borrow(cs).borrow();
            let ssid_len = s.ssid_len as usize;
            let pwd_len = s.password_len as usize;
            rec.0[..4].copy_from_slice(&MAGIC);
            rec.0[4..6].copy_from_slice(&s.dmx_base_address.to_le_bytes());
            rec.0[6] = s.ssid_len;
            rec.0[7..7 + ssid_len].copy_from_slice(&s.ssid[..ssid_len]);
            rec.0[39] = s.password_len;
            rec.0[40..40 + pwd_len].copy_from_slice(&s.password[..pwd_len]);
        });

        // Take storage out so the erase/write (~40 ms) runs outside any critical
        // section, keeping the embassy executor responsive.
        let mut opt = critical_section::with(|cs| self.flash.borrow(cs).borrow_mut().take());

        let result = if let Some(fs) = opt.as_mut() {
            fs.erase(BASE, BASE + FlashStorage::SECTOR_SIZE)
                .and_then(|_| fs.write(BASE, &rec.0))
        } else {
            Ok(())
        };

        critical_section::with(|cs| {
            *self.flash.borrow(cs).borrow_mut() = opt;
        });

        result
    }
}
