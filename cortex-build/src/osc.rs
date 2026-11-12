//! A device's OSC address space, from its OSCQuery file in `fixtures/`, as Rust: what the brain
//! decodes what the device sends by.
//!
//! The file is read by `clockwork/oscquery.py`, the reader Wrist's firmware build uses for its
//! encoders, so the encoders and the decoders come from one reading of one file, and both
//! builds refuse the same files.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

/// One OSC address, as the reader prints it.
struct Leaf {
    path: String,
    kind: String,
    values: Vec<String>,
}

/// Generates `osc_<device>.rs` in `OUT_DIR` from `<device>.oscquery.json`, holding
/// `pub mod <device>`: for every string address that lists its values, an enum of them, with
/// its address, its type tag and the words for each.
///
/// The reader is found in `clockwork/`, beside the `fixtures/` the file is in.
pub fn osc(file: &Path) {
    let name = file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_else(|| panic!("{} has no file name", file.display()));
    let device = name
        .strip_suffix(".oscquery.json")
        .unwrap_or_else(|| panic!("{} is not named <device>.oscquery.json", file.display()));
    let reader = file
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| panic!("{} is not in fixtures/ beside clockwork/", file.display()))
        .join("clockwork")
        .join("oscquery.py");
    println!("cargo:rerun-if-changed={}", file.display());
    println!("cargo:rerun-if-changed={}", reader.display());

    let leaves: Vec<Leaf> = crate::read_with(&reader, file, &[])
        .lines()
        .map(|line| {
            let mut words = line.split(' ').map(str::to_owned);
            let mut next = || {
                words
                    .next()
                    .unwrap_or_else(|| panic!("oscquery.py printed {line:?} for {}", file.display()))
            };
            let (path, kind) = (next(), next());
            Leaf {
                path,
                kind,
                values: words.collect(),
            }
        })
        .collect();

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    fs::write(out_dir.join(format!("osc_{device}.rs")), render(device, name, &leaves)).unwrap();
}

fn render(device: &str, source: &str, leaves: &[Leaf]) -> String {
    let mut out = String::new();
    writeln!(out, "// Generated from {source} by clockwork/oscquery.py. Do not edit.").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "#[allow(dead_code)]").unwrap();
    writeln!(out, "pub mod {device} {{").unwrap();
    for (i, leaf) in leaves.iter().filter(|leaf| !leaf.values.is_empty()).enumerate() {
        let name = camel(&leaf.path.trim_start_matches('/').replace('/', "_"));
        let variants: Vec<String> = leaf.values.iter().map(|v| camel(v)).collect();
        if i > 0 {
            writeln!(out).unwrap();
        }
        writeln!(out, "    /// `{}`'s values, in the order the file lists them.", leaf.path).unwrap();
        writeln!(out, "    #[derive(Clone, Copy, PartialEq, Eq, Debug)]").unwrap();
        writeln!(out, "    pub enum {name} {{").unwrap();
        for variant in &variants {
            writeln!(out, "        {variant},").unwrap();
        }
        writeln!(out, "    }}").unwrap();
        writeln!(out).unwrap();
        writeln!(out, "    impl {name} {{").unwrap();
        writeln!(out, "        pub const PATH: &str = {:?};", leaf.path).unwrap();
        writeln!(out, "        pub const TAG: &str = \",{}\";", leaf.kind).unwrap();
        writeln!(out).unwrap();
        writeln!(out, "        /// The value a word on the wire names, if it names one.").unwrap();
        writeln!(out, "        pub fn from_word(word: &str) -> Option<Self> {{").unwrap();
        writeln!(out, "            match word {{").unwrap();
        for (value, variant) in leaf.values.iter().zip(&variants) {
            writeln!(out, "                {value:?} => Some(Self::{variant}),").unwrap();
        }
        writeln!(out, "                _ => None,").unwrap();
        writeln!(out, "            }}").unwrap();
        writeln!(out, "        }}").unwrap();
        writeln!(out).unwrap();
        writeln!(out, "        /// The word on the wire.").unwrap();
        writeln!(out, "        pub fn word(self) -> &'static str {{").unwrap();
        writeln!(out, "            match self {{").unwrap();
        for (value, variant) in leaf.values.iter().zip(&variants) {
            writeln!(out, "                Self::{variant} => {value:?},").unwrap();
        }
        writeln!(out, "            }}").unwrap();
        writeln!(out, "        }}").unwrap();
        writeln!(out, "    }}").unwrap();
    }
    writeln!(out, "}}").unwrap();
    out
}

/// `wrist_pluck` → `WristPluck`.
fn camel(name: &str) -> String {
    name.split('_')
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default()
        })
        .collect()
}
