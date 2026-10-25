//! The face's channels: its two lamps, which say test mode is on.
//!
//! Only the show's side of the face. Its buttons, its e-stop and the pins that dim the lamps
//! from the frame that went out are the face's own, in `face`.

use crate::patch::FACE;

pub fn fill(slots: &mut [u8], testing: bool) {
    let lamps = if testing { 1.0 } else { 0.0 };
    FACE.left_lamp.set_unit(slots, lamps);
    FACE.right_lamp.set_unit(slots, lamps);
}
