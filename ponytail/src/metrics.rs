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
// Sum of acknowledged-write latencies (µs) since the last tick. Divided by BLE_PACKETS
// in the report task to get the mean per-write service time. A second's worth (~16
// writes × ~64 ms) is well within u32; only completed writes contribute, so it cannot
// be inflated by a stalled one.
static ACK_MICROS: AtomicU32 = AtomicU32::new(0);

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

/// One acknowledged BLE write completed, with the round-trip latency (µs) the fixture
/// took to ack it. The latency is the fixture's true per-command service time — the
/// report task averages it into the real-throughput figure.
pub fn record_ble_packet(latency_us: u32) {
    BLE_PACKETS.fetch_add(1, Ordering::Relaxed);
    ACK_MICROS.fetch_add(latency_us, Ordering::Relaxed);
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

    // The acked-write latency average seeds on the first second that actually has writes,
    // which is later than the first sample (BLE connects after WiFi), so it carries its
    // own seed flag.
    let mut avg_ack_ms = 0.0f32;
    let mut seeded_ack = false;

    loop {
        ticker.next().await;

        let universes = UNIVERSES.swap(0, Ordering::Relaxed);
        let changes = CHANGES.swap(0, Ordering::Relaxed);
        let ble = BLE_PACKETS.swap(0, Ordering::Relaxed);
        let ack_micros = ACK_MICROS.swap(0, Ordering::Relaxed);

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

        // Mean acked-write latency this second = total / count. Undefined with no writes,
        // so hold the average across an idle second rather than dragging it to zero.
        let ack_ms = if ble > 0 {
            ack_micros as f32 / ble as f32 / 1000.0
        } else {
            0.0
        };
        if ble > 0 {
            if seeded_ack {
                avg_ack_ms += ALPHA * (ack_ms - avg_ack_ms);
            } else {
                avg_ack_ms = ack_ms;
                seeded_ack = true;
            }
        }
        // The real rate the fixture can sustain, implied by that latency: it services one
        // acked command at a time, so capacity ≈ 1000 / mean-ms. This is the number to set
        // brain's change rate just under.
        let ack_rate = if avg_ack_ms > 0.0 { 1000.0 / avg_ack_ms } else { 0.0 };

        rprintln!(
            "metrics: universes {}/s (avg {:.1}) | changes {}/s (avg {:.1}) | ble {}/s (avg {:.1}) | ble/change {:.2} (avg {:.2}) | ack {:.0}ms (avg {:.0}ms, ~{:.0}/s)",
            universes, avg_universes, changes, avg_changes, ble, avg_ble, ratio, avg_ratio, ack_ms, avg_ack_ms, ack_rate
        );
    }
}
