//! The rig's fixtures, as patched in the QLC+ workspace.
//!
//! `build.rs` reads the `<Fixture>` elements of `open-claw.qxw` and generates one constant
//! per fixture, named after the fixture's QLC+ name — "Yara 1" becomes `YARA_1`. **The
//! workspace is the source of truth for addressing**; nothing here is hand-maintained, and
//! there is no second copy of the patch to drift out of step with it.
//!
//! That naming is what makes the guarantee mechanical, in both directions:
//!
//!   * Patch a **new** fixture in QLC+ and nothing drives it → its constant is unused, and
//!     the build warns about dead code. The rig gained a fixture the daemon ignores, and
//!     the warning says so.
//!   * Delete (or rename) a fixture in QLC+ that code still drives → its constant is gone,
//!     and every use site fails to compile. The daemon cannot be built believing in a
//!     fixture the patch no longer has.
//!
//! Neither check is written anywhere. They fall out of naming the fixtures and letting the
//! compiler resolve them.
//!
//! Channel counts come along too, so code that writes a fixture's channels can pin the mode
//! it expects with a `const` assertion and break the build if the patch changes underneath
//! it.

use crate::fixture::Fixture;

include!(concat!(env!("OUT_DIR"), "/patch.rs"));
