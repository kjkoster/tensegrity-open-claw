//! Runs the detector over still pictures and writes each one annotated: the boxes with their
//! scores, every leading foot, the furthest one larger, the two columns, and the distance.
//!
//! `cargo run --release -p eyeball --example annotate -- OUTDIR [MODEL.onnx] PICTURE...`
//!
//! Without a model it runs the one compiled in.
//!
//! Each is written as `<name>-distance-<distance>.png`, its name up to any `-distance-` it
//! already carries, so a set named by one run can be compared with this one's.

use eyeball::detect::Detector;
use eyeball::{draw, yard};
use std::path::Path;
use std::time::Instant;

fn main() {
    let mut arguments = std::env::args().skip(1).peekable();
    let out = arguments.next().expect("an output directory");
    let detector = match arguments.next_if(|argument| argument.ends_with(".onnx")) {
        Some(path) => {
            Detector::from_onnx(&std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}")))
        }
        None => Detector::new(),
    };
    for path in arguments {
        let mut picture = image::open(&path)
            .unwrap_or_else(|e| panic!("{path}: {e}"))
            .to_rgb8();
        let started = Instant::now();
        let people = detector.detect(&picture);
        let took = started.elapsed();
        let distance = yard::distance(&people, picture.width());

        draw::annotate(&mut picture, &people, "rust");
        let stem = Path::new(&path).file_stem().unwrap().to_string_lossy();
        let stem = stem.split("-distance-").next().unwrap();
        let name = format!("{stem}-distance-{}.png", word(distance));
        picture
            .save(Path::new(&out).join(&name))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        println!(
            "{stem:20} {:3} people  distance {:9}  {:4} ms",
            people.len(),
            word(distance),
            took.as_millis()
        );
    }
}

fn word(distance: f64) -> String {
    if distance.is_infinite() {
        "tombstone".to_string()
    } else {
        format!("{distance:.3}")
    }
}
