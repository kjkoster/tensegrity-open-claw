//! Throughput telemetry — once-a-second counters and the task that reports them.
//!
//! Two questions drive this: (a) is WiFi delivering sACN reliably, and (b) how many
//! BLE frames are we burning per actual DMX change (the link is the bottleneck, so
//! every redundant frame matters). The producers — the sACN listener and the BLE
//! bridge — just bump lock-free counters on their hot paths; this task owns all the
//! arithmetic and the RTT output so it costs the producers nothing but an atomic add.

use core::sync::atomic::{AtomicU32, Ordering};

use embassy_time::{Duration, Ticker};
use rtt_target::rprintln;

// Events accumulated since the last report tick. Relaxed is sufficient: these are
// plain event tallies with no ordering relationship to other memory, and the only
// reader (the report task) swaps each to zero once a second.
static UNIVERSES: AtomicU32 = AtomicU32::new(0);
static CHANGES: AtomicU32 = AtomicU32::new(0);
static BLE_PACKETS: AtomicU32 = AtomicU32::new(0);

/// A valid sACN packet for our universe arrived (counts delivery, before the
/// change filter — this is the WiFi-reliability signal).
pub fn record_universe() {
    UNIVERSES.fetch_add(1, Ordering::Relaxed);
}

/// The received universe differed from the last one, i.e. a real DMX change that
/// the consumers must act on.
pub fn record_change() {
    CHANGES.fetch_add(1, Ordering::Relaxed);
}

/// One BLE write-without-response frame went out to the fixture.
pub fn record_ble_packet() {
    BLE_PACKETS.fetch_add(1, Ordering::Relaxed);
}

// Per-second EWMA smoothing factor. With one update per second, alpha = 1/30 gives
// the average a ~30 s memory (the most recent ~30 samples dominate), matching the
// "decaying to the last 30 seconds" window we want for trend-spotting.
const ALPHA: f32 = 1.0 / 30.0;

/// Once a second, drain the counters, fold them into ~30 s decaying averages, and
/// print both the instantaneous and smoothed values over RTT. Never returns.
#[embassy_executor::task]
pub async fn report() -> ! {
    let mut ticker = Ticker::every(Duration::from_secs(1));

    // Seed the averages from the first sample rather than letting them crawl up out
    // of zero, so the smoothed column is meaningful from the first line.
    let mut avg_universes = 0.0f32;
    let mut avg_changes = 0.0f32;
    let mut avg_ble = 0.0f32;
    let mut seeded = false;

    loop {
        ticker.next().await;

        let universes = UNIVERSES.swap(0, Ordering::Relaxed);
        let changes = CHANGES.swap(0, Ordering::Relaxed);
        let ble = BLE_PACKETS.swap(0, Ordering::Relaxed);

        if seeded {
            avg_universes += ALPHA * (universes as f32 - avg_universes);
            avg_changes += ALPHA * (changes as f32 - avg_changes);
            avg_ble += ALPHA * (ble as f32 - avg_ble);
        } else {
            avg_universes = universes as f32;
            avg_changes = changes as f32;
            avg_ble = ble as f32;
            seeded = true;
        }

        // Frames-per-change. Guard the zero-change second (no changes ⇒ ratio
        // undefined) rather than print inf/NaN. The decaying ratio is taken from the
        // smoothed counts so a single quiet second doesn't whipsaw it.
        let ratio = if changes > 0 {
            ble as f32 / changes as f32
        } else {
            0.0
        };
        let avg_ratio = if avg_changes > 0.0 {
            avg_ble / avg_changes
        } else {
            0.0
        };

        rprintln!(
            "metrics: universes {}/s (avg {:.1}) | changes {}/s (avg {:.1}) | ble {}/s (avg {:.1}) | ble/change {:.2} (avg {:.2})",
            universes, avg_universes, changes, avg_changes, ble, avg_ble, ratio, avg_ratio
        );
    }
}
