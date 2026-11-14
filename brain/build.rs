// The table names its days by kind, and need not use both.
#[allow(unused_imports)]
use cortex_build::Day::{self, DuskShowDay, TestDay};
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

    // What Wrist sends, which the brain decodes by.
    cortex_build::osc(&root.join("fixtures").join("wrist.oscquery.json"));

    println!("cargo:rerun-if-changed=shows.rs");
    cortex_build::calendar(CITY, SHOWS);
}
