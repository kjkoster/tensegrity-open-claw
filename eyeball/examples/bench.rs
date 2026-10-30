//! Times the detector the way the brain runs it: on the background cores at ordinary priority,
//! with the model's thread pool spawned after the pin, so it lands there too.
//!
//! `cargo run --release -p eyeball --example bench -- [MODEL.onnx] [PICTURE]`
//!
//! Without a model it times the one compiled in. Without a picture it times a generated 640×360
//! frame: the model's run time does not depend on what the picture shows, only its size.

use eyeball::config as cfg;
use eyeball::detect::Detector;
use image::{Rgb, RgbImage};
use std::time::{Duration, Instant};

/// Runs before timing starts, so the first allocations and cold caches are not counted.
const WARM_UP: usize = 5;
const RUNS: usize = 50;

fn main() {
    #[cfg(target_os = "linux")]
    cortex::sched::background();

    let mut model = None;
    let mut picture = RgbImage::from_fn(640, 360, |x, y| {
        let grey = ((x + y) % 256) as u8;
        Rgb([grey, grey, grey])
    });
    for argument in std::env::args().skip(1) {
        if argument.ends_with(".onnx") {
            model = Some(std::fs::read(&argument).unwrap_or_else(|e| panic!("{argument}: {e}")));
        } else {
            picture = image::open(&argument)
                .unwrap_or_else(|e| panic!("{argument}: {e}"))
                .to_rgb8();
        }
    }

    let started = Instant::now();
    let detector = match &model {
        Some(onnx) => Detector::from_onnx(onnx),
        None => Detector::new(),
    };
    let (width, height) = detector.input();
    println!(
        "{width}x{height} model loaded and optimised in {} ms",
        started.elapsed().as_millis()
    );

    for _ in 0..WARM_UP {
        detector.detect(&picture);
    }
    let mut times: Vec<Duration> = (0..RUNS)
        .map(|_| {
            let started = Instant::now();
            detector.detect(&picture);
            started.elapsed()
        })
        .collect();
    times.sort();

    let median = times[RUNS / 2];
    println!(
        "{}x{} picture, {} threads, {RUNS} runs: median {} ms, 95% within {} ms, slowest {} ms, \
         {:.1} frames a second at the median",
        picture.width(),
        picture.height(),
        cfg::MODEL_THREADS,
        median.as_millis(),
        times[RUNS * 95 / 100].as_millis(),
        times[RUNS - 1].as_millis(),
        1.0 / median.as_secs_f64(),
    );
}
