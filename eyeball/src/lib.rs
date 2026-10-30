//! The eyeball: the retina's frames in, and how far into the yard the furthest person has come
//! out, for the show to read.
//!
//! A thread of its own in the brain, on the background cores, taking each newest frame from the
//! retina, finding the people in it and measuring the yard from them. The reading goes out once
//! per frame on a latest-value seam, with the time the frame was seen, so a camera or detector
//! that stops is a reading going stale, which the show turns into an empty yard.
//!
//! It is a part on the broker, publishing through the brain's connection under `eyeball/`:
//! `distance`, in paths to a hundredth or `tombstone`, and `people`, each when they change, and
//! `fps`, the frames a second it gets through. What it sees, drawn over, is the annotated feed,
//! at `config::FEED_ADDRESS`.

pub mod config;
pub mod detect;
pub mod draw;
pub mod model;
pub mod yard;

mod feed;

use cortex::latest::{LatestRx, latest};
use cortex::rate::Rate;
use cortex::telemetry::Publisher;
use detect::Detector;
use feed::Look;
use image::ImageFormat;
use retina::Source;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

/// What the eyeball saw in one frame.
#[derive(Clone, Copy, Debug)]
pub struct Sighting {
    /// The furthest person's distance into the yard, in paths, or `+∞` for nobody.
    pub distance: f64,
    /// When the frame was seen, so the show can tell a reading from a stale one.
    pub seen: Instant,
}

/// Starts the retina on `source`, set up for the eyeball's `config::VIEW`, the eyeball's thread
/// on its frames, and the annotated feed beside it, and returns the seam the eyeball's readings
/// go out on: `None` until the first frame has been through the detector. Both publish through
/// `telemetry`'s connection, under `retina/` and `eyeball/`.
pub fn spawn(source: Source, telemetry: Publisher) -> LatestRx<Option<Sighting>> {
    let frames = retina::spawn(source, config::VIEW, telemetry.part("retina"));
    let telemetry = telemetry.part("eyeball");
    let (sightings, reader) = latest(None);
    let (looks, feed_looks) = latest(None);
    feed::spawn(feed_looks);
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
                sightings.publish(Some(Sighting {
                    distance,
                    seen: frame.seen,
                }));

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
                looks.publish(Some(Arc::new(Look {
                    picture,
                    people,
                    fps,
                })));
            }
        })
        .expect("failed to spawn the eyeball's thread");
    reader
}
