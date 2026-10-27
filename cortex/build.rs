include!("frame_rate.rs");

fn main() {
    // Hands the target triplet the brain is compiled for to the code, which publishes it on
    // `sys/brain/triplet`: cargo only tells build scripts.
    println!("cargo::rustc-env=TRIPLET={}", std::env::var("TARGET").unwrap());

    // Hands the frame rate to the build script of every crate that depends on this one, as
    // `DEP_CORTEX_FRAME_RATE_HZ`, which is what the `links` key in the manifest is for. A table
    // built per frame is then built at the rate the frame loop runs at, from the same number.
    println!("cargo::rerun-if-changed=frame_rate.rs");
    println!("cargo::metadata=frame_rate_hz={FRAME_RATE_HZ}");
}
