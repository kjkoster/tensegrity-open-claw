extern crate alloc;

use core::cell::RefCell;
use critical_section::Mutex;
use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};
use esp_hal::peripherals::FLASH;
use esp_storage::{FlashStorage, FlashStorageError};

use crate::models::{DmxConfig, WifiConfig};

// Flash layout — first sector of the NVS partition (default IDF layout).
// This project does not use IDF NVS, so the full 4 KiB sector is available
// as raw storage.
//
//   [0..4]    magic = b"DMX3"
//   [4..6]    dmx_base_address  (u16 LE)
//   [6]       ssid_len          (u8, 0..=32)
//   [7..39]   ssid              (32 bytes, zero-padded)
//   [39]      password_len      (u8, 0..=64)
//   [40..104] password          (64 bytes, zero-padded)
//   [104..106] universe         (u16 LE, 1..=63999)
//   [106..108] sacn_port        (u16 LE, 1..=65535)
//
// Total 108 bytes — a multiple of WORD_SIZE (4), so reads and writes are
// always aligned.
const BASE: u32 = 0x9000;
const MAGIC: [u8; 4] = *b"DMX3";
const SSID_MAX: usize = 32;
const PASSWORD_MAX: usize = 64;
const RECORD_SIZE: usize = 108;

const DEFAULT_DMX_BASE_ADDRESS: u16 = 333;
const DEFAULT_UNIVERSE: u16 = 1;
const DEFAULT_SACN_PORT: u16 = 5568;
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
    universe: u16,
    sacn_port: u16,
    ssid: [u8; SSID_MAX],
    ssid_len: u8,
    password: [u8; PASSWORD_MAX],
    password_len: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dmx_base_address: DEFAULT_DMX_BASE_ADDRESS,
            universe: DEFAULT_UNIVERSE,
            sacn_port: DEFAULT_SACN_PORT,
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
            let universe = u16::from_le_bytes([rec.0[104], rec.0[105]]);
            let sacn_port = u16::from_le_bytes([rec.0[106], rec.0[107]]);

            if (1..=512).contains(&dmx)
                && ssid_len <= SSID_MAX
                && pwd_len <= PASSWORD_MAX
                && (1..=63999).contains(&universe)
                && sacn_port != 0
            {
                settings.dmx_base_address = dmx;
                settings.universe = universe;
                settings.sacn_port = sacn_port;
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

    pub fn read_dmx_config(&self) -> DmxConfig {
        critical_section::with(|cs| {
            let s = self.settings.borrow(cs).borrow();
            DmxConfig::new(s.dmx_base_address, s.universe, s.sacn_port)
                .expect("stored dmx config was corrupted")
        })
    }

    pub fn read_wifi_config(&self) -> WifiConfig {
        critical_section::with(|cs| {
            let s = self.settings.borrow(cs).borrow();
            let ssid_len = s.ssid_len as usize;
            let pwd_len = s.password_len as usize;
            let ssid = core::str::from_utf8(&s.ssid[..ssid_len])
                .unwrap_or(DEFAULT_SSID)
                .into();
            let password = core::str::from_utf8(&s.password[..pwd_len])
                .unwrap_or(DEFAULT_PASSWORD)
                .into();
            WifiConfig::new(ssid, password).expect("stored wifi config was corrupted")
        })
    }

    pub fn write_dmx_config(&self, config: DmxConfig) -> Result<(), FlashStorageError> {
        critical_section::with(|cs| {
            let mut s = self.settings.borrow(cs).borrow_mut();
            s.dmx_base_address = config.address();
            s.universe = config.universe();
            s.sacn_port = config.sacn_port();
        });
        self.flush()
    }

    pub fn write_wifi_config(&self, config: &WifiConfig) -> Result<(), FlashStorageError> {
        critical_section::with(|cs| {
            let mut s = self.settings.borrow(cs).borrow_mut();
            let ssid = config.ssid();
            let password = config.password();
            s.ssid_len = ssid.len() as u8;
            s.ssid[..ssid.len()].copy_from_slice(ssid.as_bytes());
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
            rec.0[104..106].copy_from_slice(&s.universe.to_le_bytes());
            rec.0[106..108].copy_from_slice(&s.sacn_port.to_le_bytes());
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
