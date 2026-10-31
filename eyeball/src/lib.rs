//! The eyeball: the retina's frames in, and how far into the yard the furthest person has come
//! out, for the show to read.
//!
//! A thread of its own in the brain, on the background cores, taking each newest frame from the
//! retina, finding the people in it and measuring the yard from them. Each frame's sighting goes
//! out on a latest-value seam with the time the frame was seen, so a camera or detector that
//! stops is a sighting going stale, which the show turns into an empty yard. The sighting
//! carries the picture and the boxes too, for the rig's annotated feed to draw.
//!
//! It is a part on the broker, publishing through the brain's connection under `eyeball/`:
//! `distance`, in paths to a hundredth or `tombstone`, and `people`, each when they change, and
//! `fps`, the frames a second it gets through.

pub mod config;
pub mod detect;
pub mod yard;

mod model;

use cortex::latest::{LatestRx, latest};
use cortex::rate::Rate;
use cortex::telemetry::Publisher;
use detect::{Detector, Person};
use image::{ImageFormat, RgbImage};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

/// What the eyeball saw in one frame.
pub struct Sighting {
    /// The furthest person's distance into the yard, in paths, or `+∞` for nobody.
    pub distance: f64,
    /// When the frame was seen, so the show can tell a sighting from a stale one.
    pub seen: Instant,
    pub picture: RgbImage,
    pub people: Vec<Person>,
    /// The eyeball's latest frame rate, once it has one.
    pub fps: Option<f64>,
}

/// The newest sighting, or none before the first frame has been through the detector.
pub type Sightings = LatestRx<Option<Arc<Sighting>>>;

/// Starts the retina, set up for the eyeball's `config::VIEW`, and the eyeball's thread on its
/// frames. Both publish through `telemetry`'s connection, under `retina/` and `eyeball/`.
pub fn spawn(telemetry: Publisher) -> Sightings {
    let frames = retina::spawn(config::VIEW, telemetry.part("retina"));
    let telemetry = telemetry.part("eyeball");
    let (sightings, reader) = latest(None);
    thread::Builder::new()
        .name("eyeball".into())
        .spawn(move || {
            // Before the detector exists, so its thread pool is spawned on the background cores.
            cortex::sched::background();
            let detector = Detector::new();
            let mut rate = Rate::new();
            let mut fps = None;
            // What `distance` and `people` last said, so each is published only when it changes.
            let mut published_distance = String::new();
            let mut published_people = usize::MAX;
            loop {
                let frame = frames.take();
                // A frame garbled on the wire costs that frame; the next one is likely whole.
                let picture =
                    match image::load_from_memory_with_format(&frame.jpeg, ImageFormat::Jpeg) {
                        Ok(picture) => picture.to_rgb8(),
                        Err(e) => {
                            let bytes = frame.jpeg.len();
                            eprintln!("eyeball: a frame of {bytes} bytes that is not a JPEG: {e}");
                            continue;
                        }
                    };
                let people = detector.detect(&picture);
                let distance = yard::distance(&people, picture.width());

                let word = yard::word(distance);
                if word != published_distance {
                    telemetry.publish("distance", word.clone(), true);
                    published_distance = word;
                }
                if people.len() != published_people {
                    telemetry.publish("people", people.len().to_string(), true);
                    published_people = people.len();
                }
                if let Some(rate) = rate.tick() {
                    telemetry.publish("fps", format!("{rate:.1}"), true);
                    fps = Some(rate);
                }
                sightings.publish(Some(Arc::new(Sighting {
                    distance,
                    seen: frame.seen,
                    picture,
                    people,
                    fps,
                })));
            }
        })
        .expect("failed to spawn the eyeball's thread");
    reader
}
