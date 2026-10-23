//! The washes' channels for a cue.
//!
//! In the base show each wash holds its own leg's colour, with white beside it, breathing and
//! flashing with the one sparkle modulation all three share, so the rig pulses in step. The
//! incidents that have washes of their own replace that outright, at the state change.

use crate::ball_lighting;
use crate::legs::{Colour, LEGS, paint_wash};
use crate::show_state::{Base, Cue, Look};

/// Each wash's own colour, and the white beside it, at the top of the modulation.
const COLOUR_LEVEL: f64 = 0.8;
const WHITE_LEVEL: f64 = 0.5;
/// How far down the modulation may take the washes, because the black between glints reads
/// as the rig being off. Higher in daylight, which swamps a dim wash. The modulation is lifted
/// onto the range above the floor rather than clamped at it, so the quiet stretches still
/// move instead of sitting flat.
const DAYLIGHT_FLOOR: f64 = 0.35;
const NIGHT_FLOOR: f64 = 0.20;

/// Fills the washes for this frame. `sparkle` is the modulation's level.
pub fn fill(slots: &mut [u8], cue: Cue, sparkle: f64) {
    for leg in &LEGS {
        match cue.look() {
            Look::Base(Base::Blackout) => paint_wash(&leg.wash, slots, &[]),
            Look::Base(base) => {
                let floor = match base {
                    Base::Daylight => DAYLIGHT_FLOOR,
                    _ => NIGHT_FLOOR,
                };
                let level = floor + (1.0 - floor) * sparkle;
                paint_wash(
                    &leg.wash,
                    slots,
                    &[(leg.colour, COLOUR_LEVEL * level), (Colour::White, WHITE_LEVEL * level)],
                );
            }
            // Not scaled by the sparkle: in music it sits dark between glints, and the fade
            // would show only on the onsets.
            Look::BallLighting => {
                let colour = ball_lighting::phase(&cue);
                paint_wash(&leg.wash, slots, &[(leg.colour, colour), (Colour::White, 1.0 - colour)]);
            }
        }
    }
}
