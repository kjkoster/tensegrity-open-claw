use std::path::PathBuf;

/// The shared OSC blocks every device built on cortex implements, as `report` builds them.
const SHARED: [&str; 4] = ["sys", "wifi", "health", "dmx"];

fn main() {
    // Hands the target triplet the program is compiled for to the code, which reports it on
    // `/sys/<device>/triplet`: cargo only tells build scripts.
    println!(
        "cargo::rustc-env=TRIPLET={}",
        std::env::var("TARGET").unwrap()
    );

    let fixtures = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("..")
        .join("fixtures");
    for block in SHARED {
        cortex_build::shared(&fixtures.join(format!("{block}.oscquery.json")));
    }
}
