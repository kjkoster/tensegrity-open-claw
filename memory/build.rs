use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest_dir.join("..");

    // Where Memory listens for OSC and the broker it relays to.
    cortex_build::network(&cortex_build::Network {
        dot: &root.join("clockwork.dot"),
        device: "chaos_pi",
        service: "memory",
    });

    // Every device's OSC, Memory's own among them: its own to send by, all of them to know an
    // address no file has. Watched as a directory too, so a file added is picked up. A shared
    // file is no device's: it is in every file that implements it.
    let fixtures = root.join("fixtures");
    println!("cargo:rerun-if-changed={}", fixtures.display());
    let mut devices: Vec<String> = fs::read_dir(&fixtures)
        .unwrap_or_else(|e| panic!("{}: {e}", fixtures.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.to_string_lossy().ends_with(".oscquery.json"))
        .filter(|path| !cortex_build::is_shared(path))
        .filter_map(|path| {
            let name = path.file_name()?.to_str()?;
            name.strip_suffix(".oscquery.json").map(str::to_owned)
        })
        .collect();
    devices.sort();

    let mut out = String::new();
    for device in &devices {
        cortex_build::osc(&fixtures.join(format!("{device}.oscquery.json")));
        writeln!(
            out,
            "include!(concat!(env!(\"OUT_DIR\"), \"/osc_{device}.rs\"));"
        )
        .unwrap();
    }
    writeln!(out).unwrap();
    writeln!(out, "/// Every address in every device's file.").unwrap();
    writeln!(out, "pub const KNOWN: &[&[&str]] = &[").unwrap();
    for device in &devices {
        writeln!(out, "    {device}::PATHS,").unwrap();
    }
    writeln!(out, "];").unwrap();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    fs::write(out_dir.join("fixtures.rs"), out).unwrap();
}
