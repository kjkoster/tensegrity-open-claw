//! Decimating filter stage between the sACN listener and the BLE consumer.
//!
//! The BLE fixtures' original controllers absorb only a limited rate of DMX change
//! before the link saturates and wedges (ISSUE.md), while the sACN wire and the PWM
//! personality run far faster. This stage resamples the wire down to
//! `config::BLE_UPDATE_RATE_HZ` for the BLE consumer alone: it observes every distinct
//! wire frame, and once per emit window publishes the average of that window onto a
//! second `DmxWatch` that only the BLE bridge reads. Every other consumer keeps full
//! rate.
//!
//! ## Why a separate task, not a filter inside the BLE serve loop
//!
//! The averaging has to see every wire frame. The BLE serve loop spends most of each
//! window blocked on acknowledged writes, and a `Watch` coalesces to its latest value —
//! so a frame that lands mid-write is collapsed away before the loop looks again. Only a
//! task that never blocks on BLE can observe the whole window. This stage does exactly
//! that, leaving the BLE transport code untouched.
//!
//! ## The white/RGB interlock
//!
//! The fixture is modal: white and RGB cannot light together, so brain sends each frame
//! in one mode (white-mode frames carry R=G=B=0, RGB-mode frames carry White=0).
//! Averaging all six channels blindly across a mode flip would blend white into RGB and
//! produce a value that is neither look. Instead the window routes each frame's colour
//! into a per-mode bucket, and the emitted value takes its mode from the *latest* frame —
//! white↔RGB is a hard cut, not something to smear through grey — averaging only the
//! same-mode frames for colour. Intensity and gobo are mode-independent and average over
//! the whole window.

use embassy_futures::select::{Either, select};
use embassy_time::{Duration, Ticker};

use crate::config as cfg;
use crate::metrics;
use crate::models::{DmxReceiver, DmxSender, DmxValue};

/// The wire frames seen within one emit window, reduced on demand to a single
/// interlock-aware average. Colour is kept in two mode-segregated buckets so a
/// cross-mode window never blends white into RGB; intensity and gobo are
/// mode-independent and sum over every frame.
struct Window {
    count: u16,
    sum_intensity: u32,
    sum_gobo: u32,
    rgb_count: u16,
    sum_r: u32,
    sum_g: u32,
    sum_b: u32,
    white_count: u16,
    sum_white: u32,
    latest: DmxValue,
}

impl Window {
    fn new() -> Self {
        Self {
            count: 0,
            sum_intensity: 0,
            sum_gobo: 0,
            rgb_count: 0,
            sum_r: 0,
            sum_g: 0,
            sum_b: 0,
            white_count: 0,
            sum_white: 0,
            latest: DmxValue::new([0; DmxValue::LEN]),
        }
    }

    fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Fold one wire frame in. The frame's mode (white > 0) selects the colour bucket,
    /// so the later average stays within a single mode.
    fn add(&mut self, v: DmxValue) {
        self.count += 1;
        self.sum_intensity += v.intensity() as u32;
        self.sum_gobo += v.gobo() as u32;
        if v.white() > 0 {
            self.white_count += 1;
            self.sum_white += v.white() as u32;
        } else {
            self.rgb_count += 1;
            self.sum_r += v.red() as u32;
            self.sum_g += v.green() as u32;
            self.sum_b += v.blue() as u32;
        }
        self.latest = v;
    }

    /// Reduce the window to one averaged, mode-consistent value. Only ever called on a
    /// non-empty window. The emitted mode follows the latest frame, which also
    /// guarantees the colour divisor is non-zero: the latest frame is in that bucket.
    fn average(&self) -> DmxValue {
        let intensity = mean(self.sum_intensity, self.count);
        let gobo = mean(self.sum_gobo, self.count);
        if self.latest.white() > 0 {
            DmxValue::new([intensity, 0, 0, 0, mean(self.sum_white, self.white_count), gobo])
        } else {
            let r = mean(self.sum_r, self.rgb_count);
            let g = mean(self.sum_g, self.rgb_count);
            let b = mean(self.sum_b, self.rgb_count);
            DmxValue::new([intensity, r, g, b, 0, gobo])
        }
    }
}

/// Rounded integer mean of a channel sum over `count` samples. `count` is always >= 1 at
/// the call sites — empty windows never emit, and the emitted mode's bucket holds at
/// least the latest frame.
fn mean(sum: u32, count: u16) -> u8 {
    let count = count as u32;
    ((sum + count / 2) / count) as u8
}

/// Filter stage: observe every distinct wire frame, and once per emit period publish the
/// interlock-aware average of that window downstream to the BLE consumer. A new average
/// is only sent when it differs from the last, mirroring the listener's own
/// change-suppression, so `changed()` downstream fires on real change and the `filtered`
/// metric counts BLE-facing changes. Never returns.
#[embassy_executor::task]
pub async fn run(mut dmx_in: DmxReceiver, dmx_out: DmxSender) -> ! {
    let period = Duration::from_micros(1_000_000 / cfg::BLE_UPDATE_RATE_HZ);
    let mut ticker = Ticker::every(period);
    let mut window = Window::new();
    let mut last_sent: Option<DmxValue> = None;

    loop {
        match select(dmx_in.changed(), ticker.next()).await {
            Either::First(value) => window.add(value),
            Either::Second(()) => {
                // An empty window means a static look (the listener suppresses identical
                // wire frames): emit nothing and let the BLE side hold, guarded by its
                // heartbeat.
                if !window.is_empty() {
                    let avg = window.average();
                    window = Window::new();
                    if Some(avg) != last_sent {
                        last_sent = Some(avg);
                        dmx_out.send(avg);
                        metrics::record_filtered();
                    }
                }
            }
        }
    }
}
