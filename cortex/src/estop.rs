//! The e-stop's mask: what it overrides in the frame it holds.
//!
//! A frame of zeros is not "off". Zero is off for light only; for a head it is the end of its
//! travel, for a speed channel often the fastest, for a wheel a slot. So the stop keeps every
//! channel where it was and overrides only the ones whose held value would be wrong: light
//! goes dark, and anything that runs by itself — strobe, auto programs, resets, rotation —
//! goes to the band where it does nothing.
//!
//! The overrides are compiled from the fixture definitions into the rig's `patch::STOP`, so
//! this module holds no opinion about any fixture.

use crate::dmx::Frame;
use crate::qlc_plus::Channel;

/// One channel the stop overrides, and the value it holds it at. A channel with no override
/// is held at whatever it last carried.
#[derive(Clone, Copy)]
pub struct StopOverride {
    pub channel: Channel,
    pub value: u8,
}

/// The frame the stop holds: `last`, the frame that went out before the press, under the
/// overrides.
pub fn held(last: &Frame, overrides: &[StopOverride]) -> Frame {
    let mut held = *last;
    for rule in overrides {
        rule.channel.set(&mut held, rule.value);
    }
    held
}
