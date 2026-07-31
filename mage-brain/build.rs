use std::env;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    cortex_build::ingest(&manifest_dir.join("..").join("mage.qxw"));
}
