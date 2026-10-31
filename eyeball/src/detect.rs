//! The person detector: a picture in, the people in it out, as boxes in the picture's pixels.

use crate::config as cfg;
use crate::model;
use image::RgbImage;
use image::imageops::{self, FilterType};
use tract_linalg::multithread::{self, Executor};
use tract_onnx::prelude::*;

/// One person the detector found.
#[derive(Clone, Copy, Debug)]
pub struct Person {
    /// Objectness times the person class.
    pub score: f32,
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Person {
    fn area(&self) -> f32 {
        (self.right - self.left) * (self.bottom - self.top)
    }

    fn overlap(&self, other: &Person) -> f32 {
        let width = (self.right.min(other.right) - self.left.max(other.left)).max(0.0);
        let height = (self.bottom.min(other.bottom) - self.top.max(other.top)).max(0.0);
        let shared = width * height;
        shared / (self.area() + other.area() - shared)
    }
}

pub struct Detector {
    plan: Arc<TypedRunnableModel>,
}

/// The model's input, in pixels.
const WIDTH: usize = model::INPUT.0;
const HEIGHT: usize = model::INPUT.1;

impl Detector {
    /// Loads the compiled-in model and optimises it once, for its fixed shapes. The thread pool
    /// for its matrix products is spawned here and inherits the calling thread's cores and
    /// scheduling, so call it from the thread that will run the detector, once that thread has
    /// set them. A model that will not load is the build's, and no retry fixes it.
    pub fn new() -> Detector {
        multithread::set_default_executor(Executor::multithread_with_name(
            cfg::MODEL_THREADS,
            "detect",
        ));
        let optimised = tract_onnx::onnx()
            .model_for_read(&mut &model::MODEL[..])
            .and_then(|model| model.into_optimized())
            .unwrap_or_else(|e| panic!("detect: the model: {e}"));
        match optimised.input_fact(0).map(|fact| fact.shape.as_concrete()) {
            Ok(Some(&[1, 3, HEIGHT, WIDTH])) => {}
            other => panic!("detect: the model's input is not 1×3×{HEIGHT}×{WIDTH}: {other:?}"),
        }
        let plan = optimised
            .into_runnable()
            .unwrap_or_else(|e| panic!("detect: the model: {e}"));
        Detector { plan }
    }

    /// The people in `picture`, each once.
    pub fn detect(&self, picture: &RgbImage) -> Vec<Person> {
        let (input, scale) = letterbox(picture);
        let outputs = self
            .plan
            .run(tvec!(input.into()))
            .unwrap_or_else(|e| panic!("detect: run: {e}"));
        let rows = outputs[0]
            .try_as_plain_ram()
            .and_then(|view| view.as_slice::<f32>())
            .unwrap_or_else(|e| panic!("detect: output: {e}"));
        suppress(decode(rows, scale))
    }
}

/// The picture scaled to fit the model's input, keeping its aspect ratio, placed top-left on
/// grey padding, as BGR raw 0–255 in NCHW; and the scale, for the way back.
fn letterbox(picture: &RgbImage) -> (Tensor, f32) {
    let scale =
        (WIDTH as f32 / picture.width() as f32).min(HEIGHT as f32 / picture.height() as f32);
    let width = (picture.width() as f32 * scale) as u32;
    let height = (picture.height() as f32 * scale) as u32;
    let resized = imageops::resize(picture, width, height, FilterType::Triangle);

    let plane = WIDTH * HEIGHT;
    let mut data = vec![f32::from(model::PAD); 3 * plane];
    for (x, y, pixel) in resized.enumerate_pixels() {
        let at = y as usize * WIDTH + x as usize;
        let [red, green, blue] = pixel.0;
        data[at] = f32::from(blue);
        data[plane + at] = f32::from(green);
        data[2 * plane + at] = f32::from(red);
    }
    let shape = [1, 3, HEIGHT, WIDTH];
    let tensor = Tensor::from_shape(&shape, &data).unwrap_or_else(|e| panic!("detect: input: {e}"));
    (tensor, scale)
}

/// Every row that is a person, its box decoded from its grid cell and scaled back to the
/// picture's pixels. The rows are one grid per stride, each row-major.
fn decode(rows: &[f32], scale: f32) -> Vec<Person> {
    let mut people = Vec::new();
    let mut cells = rows.as_chunks::<{ model::COLUMNS }>().0.iter();
    for stride in model::STRIDES {
        let across = WIDTH / stride;
        let down = HEIGHT / stride;
        for (cell, row) in (&mut cells).take(across * down).enumerate() {
            let score = row[model::OBJECTNESS] * row[model::PERSON];
            if score <= cfg::CONFIDENCE {
                continue;
            }
            let stride = stride as f32;
            let x = (row[0] + (cell % across) as f32) * stride;
            let y = (row[1] + (cell / across) as f32) * stride;
            let half_width = row[2].exp() * stride / 2.0;
            let half_height = row[3].exp() * stride / 2.0;
            people.push(Person {
                score,
                left: (x - half_width) / scale,
                top: (y - half_height) / scale,
                right: (x + half_width) / scale,
                bottom: (y + half_height) / scale,
            });
        }
    }
    people
}

/// Non-maximum suppression: the surest box first, and any box overlapping a kept one by more
/// than `OVERLAP` taken for the same person.
fn suppress(mut people: Vec<Person>) -> Vec<Person> {
    people.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut kept: Vec<Person> = Vec::new();
    for person in people {
        if kept
            .iter()
            .all(|other| person.overlap(other) <= cfg::OVERLAP)
        {
            kept.push(person);
        }
    }
    kept
}
