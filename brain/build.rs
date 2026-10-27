use std::env;
use std::path::PathBuf;

include!("presets.rs");
include!("shows.rs");
include!("sounds.rs");

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    println!("cargo:rerun-if-changed=presets.rs");
    cortex_build::ingest(&manifest_dir.join("..").join("claw.qxw"), PRESETS);

    println!("cargo:rerun-if-changed=shows.rs");
    cortex_build::calendar(CITY, SHOWS);

    // The envelopes hold a value per frame, so they are built at the rate cortex runs the
    // frame loop at, which it hands every crate that depends on it.
    println!("cargo:rerun-if-changed=sounds.rs");
    let frame_rate_hz = env::var("DEP_CORTEX_FRAME_RATE_HZ")
        .unwrap()
        .parse()
        .unwrap();
    mouth_build::sounds(&manifest_dir.join("sounds"), SOUNDS, frame_rate_hz);
}
