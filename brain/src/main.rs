mod audio_features;
mod capture;
mod clock;
mod config;
mod dmx;
mod dmx_hat;
mod fixture;
mod laser;
mod latest;
mod orchestrator;
mod perlin;
mod recorder;
mod sparkle;

use audio_features::AudioFeatures;
use config::{FRAME_RATE_HZ, SACN_PORT, SACN_PRIORITY, SACN_RELEASE_FRAMES, UNIVERSE};
use embassy_executor::Spawner;
use signal_hook::{consts::SIGTERM, iterator::Signals};
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

    // systemd stops the brain with SIGTERM. Catch it to release the brain's sACN
    // source — a burst of terminate frames — so a higher-priority console or the
    // fixtures' fallback takes over at once instead of waiting out the 2.5 s data-loss
    // timeout. signal_hook's iterator runs on its own thread, outside async-signal
    // context, so the log line and socket sends here are safe.
    let shutdown_socket = socket.try_clone().expect("socket clone failed");
    let mut signals = Signals::new([SIGTERM]).expect("SIGTERM registration failed");
    std::thread::spawn(move || {
        if signals.forever().next().is_some() {
            eprintln!("brain: SIGTERM — releasing sACN source ({SACN_RELEASE_FRAMES} terminate frames)");
            for sequence in 0..SACN_RELEASE_FRAMES {
                let packet = dmx::encode_release(UNIVERSE, sequence, SACN_PRIORITY, &cid);
                dmx::send_multicast(&shutdown_socket, UNIVERSE, SACN_PORT, &packet);
            }
            std::process::exit(0);
        }
    });

    spawner.spawn(orchestrator::noise_task(socket, cid, rx).unwrap());
}
