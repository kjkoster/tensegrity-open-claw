//! The person detector: YOLOX-Nano, compiled in.
//!
//! `yolox_nano_320x256.onnx`: Megvii's own YOLOX-Nano weights, `yolox_nano.pth` from YOLOX's
//! release `0.1.1rc0` (Apache-2.0, 0.9 M parameters), exported at 320×256 by `models/export.py`,
//! which says why and how; SHA-256
//! `04f0c2e08cbb06ba226138c19e2cc6456b712daface7c93c9f6272d10aba3c60`, which the script
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
//! One input, `images`, `[1, 3, 256, 320]`; one output, `output`, `[1, 1680, 85]`. The
//! detector reads the size from the model, so an export at another size runs unchanged.
//!
//! **What it gives out.** One row per cell of three grids, strides 8, 16 and 32 in that order,
//! each grid row-major: `[cx, cy, w, h, objectness, class_0 … class_79]`. Objectness and the
//! classes have been through a sigmoid already; the box has not been decoded, since the graph
//! has no `exp`: `x = (cx + grid_x) · stride`, `w = exp(w) · stride`, and likewise for y and h.

/// The ONNX file, so the binary on the Pi is the whole show and has no model file to miss.
pub const MODEL: &[u8] = include_bytes!("../models/yolox_nano_320x256.onnx");

// A different file under the same name would load and quietly detect differently.
const _: () = assert!(
    MODEL.len() == 3_713_972,
    "not the eyeball's yolox_nano_320x256.onnx"
);

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
