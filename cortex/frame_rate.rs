// The rig's frame rate, in its own file so that both the code and the build script read the
// one number: `config.rs` includes it, and `build.rs` hands it to every rig crate's build
// script, which builds tables that hold a value per frame, such as a recording's envelope.
//
// Fixed 40 Hz, just under the ~43 Hz wire ceiling of a full 512-slot wired universe.
pub const FRAME_RATE_HZ: u64 = 40;
