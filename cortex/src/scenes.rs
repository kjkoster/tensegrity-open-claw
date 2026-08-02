//! The shape of a QLC+ scene, resolved into DMX slots.
//!
//! The scenes themselves are per-rig: each rig's build script generates its own `SCENES`
//! table from its own workspace, and only this type is shared. Scenes are keyed by **name**
//! — QLC+'s numeric Function IDs shift whenever functions are deleted and deliberately do
//! not cross into Rust.
//!
//! To change a scene: edit it in QLC+, save the workspace, deploy. The startup log names
//! the scenes the running binary was built from, which is how the operator confirms the
//! edit was picked up.

/// One QLC+ scene.
pub struct Scene {
    /// The QLC+ Function name, snake_case and unique across the workspace.
    pub name: &'static str,
    /// `(slot, value)` pairs, ascending by slot. `slot` is a 0-based index into the
    /// universe's slot buffer, the same indexing `qlc_plus::Channel` carries, so
    /// DMX address 1 is slot 0.
    ///
    /// Sparse on purpose. A QLC+ scene carries only the channels the operator actually
    /// touched, and a slot missing here is one the scene says nothing about — which is
    /// not the same as a slot the scene sets to zero. A dense array would have to invent
    /// the difference away.
    pub values: &'static [(u32, u8)],
}

impl Scene {
    /// Lays the scene's values into a slot buffer.
    ///
    /// Into a buffer the caller owns rather than into one returned, because the sparseness
    /// above is the whole point: the slots this scene never mentions keep whatever they were
    /// carrying, which is what lets a scene be a statement about a few fixtures instead of
    /// about the universe. Reading a fixture's channels out of the result is then the same
    /// operation whether the values arrived from a scene or from the running show.
    pub fn apply(&self, slots: &mut [u8]) {
        for (slot, value) in self.values {
            slots[*slot as usize] = *value;
        }
    }
}
