//! The rig's fixtures, as patched in the QLC+ workspace.
//!
//! `build.rs` reads the `<Fixture>` elements of `open-claw.qxw`, resolves each one against
//! its `.qxf` definition in `fixtures/`, and generates a struct per fixture with a named
//! field per channel of its patched mode. **The workspace and the definitions are the
//! source of truth**; nothing here is hand-maintained, and there is no second copy of the
//! patch to drift out of step with it.
//!
//! The generated names are what make drift mechanical, in both directions:
//!
//!   * Patch a **new** fixture in QLC+ and nothing drives it → its constant is unused, and
//!     the build warns about dead code.
//!   * Delete or rename a fixture that code drives → its constant is gone, and every use
//!     site fails to compile.
//!   * Repatch a fixture to a mode without a channel the code writes → the field is gone,
//!     or the capability trait is no longer implemented, and again the use site fails.
//!
//! That last one is why channels are typed rather than numbered. `PINSPOT.red` names the
//! red emitter because the definition says channel 2 of this mode is `IntensityRed`; there
//! is no offset in the source to get wrong, and a fixture with no red simply has no `red`.
//!
//! See `qlc_plus.rs` for the vocabulary these structs are built from.

include!(concat!(env!("OUT_DIR"), "/patch.rs"));
