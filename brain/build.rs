use std::env;
use std::path::PathBuf;

include!("shows.rs");

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest_dir.join("..");
    cortex_build::ingest(
        &root.join("clockwork.qxw"),
        &cortex_build::Network {
            dot: &root.join("clockwork.dot"),
            device: "chaos_pi",
            service: "brain",
        },
    );

    println!("cargo:rerun-if-changed=shows.rs");
    cortex_build::calendar(CITY, SHOWS);
}
