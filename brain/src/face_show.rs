//! The face's channels: the left lamp is the show's light, and the right lamp says a pluck can
//! be started with the right button.
//!
//! Only the show's side of the face. Its buttons, its e-stop and the pins that dim the lamps
//! from the frame that went out are the face's own, in `face`.

use crate::patch::FACE;

pub fn fill(slots: &mut [u8], left: f64, can_pluck: bool) {
    FACE.left_lamp.set_unit(slots, left);
    FACE.right_lamp.set_unit(slots, if can_pluck { 1.0 } else { 0.0 });
}
