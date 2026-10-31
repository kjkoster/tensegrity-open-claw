//! The person detector: YOLOX-Nano, compiled in.
//!
//! `yolox_nano_320x192.onnx`: Megvii's own YOLOX-Nano weights, `yolox_nano.pth` from YOLOX's
//! release `0.1.1rc0` (Apache-2.0, 0.9 M parameters), exported at 320×192 by `models/export.py`,
//! which says why and how; SHA-256
//! `0258249894b093f52f002830d09184dfadcd7cb4388fcb82bb4b01547e57a919`, which the script
//! reproduces byte for byte. ONNX opset 11; every shape is fixed, and the graph uses only
//! operators every runtime has: convolutions, sigmoids, slices, concatenations, max-pooling,
//! resizes and reshapes.
//!
//! **What it wants in.** The picture in BGR order, as raw 0–255 values, letterboxed into the
//! top-left of the input padded with grey 114, as NCHW `f32`. The weights in this release were
//! retrained without normalisation, and the graph scales nothing itself, so any division by
//! 255 or mean/std subtraction makes it see near-black and find nobody, without an error. The
//! demo tagged `0.1.1rc0` in YOLOX's repository still normalises: it predates the retrain, and
//! is not the reference for these weights. YOLOX's later code is.
//!
//! One input, `images`, `[1, 3, 192, 320]`; one output, `output`, `[1, 1260, 85]`. An
//! export at another size is a change to `INPUT` with it.
//!
//! **What it gives out.** One row per cell of three grids, strides 8, 16 and 32 in that order,
//! each grid row-major: `[cx, cy, w, h, objectness, class_0 … class_79]`. Objectness and the
//! classes have been through a sigmoid already; the box has not been decoded, since the graph
//! has no `exp`: `x = (cx + grid_x) · stride`, `w = exp(w) · stride`, and likewise for y and h.

/// The ONNX file, so the binary on the Pi is the whole show and has no model file to miss.
pub const MODEL: &[u8] = include_bytes!("../models/yolox_nano_320x192.onnx");

/// The compiled-in model's input, width by height: the camera's 16:9 at half of 640×360,
/// rounded up to the 32 the coarsest grid needs. `Detector::new` checks the model against it,
/// since exports at different sizes are the same length, and a wrong one would load and
/// quietly see the yard at the wrong scale.
pub const INPUT: (usize, usize) = (320, 192);

/// The letterbox's padding, the grey the model was trained with.
pub const PAD: u8 = 114;

/// The three grids' strides, in the order their rows come out.
pub const STRIDES: [usize; 3] = [8, 16, 32];

/// A row's columns: the box, the objectness and the 80 COCO classes.
pub const COLUMNS: usize = 85;

/// The objectness's column; a person's score is it times `PERSON`'s.
pub const OBJECTNESS: usize = 4;

/// COCO's class 0, the only one read.
pub const PERSON: usize = 5;
