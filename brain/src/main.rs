mod audio;
mod fixture;
mod perlin;
mod sacn;

use embassy_executor::Spawner;
use embassy_time::{Duration, Ticker};
use fixture::Fixture;
use perlin::{noise1d, to_dmx};
use std::net::UdpSocket;

// ── Deployment config ────────────────────────────────────────────────────────
const UNIVERSE: u16 = 1;
const SACN_PORT: u16 = 5568;
const FRAME_RATE_HZ: u64 = 44;

// ── Noise engine config ──────────────────────────────────────────────────────
const NOISE_SPEED: f64 = 1.5; // cells per second
const CONTRAST: f64 = 1.6;
const GAMMA: f64 = 2.2;
const W_GAIN: f64 = 0.6; // White channel trim

// ── Intensity breathing (silence default) ────────────────────────────────────
const I_SILENCE_FLOOR: f64 = 0.05;
const I_SILENCE_CEIL: f64 = 0.95;
const I_SILENCE_PERIOD_S: f64 = 2.5;

// ── Per-channel Perlin seeds ─────────────────────────────────────────────────
// Distinct 64-bit seeds give independent noise fields sharing one drift speed.
// Order: Fixture A [R, G, B, W], Fixture B [R, G, B, W]
const SEEDS: [u64; 8] = [
    0xcafe_babe_dead_beef,
    0x1234_5678_9abc_def0,
    0xfedc_ba98_7654_3210,
    0xa5a5_a5a5_5a5a_5a5a,
    0x0f0f_0f0f_f0f0_f0f0,
    0x5555_aaaa_5555_aaaa,
    0x3c3c_3c3c_c3c3_c3c3,
    0x6969_6969_9696_9696,
];

#[embassy_executor::task]
async fn noise_task(socket: UdpSocket, cid: [u8; 16]) -> ! {
    let fixture_a = Fixture { start_address: 1 };
    let fixture_b = Fixture { start_address: 6 };
    let frame_period = Duration::from_micros(1_000_000 / FRAME_RATE_HZ);
    let mut ticker = Ticker::every(frame_period);
    let mut sequence: u8 = 0;
    let start = std::time::Instant::now();

    loop {
        ticker.next().await;
        let elapsed = start.elapsed().as_secs_f64();
        let t = elapsed * NOISE_SPEED;

        // Intensity: slow sine breathing between floor and ceiling
        let phase = 2.0 * std::f64::consts::PI * elapsed / I_SILENCE_PERIOD_S;
        let breathing =
            I_SILENCE_FLOOR + (I_SILENCE_CEIL - I_SILENCE_FLOOR) * 0.5 * (1.0 + phase.sin());
        let intensity = (breathing * 255.0) as u8;

        // 10 DMX slots for two IRGBW fixtures
        let mut slots = [0u8; 10];

        slots[fixture_a.slot(0)] = intensity;
        slots[fixture_a.slot(1)] = to_dmx(noise1d(t, SEEDS[0]), CONTRAST, GAMMA, 1.0);
        slots[fixture_a.slot(2)] = to_dmx(noise1d(t, SEEDS[1]), CONTRAST, GAMMA, 1.0);
        slots[fixture_a.slot(3)] = to_dmx(noise1d(t, SEEDS[2]), CONTRAST, GAMMA, 1.0);
        slots[fixture_a.slot(4)] = to_dmx(noise1d(t, SEEDS[3]), CONTRAST, GAMMA, W_GAIN);

        slots[fixture_b.slot(0)] = intensity;
        slots[fixture_b.slot(1)] = to_dmx(noise1d(t, SEEDS[4]), CONTRAST, GAMMA, 1.0);
        slots[fixture_b.slot(2)] = to_dmx(noise1d(t, SEEDS[5]), CONTRAST, GAMMA, 1.0);
        slots[fixture_b.slot(3)] = to_dmx(noise1d(t, SEEDS[6]), CONTRAST, GAMMA, 1.0);
        slots[fixture_b.slot(4)] = to_dmx(noise1d(t, SEEDS[7]), CONTRAST, GAMMA, W_GAIN);

        let packet = sacn::encode(UNIVERSE, sequence, 100, &cid, &slots);
        sacn::send_multicast(&socket, UNIVERSE, SACN_PORT, &packet);
        sequence = sequence.wrapping_add(1);
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // Audio capture runs on its own OS thread, alongside Embassy.
    let _audio = audio::spawn_capture();

    let socket = UdpSocket::bind("0.0.0.0:0").expect("socket bind failed");
    socket
        .set_multicast_ttl_v4(1)
        .expect("set multicast TTL failed");
    let cid = sacn::new_cid();
    eprintln!(
        "brain: universe {} → 239.255.{}.{}:{}  @ {} Hz",
        UNIVERSE,
        UNIVERSE >> 8,
        UNIVERSE as u8,
        SACN_PORT,
        FRAME_RATE_HZ,
    );
    spawner.spawn(noise_task(socket, cid).unwrap());
}
