//! The face's channels: the left lamp is the show's light, and the right lamp says a pluck can
//! be started with the right button, or, in Run, shows the motor's phase.
//!
//! Only the show's side of the face. Its buttons, its e-stop and the pins that dim the lamps
//! from the frame that went out are the face's own, in `face`.

use crate::patch::FACE;

/// Both lamps' levels, 0..1.
pub fn fill(slots: &mut [u8], left: f64, right: f64) {
    FACE.left_lamp.set_unit(slots, left);
    FACE.right_lamp.set_unit(slots, right);
}
