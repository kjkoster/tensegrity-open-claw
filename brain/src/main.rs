mod audio_features;
mod capture;
mod clock;
mod config;
mod dmx;
mod fixture;
mod latest;
mod orchestrator;
mod perlin;
mod recorder;
mod sparkle;

use audio_features::AudioFeatures;
use config::{FRAME_RATE_HZ, SACN_PORT, UNIVERSE};
use embassy_executor::Spawner;
use std::net::UdpSocket;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let (tx, rx) = latest::latest(AudioFeatures::default());

    // Audio capture and the parquet recorder each run on their own OS thread,
    // alongside Embassy.
    let _audio = capture::spawn_capture(tx);
    let _recorder = recorder::spawn_recorder(rx.clone());

    let socket = UdpSocket::bind("0.0.0.0:0").expect("socket bind failed");
    socket
        .set_multicast_ttl_v4(1)
        .expect("set multicast TTL failed");
    let cid = dmx::new_cid();
    let group = dmx::multicast_addr(UNIVERSE);
    eprintln!("brain: universe {UNIVERSE} → {group}:{SACN_PORT}  @ {FRAME_RATE_HZ} Hz");
    spawner.spawn(orchestrator::noise_task(socket, cid, rx).unwrap());
}
