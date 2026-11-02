fn main() {
    // Hands the target triplet the brain is compiled for to the code, which publishes it on
    // `sys/brain/triplet`: cargo only tells build scripts.
    println!("cargo::rustc-env=TRIPLET={}", std::env::var("TARGET").unwrap());
}
