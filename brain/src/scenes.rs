//! The scenes programmed in the QLC+ workspace, resolved into DMX slots.
//!
//! `build.rs` reads `open-claw.qxw` at build time and generates the `SCENES` table
//! included below; everything else here is hand-written. Scenes are keyed by **name** —
//! QLC+'s numeric Function IDs shift whenever functions are deleted and deliberately do
//! not cross into Rust.
//!
//! To change a scene: edit it in QLC+, save the workspace, deploy. The startup log names
//! the scenes the running binary was built from, which is how the operator confirms the
//! edit was picked up.

use crate::config as cfg;

/// One QLC+ scene.
pub struct Scene {
    /// The QLC+ Function name, snake_case and unique across the workspace.
    pub name: &'static str,
    /// `(slot, value)` pairs, ascending by slot. `slot` is a 0-based index into the
    /// universe's slot buffer — the same indexing `fixture::Fixture::slot` produces, so
    /// DMX address 1 is slot 0.
    ///
    /// Sparse on purpose. A QLC+ scene carries only the channels the operator actually
    /// touched, and a slot missing here is one the scene says nothing about — which is
    /// not the same as a slot the scene sets to zero. A dense array would have to invent
    /// the difference away.
    pub values: &'static [(u32, u8)],
}

include!(concat!(env!("OUT_DIR"), "/scenes.rs"));

// A scene reaching past the end of the frame the daemon actually sends would be silently
// truncated on the wire. This is the one place the QLC+ patch and the daemon's own
// addressing are cross-checked against each other, and the compiler does it for free.
const _: () = assert!(HIGHEST_SLOT < cfg::DMX_SLOTS);
