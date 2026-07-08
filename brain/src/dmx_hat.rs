//! Wired DMX-512 sink (HARDWARE-DMX.md): clocks the universe out `/dev/serial0` on the
//! Zihatec RS-485 HAT as raw DMX — BREAK + MAB + 0x00 start code + slots. This mirrors the
//! sACN frame onto the wire; sACN stays the WiFi-only transport. GPIO18 is held HIGH
//! (transmit) from boot via `gpio=18=op,dh`, so nothing here touches the RS-485 direction.

use crate::config as cfg;
use std::fs;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::{AsRawFd, RawFd};
use std::time::{Duration, Instant};

pub struct DmxHat {
    port: File,
}

impl DmxHat {
    /// Opens and configures the HAT's serial port for DMX. Panics with a remediation
    /// message if the OS side is not set up — a deploy-time gate, not a runtime hazard:
    /// once `/dev/serial0` points at the PL011 and no getty holds it, `open` always succeeds.
    pub fn open() -> DmxHat {
        preflight();
        let port = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOCTTY)
            .open(cfg::SERIAL_DEVICE)
            .unwrap_or_else(|e| panic!("dmx_hat: cannot open {}: {e}", cfg::SERIAL_DEVICE));
        configure_250k_8n2(port.as_raw_fd());
        eprintln!("dmx_hat: {} → 250000 baud 8N2, mirroring universe {}", cfg::SERIAL_DEVICE, cfg::UNIVERSE);
        DmxHat { port }
    }

    /// Clocks one DMX frame: flush the previous frame, generate the BREAK/MAB, then send
    /// the 0x00 start code and the slots. A write error is logged, not fatal — a yanked
    /// cable must not take the sculpture down.
    pub fn send(&self, slots: &[u8]) {
        let fd = self.port.as_raw_fd();
        // SAFETY: fd is owned by self.port and valid for the call.
        unsafe { libc::tcdrain(fd) };
        unsafe { libc::ioctl(fd, libc::TIOCSBRK as libc::c_ulong) };
        spin(cfg::DMX_BREAK_US);
        unsafe { libc::ioctl(fd, libc::TIOCCBRK as libc::c_ulong) };
        spin(cfg::DMX_MAB_US);

        let mut frame = Vec::with_capacity(1 + cfg::WIRED_FRAME_SLOTS);
        frame.push(0x00); // DMX start code
        frame.extend_from_slice(slots);
        // Pad to a full 512-slot universe (zeros past the live slots): picky fixtures
        // mis-decode short frames.
        frame.resize(1 + cfg::WIRED_FRAME_SLOTS, 0);
        // `impl Write for &File` — send the start code and slots in one UART burst.
        let mut port = &self.port;
        if let Err(e) = port.write_all(&frame) {
            eprintln!("dmx_hat: write error: {e}");
            return;
        }
        unsafe { libc::tcdrain(fd) };
    }
}

/// Busy-waits for at least `us` microseconds. Used for the sub-millisecond BREAK/MAB,
/// where sleeping is too coarse; the spin is cheap at ~100 µs total per 44 Hz frame.
fn spin(us: u64) {
    let start = Instant::now();
    let target = Duration::from_micros(us);
    while start.elapsed() < target {
        std::hint::spin_loop();
    }
}

/// Verifies the *runtime effect* of the OS setup, not `config.txt` (which is only the
/// request — a failed overlay would parse fine yet still be broken).
fn preflight() {
    // `/dev/serial0` must resolve to the PL011 (ttyAMA0). If it points at ttyS0 the
    // mini-UART is in the way (Bluetooth not disabled), whose baud tracks the core clock
    // and cannot hold 250 kbaud.
    match fs::read_link(cfg::SERIAL_DEVICE) {
        Ok(target) if target.file_name().and_then(|s| s.to_str()) == Some("ttyAMA0") => {}
        Ok(target) => panic!(
            "dmx_hat: {} → {} (expected ttyAMA0); add `dtoverlay=disable-bt` to config.txt and reboot",
            cfg::SERIAL_DEVICE,
            target.display()
        ),
        Err(e) => panic!("dmx_hat: cannot read link {}: {e}", cfg::SERIAL_DEVICE),
    }
    // A login getty on the port would inject bytes and corrupt the DMX stream.
    let cmdline = fs::read_to_string("/proc/cmdline").unwrap_or_default();
    if cmdline.contains("console=serial0") || cmdline.contains("console=ttyAMA0") {
        panic!("dmx_hat: kernel console is on the serial port; raspi-config → Serial Port: login shell off");
    }
}

/// Sets 250000 baud, 8N2, raw mode. 250 k is non-standard, so it goes through `termios2`
/// + `BOTHER` (no portable speed constant covers it); `CSTOPB` gives the second stop bit.
fn configure_250k_8n2(fd: RawFd) {
    let mut tio: libc::termios2 = unsafe { std::mem::zeroed() };
    if unsafe { libc::ioctl(fd, libc::TCGETS2 as libc::c_ulong, &mut tio as *mut libc::termios2) } < 0 {
        panic!("dmx_hat: TCGETS2 failed: {}", std::io::Error::last_os_error());
    }
    tio.c_iflag = 0;
    tio.c_oflag = 0;
    tio.c_lflag = 0;
    tio.c_cflag &= !(libc::CBAUD | libc::CSIZE | libc::PARENB);
    tio.c_cflag |= libc::CS8 | libc::CSTOPB | libc::CLOCAL | libc::CREAD | libc::BOTHER;
    tio.c_ispeed = 250_000;
    tio.c_ospeed = 250_000;
    if unsafe { libc::ioctl(fd, libc::TCSETS2 as libc::c_ulong, &tio as *const libc::termios2) } < 0 {
        panic!("dmx_hat: TCSETS2 (250000 8N2) failed: {}", std::io::Error::last_os_error());
    }
}
