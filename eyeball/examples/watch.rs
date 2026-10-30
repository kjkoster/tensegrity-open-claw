//! Runs the retina and the eyeball as the brain will, without the brain, and prints every
//! reading: for checking the camera, its settings and the yard's marks at the bench.
//!
//! `cargo run --release -p eyeball --example watch -- [FOOTAGE.mjpg]`
//!
//! Without footage it watches the camera. Its telemetry goes out under `retina/` and
//! `eyeball/`, on a connection of its own named `eyeball-watch`.

use cortex::telemetry;
use retina::Source;
use std::thread;
use std::time::Duration;

fn main() {
    let source = match std::env::args().nth(1) {
        Some(path) => Source::Footage(path.into()),
        None => Source::Camera,
    };
    let (publisher, _farewell) = telemetry::spawn("eyeball-watch");
    let sightings = eyeball::spawn(source, publisher);

    let mut last = None;
    loop {
        if let Some(sighting) = sightings.snapshot()
            && last != Some(sighting.seen)
        {
            last = Some(sighting.seen);
            let age = sighting.seen.elapsed().as_millis();
            println!(
                "distance {:>9.3}   {age:>4} ms after the frame arrived",
                sighting.distance
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
}
