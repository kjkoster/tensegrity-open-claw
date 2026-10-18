//! The universe itself, as every part of the brain holds it: one frame of 512 slots, whichever
//! transport it then goes out on.

/// One universe's worth of slots, as the frame loop fills and sends it.
pub type Frame = [u8; zihatec_rs_485_dmx::SLOTS];

/// Every slot zero.
pub const BLACKOUT: Frame = [0; zihatec_rs_485_dmx::SLOTS];
