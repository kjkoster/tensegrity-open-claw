//! BLE bridge personality — 7E/EF RGBW fixture, two dialects.
//!
//! Turns a 6-channel DMX value (Intensity, R, G, B, White, Gobo rotation) into the
//! fixture's 9-byte BLE write frames and owns the BLE connection lifecycle. It is
//! the BLE counterpart to `led_fixture::run`: a second consumer of the same
//! `DMX_VALUE`, running concurrently with the PWM personality.
//!
//! ## Two dialects (`models::Dialect`)
//!
//! Two fixture families share this 7E/EF protocol and are driven by the same code,
//! selected per board in config:
//!   * **LEDBLE** — HM-10 GATT (service 0xFFE0 / char 0xFFE1), has a gobo motor.
//!   * **ELK** (ELK-BLEDOM/-BLEDWM) — service 0xFFF0 / char 0xFFF3, no gobo.
//! A two-fixture bench test (`ble-star/two-stars-1.py`) proved the colour, white and
//! brightness frames are byte-identical across both (byte1 is don't-care), so only
//! three things branch on the dialect: the GATT UUIDs discovered, the power-frame
//! bytes (`led_power`), and whether the gobo frames are emitted at all (`has_gobo`).
//!
//! The transport (the `transport` submodule) is a `trouble-host` GATT central over
//! `esp-radio`'s BLE controller, sharing the radio that `esp-rtos` starts for WiFi
//! (coexistence via esp-radio's `coex` feature). The writable endpoint is discovered
//! by UUID (per dialect), since its ATT handle is discovery-order dependent.
//!
//! ## The fixture is modal and has no readback
//!
//! The RGB emitters and the white LED cannot light together, so White > 0 overrides
//! RGB (interlocked white). Dimming is hybrid: in RGB mode the native brightness
//! command is the master dimmer (so RGB stays full-resolution), while in white mode
//! that command is dead, so the dimmer is folded into the grayscale level in
//! software. With no readback we re-assert state
//! defensively: a change sends only the frames whose bytes differ from the set we
//! last sent, while the 10 s heartbeat and every reconnect re-send the *complete*
//! frame set. The delta is self-completing across mode flips — an off→on change
//! expands to the full set on its own (no lit frame equals the lone power-off frame)
//! — so a look that goes idle has been fully asserted, and the heartbeat then guards
//! it against a later dropped frame.

use embassy_time::Duration;

use crate::models::{Dialect, DmxValue};

/// Top of the fixture's native brightness scale (the device's 0x64 = 100). In RGB
/// mode this command is the master dimmer; in white mode it is dead, so it is pinned
/// here and the dimmer is folded into the grayscale level instead.
const BLE_BRIGHTNESS_MAX: u8 = 0x64;

/// Top of the fixture's gobo-speed scale (the device's 0x0a = 10). The minimum live
/// speed is 1; rotation 0 means "motor off" and is sent as a power frame, not a
/// speed frame.
const BLE_GOBO_SPEED_MAX: u8 = 0x0a;

/// Re-assert the full state on this period even when nothing changes.
const HEARTBEAT: Duration = Duration::from_secs(10);

/// Pause after a connect/write failure before reconnecting.
const RECONNECT_PAUSE: Duration = Duration::from_secs(2);

/// Most frames in one full-state assertion: power + brightness + colour + gobo
/// power + gobo speed.
const MAX_FRAMES: usize = 5;

type Frame = [u8; 9];
type FrameSet = heapless::Vec<Frame, MAX_FRAMES>;

// ── Frame builders ─────────────────────────────────────────────────────────────
//
// Every command is a fixed 9-byte frame with this skeleton:
//
//     offset 0   header   always 0x7e
//     offset 1   byte1    0xff ("length", ignored) for most actions; the gobo
//                         selector for gobo actions
//     offset 2   action   which command: 0x01 brightness, 0x04 power,
//                         0x05 set-colour, 0x16 gobo-speed
//     offset 3   value    action-dependent; for set-colour it selects the sub-mode
//     offset 4-6 data     action-dependent payload, zero-padded when unused
//     offset 7   led      LED selector for colour/power (0x00 = all LEDs); folded
//                         into the data for actions that have no selector
//     offset 8   footer   always 0xef
//
// All values are hardcoded from on-device capture of the 7E/EF protocol. Each
// builder's doc lists every byte, its meaning and its valid range, so a frame can be
// changed without re-sniffing; bytes shown as fixed padding are ignored by the
// fixture. Only the genuinely variable bytes are parameters.

/// RGB colour, set-colour sub-mode 0x03. The fixture is modal, so white must be off
/// for RGB to show.
///
///     0x7e   header
///     0xff   byte1: length, ignored
///     0x05   action: set colour
///     0x03   value: RGB sub-mode
///     r      red    0x00..=0xff
///     g      green  0x00..=0xff
///     b      blue   0x00..=0xff
///     0x00   LED selector: 0x00 = all LEDs
///     0xef   footer
fn rgb(r: u8, g: u8, b: u8) -> Frame {
    [0x7e, 0xff, 0x05, 0x03, r, g, b, 0x00, 0xef]
}

/// Pure white, set-colour sub-mode 0x01: the dedicated white LED, gradable and
/// cast-free (the RGB emitters stay dark).
///
///     0x7e        header
///     0xff        byte1: length, ignored
///     0x05        action: set colour
///     0x01        value: white (grayscale) sub-mode
///     level       white level 0x00..=0x64 (0..=100)
///     0xff 0xff   data: fixed padding, ignored
///     0x08        LED-selector slot: carries 0x08 in this mode and is ignored
///     0xef        footer
fn white(level: u8) -> Frame {
    [0x7e, 0xff, 0x05, 0x01, level, 0xff, 0xff, 0x08, 0xef]
}

/// Native LED brightness, action 0x01. Works in RGB mode (where it is our master
/// dimmer) but is dead in white mode (where it is pinned to `BLE_BRIGHTNESS_MAX` and
/// the dimmer is folded into the grayscale level instead).
///
///     0x7e        header
///     0xff        byte1: length, ignored
///     0x01        action: brightness
///     level       brightness 0x01..=0x64 (1..=100)
///     0x00        data
///     0xff 0xff   data
///     0x00        LED selector: 0x00 = all LEDs
///     0xef        footer
fn brightness(level: u8) -> Frame {
    [0x7e, 0xff, 0x01, level, 0x00, 0xff, 0xff, 0x00, 0xef]
}

/// LED power, action 0x04. The one frame whose bytes genuinely differ between the
/// dialects (the bench test proved colour/white/brightness do not), so it is the
/// only builder that takes the dialect.
///
/// LEDBLE — target 0x00 = LED, on/off in the value byte. Powering the LED off also
/// stops the gobo motor (a hardware coupling).
///
///     0x7e        header
///     0xff        byte1: length, ignored
///     0x04        action: power
///     on          value: 0x01 = on, 0x00 = off
///     0x00        target: 0x00 = LED
///     0x00 0x00   data
///     0x00        LED selector: 0x00 = all LEDs
///     0xef        footer
///
/// ELK — captured as two fixed frames; the on frame carries 0xF0/0x01/0xFF rather
/// than a plain on/off value byte, so it cannot share LEDBLE's `on as u8` form.
///
///     on    0x7e 0x00 0x04 0xf0 0x00 0x01 0xff 0x00 0xef
///     off   0x7e 0x00 0x04 0x00 0x00 0x00 0xff 0x00 0xef
fn led_power(on: bool, dialect: Dialect) -> Frame {
    match (dialect, on) {
        (Dialect::Ledble, on) => [0x7e, 0xff, 0x04, on as u8, 0x00, 0x00, 0x00, 0x00, 0xef],
        (Dialect::Elk, true) => [0x7e, 0x00, 0x04, 0xf0, 0x00, 0x01, 0xff, 0x00, 0xef],
        (Dialect::Elk, false) => [0x7e, 0x00, 0x04, 0x00, 0x00, 0x00, 0xff, 0x00, 0xef],
    }
}

/// Gobo motor power, action 0x04 with target 0x02 = gobo motor.
///
///     0x7e        header
///     0xff        byte1: length, ignored
///     0x04        action: power
///     on          value: 0x01 = on, 0x00 = off
///     0x02        target: 0x02 = gobo motor
///     0x00 0x00   data
///     0x00        LED selector: 0x00 = all LEDs
///     0xef        footer
fn gobo_power(on: bool) -> Frame {
    [0x7e, 0xff, 0x04, on as u8, 0x02, 0x00, 0x00, 0x00, 0xef]
}

/// Gobo motor speed, action 0x16.
///
///     0x7e             header
///     0x00             byte1: gobo selector, fixed at meteor (0x00). This fixture
///                      ignores gobo selection, which is also why there is no DMX
///                      gobo-select channel.
///     0x16             action: gobo speed
///     speed            motor speed 0x01..=0x0a (1..=BLE_GOBO_SPEED_MAX)
///     0x00 0x00 0x00   data / LED-selector slot, all ignored here
///     0xef             footer
fn gobo_speed(speed: u8) -> Frame {
    [0x7e, 0x00, 0x16, speed, 0x00, 0x00, 0x00, 0x00, 0xef]
}

// ── Dialect properties ───────────────────────────────────────────────────────────
//
// The three points where the two fixture families diverge (the bench test proved
// colour/white/brightness do not): the power-frame bytes live in `led_power` above;
// gobo presence and GATT layout are here.

/// Whether the dialect's fixture has a gobo motor. ELK fixtures have none, so their
/// gobo frames are suppressed entirely (the gobo DMX channel is ignored for them).
fn has_gobo(dialect: Dialect) -> bool {
    matches!(dialect, Dialect::Ledble)
}

/// The dialect's GATT layout: `(service_uuid, write_char_uuid)`, both 16-bit SIG
/// short UUIDs discovered by value. LEDBLE is the HM-10 serial service; ELK is the
/// ELK-BLEDOM 0xFFF0 service.
fn dialect_uuids(dialect: Dialect) -> (u16, u16) {
    match dialect {
        Dialect::Ledble => (0xFFE0, 0xFFE1),
        Dialect::Elk => (0xFFF0, 0xFFF3),
    }
}

// ── Scaling helpers ────────────────────────────────────────────────────────────

/// DMX dimmer (1..=255) mapped onto the fixture's native brightness scale
/// (1..=`BLE_BRIGHTNESS_MAX` = 0x01..=0x64). In RGB mode this *is* the master dimmer,
/// so the RGB bytes can be sent at full 8-bit resolution instead of being scaled down
/// — the native command does the dimming, which avoids crushing colour into a handful
/// of levels at low intensity. Only called while lit (`dimmer >= 1`).
fn intensity_to_brightness(dimmer: u8) -> u8 {
    let level = (dimmer as u32 * BLE_BRIGHTNESS_MAX as u32 + 127) / 255;
    (level as u8).clamp(1, BLE_BRIGHTNESS_MAX)
}

/// DMX White (0..=255) folded with the dimmer into the fixture's 0..=100 grayscale
/// level, as a single rounded conversion. (The DMX-BLE.md draft scaled twice and
/// truncated, banding the low end; this does it once.)
fn white_level(white_ch: u8, dimmer: u8) -> u8 {
    let scaled = (white_ch as u32 * dimmer as u32 * 100 + (255 * 255 / 2)) / (255 * 255);
    (scaled as u8).min(100)
}

/// A non-zero gobo-rotation channel (1..=255) mapped onto motor speed
/// 1..=`BLE_GOBO_SPEED_MAX`, rounded. Rotation 0 means "motor off" and is handled by
/// the caller, so this is only ever called with `rotation >= 1`.
fn rotation_to_speed(rotation: u8) -> u8 {
    // 1..=MAX has (MAX - 1) steps above the minimum; +127 rounds the /254 division.
    let steps = (BLE_GOBO_SPEED_MAX - 1) as u32;
    let speed = 1 + ((rotation as u32 - 1) * steps + 127) / 254;
    (speed as u8).clamp(1, BLE_GOBO_SPEED_MAX)
}

// ── Translation ────────────────────────────────────────────────────────────────

/// Build the full set of frames asserting `val` on the fixture. Modal: exactly one
/// colour frame, white interlocked over RGB. Send the frames in order, each as an
/// acknowledged write.
fn build_frames(val: &DmxValue, dialect: Dialect) -> FrameSet {
    let dimmer = val.intensity();
    let white_ch = val.white();
    let grot = val.gobo();

    let mut frames = FrameSet::new();

    let led_on = dimmer > 0;
    let _ = frames.push(led_power(led_on, dialect));

    if led_on {
        // Modal, with the dimmer applied differently per mode (hybrid dimming):
        //   RGB   — the native brightness command works, so use it as the master
        //           dimmer and send RGB at full 8-bit. Colour keeps all its levels
        //           even at low intensity, instead of collapsing into 0..dimmer.
        //   white — the native brightness command is dead, so pin it to max and bake
        //           the dimmer into the grayscale level in software.
        if white_ch > 0 {
            let _ = frames.push(brightness(BLE_BRIGHTNESS_MAX));
            let _ = frames.push(white(white_level(white_ch, dimmer)));
        } else {
            let _ = frames.push(brightness(intensity_to_brightness(dimmer)));
            let _ = frames.push(rgb(val.red(), val.green(), val.blue()));
        }

        // Gobo motor (only fixtures that have one — ELK has none). On only while
        // rotating; the LED is on here. Turning the LED off also stops the motor, so
        // the dimmer==0 path needs no gobo-off frame.
        if has_gobo(dialect) {
            if grot > 0 {
                let _ = frames.push(gobo_power(true));
                let _ = frames.push(gobo_speed(rotation_to_speed(grot)));
            } else {
                let _ = frames.push(gobo_power(false));
            }
        }
    }

    frames
}

// ── BLE transport (feature `ble`) ────────────────────────────────────────────────

pub use transport::run;

mod transport {
    use bt_hci::controller::{Controller, ExternalController};
    use embassy_futures::select::{Either, Either3, select, select3};
    use embassy_time::{Duration, Instant, Ticker, Timer, with_timeout};
    use esp_hal::peripherals::BT;
    use esp_radio::ble::controller::BleConnector;
    use rtt_target::rprintln;
    use trouble_host::prelude::*;

    use super::{HEARTBEAT, RECONNECT_PAUSE, build_frames, dialect_uuids};
    use crate::models::{BleTarget, Dialect, DmxReceiver, DmxValue};

    // HCI command slots held on the controller side.
    const HCI_SLOTS: usize = 20;
    // One outbound (central) connection; no peripheral role.
    const CONNECTIONS_MAX: usize = 1;
    const L2CAP_CHANNELS_MAX: usize = 2;
    // Services cached during discovery.
    const GATT_MAX_SERVICES: usize = 4;

    // Bound the GATT setup and discovery so a stalled handshake reconnects instead of
    // hanging forever. After an ESP reset (e.g. a re-flash) the fixture can still be
    // holding the pre-reset ACL link and never answers the new connection's GATT
    // setup — the symptom is sitting past "ble: connected" until a power cycle. On
    // timeout we drop the half-open connection and retry; dropping it frees the single
    // connection slot, and the fixture's own supervision timeout eventually releases
    // the stale link, so it now self-heals without the power cycle.
    const SETUP_TIMEOUT: Duration = Duration::from_secs(10);

    // Bound the scan-and-connect too: with a filter-accept-list, `connect()` scans
    // forever while the fixture is absent or the radio is wedged. A generous timeout
    // recovers a stuck scan and keeps the loop visible, without thrashing when the
    // fixture is simply powered off.
    const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

    // Link supervision timeout we request. This is the connection-wide value both ends
    // use to declare the link dead, so it also bounds how long the fixture keeps a
    // ghost link alive after *we* vanish (e.g. an ESP reset). Shorter ⇒ the fixture
    // frees the stale link — and accepts us again — sooner; too short risks dropping on
    // brief interference. The fixture is close and strong here (RSSI ≈ -40), so 4 s is
    // a safe trade. Tune if reconnects still lag a reset.
    const SUPERVISION_TIMEOUT: Duration = Duration::from_secs(4);

    // Bound each acknowledged write. The await completes on the fixture's ATT Write
    // Response, so if the peer goes silent — link nominally up, no Disconnected, but no
    // response coming — the await would block forever, the serve loop emits nothing, and
    // the link is never torn down (ble/s flatlines at 0 with no log). A timeout converts
    // that wedge into a return from serve() that the reconnect flow already handles. Kept
    // above SUPERVISION_TIMEOUT (4 s) so a clean link drop is caught by supervision first
    // and a merely slow fixture is not torn down prematurely.
    const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

    // Connection interval we request from the fixture (#2). trouble-host's default is a
    // slow, power-saving interval; a tight 7.5–15 ms window lets brain's frame stream
    // actually reach the fixture instead of being resampled down to the link rate. The
    // fixture negotiates the final value — watch the existing `ble/s` metric for the
    // rate that was actually granted, since trouble-host 0.6 does not surface the
    // negotiated interval through its safe API.
    const CONN_INTERVAL_MIN: Duration = Duration::from_micros(7_500);
    const CONN_INTERVAL_MAX: Duration = Duration::from_millis(15);

    // The fixture's GATT layout is dialect-specific and lives in `super::dialect_uuids`
    // (LEDBLE's HM-10 0xFFE0/0xFFE1 — its value handle is the 0x0011 seen in the 7E/EF
    // write captures — vs ELK's 0xFFF0/0xFFF3). Both are 16-bit (Bluetooth SIG short)
    // UUIDs, so discovery uses `Uuid::new_short`.

    /// BLE consumer — the counterpart to `led_fixture::run`. Brings up the host, then
    /// forever: connect to `target`, discover the writable characteristic, resync the
    /// full state, and re-assert it on every change and on the heartbeat until the link
    /// drops — then rebuild the controller and reconnect. Never returns.
    ///
    /// The controller and host stack are rebuilt from scratch on **every** session, not
    /// reused across reconnects. A wedged link keeps exchanging empty LL PDUs, so neither
    /// side's supervision timeout ever fires and the fixture never re-advertises; a
    /// reconnect on the same controller can therefore never find it again (the
    /// `ble/s`-flatlines-at-0 failure). Dropping the connector/controller/stack at the end
    /// of each session releases the radio entirely — we go silent on the air, the
    /// fixture's supervision finally fires, and it re-advertises in time for the fresh
    /// controller's next connect.
    pub async fn run(mut dmx_value: DmxReceiver, mut bt: BT<'static>, target: BleTarget) -> ! {
        let mac = target.mac();
        let dialect = target.dialect();
        // Address kind is Public, confirmed by an ADV_IND sniff (PDU TxAdd: Public).
        // bt-hci's BdAddr is written straight into the HCI command, and HCI carries
        // BD_ADDR little-endian (LSB first) — the reverse of the human-readable order
        // held in config. Reverse here, or the filter-accept-list entry never matches
        // the advertiser and connect() scans forever.
        let mut addr = mac;
        addr.reverse();
        let peer = Address {
            kind: AddrKind::PUBLIC,
            addr: BdAddr::new(addr),
        };

        loop {
            // Fresh controller + host per session (see the fn doc). `reborrow` hands the
            // BT peripheral to this session's connector; dropping it at the bottom of the
            // loop returns the radio so the next session starts from a clean controller.
            let connector = BleConnector::new(bt.reborrow(), Default::default()).unwrap();
            let controller: ExternalController<_, HCI_SLOTS> = ExternalController::new(connector);

            let mut resources: HostResources<
                DefaultPacketPool,
                CONNECTIONS_MAX,
                L2CAP_CHANNELS_MAX,
            > = HostResources::new();
            let stack = trouble_host::new(controller, &mut resources);
            let Host {
                mut central,
                mut runner,
                ..
            } = stack.build();

            // One connection attempt and serve session. Returns (ending the session) on
            // any teardown — connect/gatt failure, a disconnect, or a write that stalls
            // past WRITE_TIMEOUT — so the outer loop drops the controller and rebuilds.
            let session = async {
                rprintln!(
                    "ble connecting: {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
                    mac[0],
                    mac[1],
                    mac[2],
                    mac[3],
                    mac[4],
                    mac[5]
                );
                rprintln!(
                    "ble: requesting conn interval {}-{} us, latency 0",
                    CONN_INTERVAL_MIN.as_micros(),
                    CONN_INTERVAL_MAX.as_micros(),
                );
                let config = ConnectConfig {
                    connect_params: RequestedConnParams {
                        min_connection_interval: CONN_INTERVAL_MIN,
                        max_connection_interval: CONN_INTERVAL_MAX,
                        supervision_timeout: SUPERVISION_TIMEOUT,
                        ..Default::default()
                    },
                    scan_config: ScanConfig {
                        filter_accept_list: &[(peer.kind, &peer.addr)],
                        ..Default::default()
                    },
                };

                let conn = match with_timeout(CONNECT_TIMEOUT, central.connect(&config)).await {
                    Ok(Ok(conn)) => conn,
                    Ok(Err(e)) => {
                        rprintln!("ble: connect failed: {:?}", e);
                        return;
                    }
                    Err(_) => {
                        rprintln!("ble: no connection within {} s", CONNECT_TIMEOUT.as_secs());
                        return;
                    }
                };
                rprintln!("ble: connected");

                // The interval actually in force (#2), read straight from the link.
                // The fixture accepts our request at connect time and never runs a
                // later parameter-update procedure, so this is the only place it
                // surfaces — the ConnectionParamsUpdated event below stays silent.
                let p = conn.params();
                let interval_us = p.conn_interval.as_micros();
                // One connection event per interval bounds how often the fixture sees a
                // new frame, so this is the ceiling on update rate the link permits.
                let max_updates_hz = if interval_us > 0 {
                    1_000_000u64 / interval_us
                } else {
                    0
                };
                rprintln!(
                    "ble: conn params in force: interval {} us (~{} updates/s max), latency {}, supervision {} us",
                    interval_us,
                    max_updates_hz,
                    p.peripheral_latency,
                    p.supervision_timeout.as_micros(),
                );

                let client = match with_timeout(
                    SETUP_TIMEOUT,
                    GattClient::<_, DefaultPacketPool, GATT_MAX_SERVICES>::new(&stack, &conn),
                )
                .await
                {
                    Ok(Ok(client)) => client,
                    Ok(Err(e)) => {
                        rprintln!("ble: gatt setup failed: {:?}", e);
                        conn.disconnect();
                        return;
                    }
                    Err(_) => {
                        rprintln!(
                            "ble: gatt setup stalled past {} s",
                            SETUP_TIMEOUT.as_secs()
                        );
                        conn.disconnect();
                        return;
                    }
                };
                rprintln!("ble: gatt client ready");

                // Watch three things at once:
                //   1. the GATT client background task (must be polled while we write),
                //   2. serve() — our writes and discovery; returns on a write/discovery
                //      error, and now also on a write that stalls past WRITE_TIMEOUT,
                //   3. the connection itself. A clean supervision-timeout drop (the
                //      fixture loses power or goes out of range) surfaces as a
                //      Disconnected event. With acknowledged writes an in-flight write to
                //      a dropped link also errors out of serve() (arm #2), so a drop can
                //      surface either way and select3 takes whichever fires first. But not
                //      every failure surfaces as a Disconnected: a GATT/buffer wedge can
                //      leave the ACL link nominally up while the fixture stops responding
                //      (observed as ble/s flatlining at 0 with no log). That case is caught
                //      by the WRITE_TIMEOUT on serve()'s write path (arm #2), not here.
                let wait_disconnect = async {
                    loop {
                        match conn.next().await {
                            ConnectionEvent::Disconnected { reason } => break reason,
                            // The negotiated link parameters (#2): the fixture usually
                            // requests its own interval shortly after connecting, and
                            // this is where the value actually in force shows up.
                            ConnectionEvent::ConnectionParamsUpdated {
                                conn_interval,
                                peripheral_latency,
                                supervision_timeout,
                            } => rprintln!(
                                "ble: conn params negotiated: interval {} us, latency {}, supervision {} us",
                                conn_interval.as_micros(),
                                peripheral_latency,
                                supervision_timeout.as_micros(),
                            ),
                            other => rprintln!("ble: unhandled connection event: {:?}", other),
                        }
                    }
                };
                match select3(
                    client.task(),
                    serve(&client, &mut dmx_value, dialect),
                    wait_disconnect,
                )
                .await
                {
                    Either3::First(e) => rprintln!("ble: gatt task ended: {:?}", e),
                    Either3::Second(Err(e)) => rprintln!("ble: link error: {:?}", e),
                    Either3::Second(Ok(())) => rprintln!("ble: serve loop ended"),
                    Either3::Third(reason) => rprintln!("ble: disconnected: {:?}", reason),
                }
                // Best-effort clean terminate; on a wedged link this may not land, which
                // is exactly why the outer loop rebuilds the controller regardless.
                conn.disconnect();
            };

            // The host runner must be polled continuously while we use the central role.
            // If it exits we still fall through and rebuild rather than panicking.
            match select(runner.run(), session).await {
                Either::First(e) => rprintln!("ble: host runner exited: {:?} — rebuilding", e),
                Either::Second(()) => {}
            }

            // Dropping connector/controller/stack here releases the radio; the pause then
            // lets the fixture's supervision free the stale link and re-advertise.
            rprintln!("ble: reconnecting in {} s", RECONNECT_PAUSE.as_secs());
            Timer::after(RECONNECT_PAUSE).await;
        }
    }

    /// Discover the writable characteristic, resync the full state, then send a
    /// per-change delta with a full re-assert on the heartbeat. Returns when the link
    /// drops or errors.
    async fn serve<C: Controller>(
        client: &GattClient<'_, C, DefaultPacketPool, GATT_MAX_SERVICES>,
        dmx_value: &mut DmxReceiver,
        dialect: Dialect,
    ) -> Result<(), BleHostError<C::Error>> {
        let (service_uuid, write_char_uuid) = dialect_uuids(dialect);
        rprintln!("ble: discovering service 0x{:04X}", service_uuid);
        let services = match with_timeout(
            SETUP_TIMEOUT,
            client.services_by_uuid(&Uuid::new_short(service_uuid)),
        )
        .await
        {
            Ok(result) => result?,
            Err(_) => {
                rprintln!("ble: service discovery stalled — reconnecting");
                return Ok(());
            }
        };
        let Some(service) = services.first().cloned() else {
            rprintln!("ble: write service not found");
            return Ok(());
        };
        let write_char: Characteristic<u8> = match with_timeout(
            SETUP_TIMEOUT,
            client.characteristic_by_uuid(&service, &Uuid::new_short(write_char_uuid)),
        )
        .await
        {
            Ok(result) => result?,
            Err(_) => {
                rprintln!("ble: characteristic discovery stalled — reconnecting");
                return Ok(());
            }
        };
        rprintln!("ble: write characteristic 0x{:04X} found", write_char_uuid);

        // Resync to the latest known DMX value, not blackout: on a reconnect the
        // fixture should snap straight back to the current look instead of flashing
        // dark until the next change or heartbeat. Falls back to all-zero only before
        // the first sACN frame has arrived. The logline below makes which path we took
        // visible on the wire, so a reconnect blackout would show up as "no DMX yet"
        // appearing after frames have been flowing.
        let mut current = match dmx_value.try_get() {
            Some(value) => {
                rprintln!("ble: resyncing to current DMX state");
                value
            }
            None => {
                rprintln!("ble: resyncing to blackout (no DMX yet)");
                DmxValue::new([0; DmxValue::LEN])
            }
        };
        let mut last_frames = build_frames(&current, dialect);
        if !send_all(client, &write_char, &last_frames).await? {
            rprintln!("ble: write stalled during resync — reconnecting");
            return Ok(());
        }
        rprintln!("ble: resynced, serving");

        // Fresh ticker per connection so the heartbeat phase restarts on resync.
        let mut tick = Ticker::every(HEARTBEAT);
        loop {
            match select(dmx_value.changed(), tick.next()).await {
                Either::First(value) => {
                    // A change asserts only the frames whose bytes differ from the set
                    // we last sent. The delta is self-completing across mode flips:
                    // off→on expands to the full set (no lit frame equals the lone
                    // power-off frame), on→off collapses to just the power-off frame,
                    // and an off→off change sends nothing.
                    current = value;
                    let frames = build_frames(&current, dialect);
                    for frame in frames.iter().filter(|f| !last_frames.contains(*f)) {
                        if !write_frame(client, &write_char, frame).await? {
                            rprintln!("ble: write stalled — reconnecting");
                            return Ok(());
                        }
                    }
                    last_frames = frames;
                }
                Either::Second(()) => {
                    // Heartbeat: re-assert the whole state. With no readback this is the
                    // only correction for a static look, where no later change is coming
                    // to heal a dropped frame.
                    last_frames = build_frames(&current, dialect);
                    if !send_all(client, &write_char, &last_frames).await? {
                        rprintln!("ble: write stalled on heartbeat — reconnecting");
                        return Ok(());
                    }
                }
            }
        }
    }

    /// Assert a complete frame set — every frame, in order. Used on resync and on the
    /// heartbeat. Returns Ok(false) if a write stalled (caller reconnects).
    async fn send_all<C: Controller>(
        client: &GattClient<'_, C, DefaultPacketPool, GATT_MAX_SERVICES>,
        write_char: &Characteristic<u8>,
        frames: &super::FrameSet,
    ) -> Result<bool, BleHostError<C::Error>> {
        for frame in frames {
            if !write_frame(client, write_char, frame).await? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Write one 9-byte frame as an acknowledged write and record it with its latency.
    /// The await completes only when the fixture returns the ATT Write Response, so its
    /// duration is the fixture's true per-command service time — link round-trip plus
    /// however long its MCU takes to digest the frame. Timing it is how we measure the
    /// real throughput of an otherwise-opaque controller, and the ack also paces the
    /// serve loop to exactly what the fixture can absorb (no overrun). The await is
    /// bounded by WRITE_TIMEOUT. Returns Ok(true) on a counted write, Ok(false) if the
    /// write stalled past the timeout (the caller should return from serve to
    /// reconnect), and Err for a real link/controller error.
    async fn write_frame<C: Controller>(
        client: &GattClient<'_, C, DefaultPacketPool, GATT_MAX_SERVICES>,
        write_char: &Characteristic<u8>,
        frame: &super::Frame,
    ) -> Result<bool, BleHostError<C::Error>> {
        let started = Instant::now();
        match with_timeout(WRITE_TIMEOUT, client.write_characteristic(write_char, frame)).await {
            Ok(result) => {
                result?;
                crate::metrics::record_ble_packet(started.elapsed().as_micros() as u32);
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }
}
