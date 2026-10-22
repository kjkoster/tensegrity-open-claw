use std::env;
use std::path::PathBuf;

include!("presets.rs");
include!("shows.rs");

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    println!("cargo:rerun-if-changed=presets.rs");
    cortex_build::ingest(&manifest_dir.join("..").join("claw.qxw"), PRESETS);

    println!("cargo:rerun-if-changed=shows.rs");
    cortex_build::calendar(CITY, SHOWS);
}
