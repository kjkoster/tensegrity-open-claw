mod audio;
mod config;
mod control;
mod dsp;
mod features;
mod fixture;
mod mapping;
mod perlin;
mod recorder;
mod sacn;
mod spectral;

use config::{
    CONTRAST, FRAME_RATE_HZ, GAMMA, I_SILENCE_CEIL, I_SILENCE_FLOOR, I_SILENCE_PERIOD_S,
    SACN_PORT, SEEDS, UNIVERSE,
};
use control::ControlReader;
use embassy_executor::Spawner;
use embassy_time::{Duration, Ticker};
use fixture::Fixture;
use mapping::Mapping;
use perlin::{fbm2, to_dmx};
use std::net::UdpSocket;

#[embassy_executor::task]
async fn noise_task(socket: UdpSocket, cid: [u8; 16], control: ControlReader) -> ! {
    let fixture_a = Fixture { start_address: 1 };
    let fixture_b = Fixture { start_address: 6 };
    let frame_period = Duration::from_micros(1_000_000 / FRAME_RATE_HZ);
    let mut ticker = Ticker::every(frame_period);
    let mut sequence: u8 = 0;
    let start = std::time::Instant::now();
    let dt = 1.0 / FRAME_RATE_HZ as f64;
    let mut mapping = Mapping::new();
    // Drift speed varies per frame, so the noise coordinate is an accumulated
    // phase rather than elapsed × speed — otherwise speed changes jump the field.
    let mut t = 0.0_f64;

    loop {
        ticker.next().await;
        let snapshot = control.snapshot();
        let elapsed = start.elapsed().as_secs_f64();

        // Intensity baseline: slow sine breathing between floor and ceiling
        let phase = 2.0 * std::f64::consts::PI * elapsed / I_SILENCE_PERIOD_S;
        let breathing =
            I_SILENCE_FLOOR + (I_SILENCE_CEIL - I_SILENCE_FLOOR) * 0.5 * (1.0 + phase.sin());

        let out = mapping.frame(&snapshot, breathing, dt);
        t += out.speed * dt;
        let intensity = (out.intensity * 255.0) as u8;

        // 10 DMX slots for two IRGBW fixtures
        let mut slots = [0u8; 10];

        slots[fixture_a.slot(0)] = intensity;
        slots[fixture_a.slot(1)] = to_dmx(fbm2(t, SEEDS[0], out.octave2), CONTRAST, GAMMA, 1.0);
        slots[fixture_a.slot(2)] = to_dmx(fbm2(t, SEEDS[1], out.octave2), CONTRAST, GAMMA, 1.0);
        slots[fixture_a.slot(3)] = to_dmx(fbm2(t, SEEDS[2], out.octave2), CONTRAST, GAMMA, 1.0);
        slots[fixture_a.slot(4)] =
            to_dmx(fbm2(t, SEEDS[3], out.octave2), CONTRAST, GAMMA, out.w_gain);

        slots[fixture_b.slot(0)] = intensity;
        slots[fixture_b.slot(1)] = to_dmx(fbm2(t, SEEDS[4], out.octave2), CONTRAST, GAMMA, 1.0);
        slots[fixture_b.slot(2)] = to_dmx(fbm2(t, SEEDS[5], out.octave2), CONTRAST, GAMMA, 1.0);
        slots[fixture_b.slot(3)] = to_dmx(fbm2(t, SEEDS[6], out.octave2), CONTRAST, GAMMA, 1.0);
        slots[fixture_b.slot(4)] =
            to_dmx(fbm2(t, SEEDS[7], out.octave2), CONTRAST, GAMMA, out.w_gain);

        let packet = sacn::encode(UNIVERSE, sequence, 100, &cid, &slots);
        sacn::send_multicast(&socket, UNIVERSE, SACN_PORT, &packet);
        sequence = sequence.wrapping_add(1);
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let (publisher, reader) = control::control_pair();

    // Audio capture and the parquet recorder each run on their own OS thread,
    // alongside Embassy.
    let _audio = audio::spawn_capture(publisher);
    let _recorder = recorder::spawn_recorder(reader.clone());

    let socket = UdpSocket::bind("0.0.0.0:0").expect("socket bind failed");
    socket
        .set_multicast_ttl_v4(1)
        .expect("set multicast TTL failed");
    let cid = sacn::new_cid();
    let group = sacn::multicast_addr(UNIVERSE);
    eprintln!("brain: universe {UNIVERSE} → {group}:{SACN_PORT}  @ {FRAME_RATE_HZ} Hz");
    spawner.spawn(noise_task(socket, cid, reader).unwrap());
}
