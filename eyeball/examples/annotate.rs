//! Runs the detector over still pictures and writes each one annotated: the boxes with their
//! scores, every leading foot, the furthest one larger, the two columns, and the distance.
//!
//! `cargo run --release -p eyeball --example annotate -- OUTDIR [MODEL.onnx] PICTURE...`
//!
//! Without a model it runs the one compiled in.
//!
//! Each is written as `<name>-distance-<distance>.png`, its name up to any `-distance-` it
//! already carries, so a set named by one run can be compared with this one's.

use eyeball::config as cfg;
use eyeball::detect::{Detector, Person};
use eyeball::yard;
use font8x8::UnicodeFonts;
use image::{Rgb, RgbImage};
use std::path::Path;
use std::time::Instant;

const YELLOW: Rgb<u8> = Rgb([255, 220, 0]);
const GREEN: Rgb<u8> = Rgb([0, 255, 0]);
const RED: Rgb<u8> = Rgb([255, 0, 0]);
const MAGENTA: Rgb<u8> = Rgb([255, 0, 255]);
const WHITE: Rgb<u8> = Rgb([255, 255, 255]);
const BLACK: Rgb<u8> = Rgb([0, 0, 0]);

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
        let width = picture.width();
        let distance = yard::distance(&people, width);
        let furthest = yard::furthest(&people, width);

        annotate(&mut picture, &people, furthest, distance);
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

fn annotate(picture: &mut RgbImage, people: &[Person], furthest: Option<&Person>, distance: f64) {
    let (width, height) = picture.dimensions();
    for (column, label) in [(cfg::TRIGGER_COLUMN, "rose"), (cfg::ENTRY_COLUMN, "entry")] {
        let x = (column * width as f32) as i32;
        rectangle(picture, x - 1, 0, x, height as i32 - 1, YELLOW);
        text(picture, x + 4, height as i32 - 20, label, 2, YELLOW);
    }
    for person in people {
        let (left, top) = (person.left as i32, person.top as i32);
        let (right, bottom) = (person.right as i32, person.bottom as i32);
        for inset in 0..2 {
            outline(
                picture,
                left + inset,
                top + inset,
                right - inset,
                bottom - inset,
                GREEN,
            );
        }
        text(
            picture,
            left + 3,
            top + 3,
            &format!("{:.2}", person.score),
            2,
            GREEN,
        );
        let is_furthest = furthest.is_some_and(|f| std::ptr::eq(f, person));
        let (colour, radius) = if is_furthest { (MAGENTA, 8) } else { (RED, 5) };
        disc(picture, left, bottom, radius, colour);
    }
    let heading = format!(
        "distance {}   {} people   rust",
        word(distance),
        people.len()
    );
    text(picture, 8, 8, &heading, 2, WHITE);
}

fn put(picture: &mut RgbImage, x: i32, y: i32, colour: Rgb<u8>) {
    if x >= 0 && y >= 0 && (x as u32) < picture.width() && (y as u32) < picture.height() {
        picture.put_pixel(x as u32, y as u32, colour);
    }
}

fn rectangle(
    picture: &mut RgbImage,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    colour: Rgb<u8>,
) {
    for y in top..=bottom {
        for x in left..=right {
            put(picture, x, y, colour);
        }
    }
}

fn outline(picture: &mut RgbImage, left: i32, top: i32, right: i32, bottom: i32, colour: Rgb<u8>) {
    rectangle(picture, left, top, right, top, colour);
    rectangle(picture, left, bottom, right, bottom, colour);
    rectangle(picture, left, top, left, bottom, colour);
    rectangle(picture, right, top, right, bottom, colour);
}

fn disc(picture: &mut RgbImage, cx: i32, cy: i32, radius: i32, colour: Rgb<u8>) {
    for y in -radius..=radius {
        for x in -radius..=radius {
            if x * x + y * y <= radius * radius {
                put(picture, cx + x, cy + y, colour);
            }
        }
    }
}

/// 8×8 bitmap text at `scale`, on a black outline so it reads over any picture.
fn text(picture: &mut RgbImage, left: i32, top: i32, words: &str, scale: i32, colour: Rgb<u8>) {
    for (shift, ink) in [(1, BLACK), (0, colour)] {
        for (index, character) in words.chars().enumerate() {
            let glyph = font8x8::BASIC_FONTS.get(character).unwrap_or([0; 8]);
            let x0 = left + index as i32 * 8 * scale;
            for (row, bits) in glyph.iter().enumerate() {
                for column in 0..8 {
                    if bits >> column & 1 == 1 {
                        let x = x0 + column * scale;
                        let y = top + row as i32 * scale;
                        let grow = if shift == 1 { 1 } else { 0 };
                        rectangle(
                            picture,
                            x - grow,
                            y - grow,
                            x + scale - 1 + grow,
                            y + scale - 1 + grow,
                            ink,
                        );
                    }
                }
            }
        }
    }
}
