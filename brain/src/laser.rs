//! JB Systems Space-4 laser mapping (LASER.md): drives the wired laser's 8 DMX channels.
//! The pattern is held static and CH7/CH8 (absolute X/Y position) are swept in lockstep
//! along the main diagonal, so the line is traced by positioning alone — no galvo scanning.

use crate::config as cfg;

/// The laser's eight channel values, in CH1..CH8 order.
pub struct LaserOut {
    pub channels: [u8; cfg::LASER_CHANNELS],
}

#[derive(Default)]
pub struct LaserMapping {
    phase: f64,      // position-sweep phase (frozen while LASER_POS_MIN == LASER_POS_MAX)
    mode_phase: f64, // TEMP diagnostic: 0..1 CH1 mode-sweep phase
    log_accum: f64,  // TEMP diagnostic: seconds since the last log
}

impl LaserMapping {
    pub fn frame(&mut self, dt: f64) -> LaserOut {
        self.phase = (self.phase + dt / cfg::LASER_SWEEP_PERIOD_S).rem_euclid(1.0);
        let tri = 1.0 - (2.0 * self.phase - 1.0).abs();
        let span = (cfg::LASER_POS_MAX - cfg::LASER_POS_MIN) as f64;
        let pos = cfg::LASER_POS_MIN + (tri * span).round() as u8; // frozen at 60 (MIN == MAX)

        // TEMPORARY diagnostic: sweep CH1 (mode) across 0..255, logging once a second, with
        // every other channel static. Watch for the laser to CHANGE — go dark in the 0–63
        // blackout band, or freeze on the held position — which proves CH1 is read and reveals
        // the real DMX-mode gate. If it animates identically across the whole range, CH1 is not
        // honoured at all and the fault is framing, not values.
        self.mode_phase = (self.mode_phase + dt / cfg::LASER_MODE_SWEEP_PERIOD_S).rem_euclid(1.0);
        let mode = (self.mode_phase * 255.0).round() as u8;
        self.log_accum += dt;
        if self.log_accum >= 1.0 {
            self.log_accum -= 1.0;
            eprintln!("laser: CH1 mode sweep = {mode}");
        }

        let mut channels = [0u8; cfg::LASER_CHANNELS];
        channels[0] = mode; // CH1: TEMP sweep (was cfg::LASER_DMX_MODE)
        channels[1] = cfg::LASER_PATTERN; // CH2 pattern
        channels[2] = cfg::LASER_ZOOM; // CH3 zoom
        channels[6] = pos; // CH7 X position (frozen at 60)
        channels[7] = pos; // CH8 Y position (frozen at 60)
        LaserOut { channels }
    }
}
