//! The door wash's channels: the front door's colour, as R, G, B.

use crate::patch::DOOR_WASH;

pub fn fill(slots: &mut [u8], door: [u8; 3]) {
    let [red, green, blue] = door;
    DOOR_WASH.red.set(slots, red);
    DOOR_WASH.green.set(slots, green);
    DOOR_WASH.blue.set(slots, blue);
}
