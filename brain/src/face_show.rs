//! The face's channels: the left lamp shows what the rose is doing, and the right lamp says
//! test mode is on.
//!
//! The left lamp stands in for the rose while the heads are not driven, so the heart and the
//! thunderclap can be watched against the sound from the cabinet door.
//!
//! Only the show's side of the face. Its buttons, its e-stop and the pins that dim the lamps
//! from the frame that went out are the face's own, in `face`.

use crate::patch::FACE;

pub fn fill(slots: &mut [u8], rose: f64, testing: bool) {
    let test_lamp = if testing { 1.0 } else { 0.0 };
    FACE.left_lamp.set_unit(slots, rose);
    FACE.right_lamp.set_unit(slots, test_lamp);
}
