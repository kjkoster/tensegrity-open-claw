//! What the eyeball saw, drawn onto the picture it saw it in: a box and its score for every
//! person, a dot on every leading foot and a larger one on the furthest, the two columns, and
//! the distance. For aiming the camera, and for judging the detector against the yard.

use crate::config as cfg;
use crate::detect::Person;
use crate::yard;
use font8x8::UnicodeFonts;
use image::{Rgb, RgbImage};

const YELLOW: Rgb<u8> = Rgb([255, 220, 0]);
const GREEN: Rgb<u8> = Rgb([0, 255, 0]);
const RED: Rgb<u8> = Rgb([255, 0, 0]);
const MAGENTA: Rgb<u8> = Rgb([255, 0, 255]);
const WHITE: Rgb<u8> = Rgb([255, 255, 255]);
const BLACK: Rgb<u8> = Rgb([0, 0, 0]);

/// Draws `people` and the yard onto `picture`, with a heading of the distance, the count, and
/// `more` after them.
pub fn annotate(picture: &mut RgbImage, people: &[Person], more: &str) {
    let (width, height) = picture.dimensions();
    for (column, label) in [(cfg::TRIGGER_COLUMN, "rose"), (cfg::ENTRY_COLUMN, "entry")] {
        let x = (column * width as f32) as i32;
        rectangle(picture, x - 1, 0, x, height as i32 - 1, YELLOW);
        text(picture, x + 4, height as i32 - 20, label, 2, YELLOW);
    }
    let furthest = yard::furthest(people, width);
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
    let distance = yard::word(yard::distance(people, width));
    let heading = format!("distance {distance}   {} people   {more}", people.len());
    text(picture, 8, 8, heading.trim_end(), 2, WHITE);
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
    for (grow, ink) in [(1, BLACK), (0, colour)] {
        for (index, character) in words.chars().enumerate() {
            let glyph = font8x8::BASIC_FONTS.get(character).unwrap_or([0; 8]);
            let x0 = left + index as i32 * 8 * scale;
            for (row, bits) in glyph.iter().enumerate() {
                for column in 0..8 {
                    if bits >> column & 1 == 1 {
                        let x = x0 + column * scale;
                        let y = top + row as i32 * scale;
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
