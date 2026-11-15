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
    /// `connect`, `data`, or a rate in Hz.
    how: String,
    values: Vec<String>,
}

impl Leaf {
    /// `pub const RATE_HZ`, for a leaf sent at a rate; nothing for one sent on an event.
    fn rate(&self) -> Option<String> {
        self.how
            .parse::<f64>()
            .ok()
            .map(|hz| format!("        pub const RATE_HZ: f64 = {hz:?};"))
    }
}

/// Generates `osc_<device>.rs` in `OUT_DIR` from `<device>.oscquery.json`, holding
/// `pub mod <device>`: for every string address that lists its values, an enum of them, with
/// its address, its type tag and the words for each; for every other address, a unit struct
/// with its address and type tag; for an address sent at a rate, its `RATE_HZ`; and `PATHS`,
/// every address.
///
/// The reader is found in `clockwork/`, beside the `fixtures/` the file is in.
pub fn osc(file: &Path) {
    let (name, device) = names(file);
    let leaves = read_leaves(file);
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    fs::write(
        out_dir.join(format!("osc_{device}.rs")),
        render(device, name, &leaves),
    )
    .unwrap();
}

/// Whether `file` is a shared subtree, such as `sys.oscquery.json`, that devices implement,
/// rather than a device's own, as `oscquery.py` decides it.
pub fn is_shared(file: &Path) -> bool {
    match crate::read_with(&reader(file), file, &["--shared"]).trim() {
        "shared" => true,
        "device" => false,
        other => panic!(
            "oscquery.py printed {other:?} for {} --shared",
            file.display()
        ),
    }
}

/// Generates `shared_<name>.rs` in `OUT_DIR` from a shared file, `<name>.oscquery.json`,
/// holding `pub mod <name>`: for every address, a function from the device's name to it,
/// named for the address after `{device}`, and its inverse, `<function>_device`, the device an
/// address is that leaf of. `sys::board("brain")` is `/sys/brain/board`, and
/// `sys::board_device("/sys/brain/board")` is `Some("brain")`.
pub fn shared(file: &Path) {
    let (name, block) = names(file);
    let mut out = String::new();
    writeln!(
        out,
        "// Generated from {name} by clockwork/oscquery.py. Do not edit."
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(out, "pub mod {block} {{").unwrap();
    for leaf in read_leaves(file) {
        let (before, after) = leaf
            .path
            .split_once(DEVICE)
            .unwrap_or_else(|| panic!("{}: {} has no {DEVICE}", file.display(), leaf.path));
        let function = after.trim_start_matches('/').replace('/', "_");
        writeln!(out, "    /// `{}`, for `device`.", leaf.path).unwrap();
        writeln!(out, "    pub fn {function}(device: &str) -> String {{").unwrap();
        writeln!(out, "        format!(\"{before}{{device}}{after}\")").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(
            out,
            "    /// The device `address` is `{}` of, if it is.",
            leaf.path
        )
        .unwrap();
        writeln!(
            out,
            "    pub fn {function}_device(address: &str) -> Option<&str> {{"
        )
        .unwrap();
        writeln!(
            out,
            "        address.strip_prefix({before:?})?.strip_suffix({after:?}).filter(|d| !d.is_empty() && !d.contains('/'))"
        )
        .unwrap();
        writeln!(out, "    }}").unwrap();
    }
    writeln!(out, "}}").unwrap();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    fs::write(out_dir.join(format!("shared_{block}.rs")), out).unwrap();
}

/// What a shared file names the device that implements it by.
const DEVICE: &str = "{device}";

/// The file's name, and the device or block it is for.
fn names(file: &Path) -> (&str, &str) {
    let name = file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_else(|| panic!("{} has no file name", file.display()));
    let device = name
        .strip_suffix(".oscquery.json")
        .unwrap_or_else(|| panic!("{} is not named <device>.oscquery.json", file.display()));
    (name, device)
}

/// The reader, in `clockwork/`, beside the `fixtures/` the file is in.
fn reader(file: &Path) -> PathBuf {
    file.parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| panic!("{} is not in fixtures/ beside clockwork/", file.display()))
        .join("clockwork")
        .join("oscquery.py")
}

/// Every leaf of `file`, the shared files it implements merged in. The whole of `fixtures/`
/// is watched, because a device's file reads the shared ones beside it.
fn read_leaves(file: &Path) -> Vec<Leaf> {
    let reader = reader(file);
    println!(
        "cargo:rerun-if-changed={}",
        file.parent().unwrap().display()
    );
    println!("cargo:rerun-if-changed={}", reader.display());
    crate::read_with(&reader, file, &[])
        .lines()
        .map(|line| {
            let mut words = line.split(' ').map(str::to_owned);
            let mut next = || {
                words.next().unwrap_or_else(|| {
                    panic!("oscquery.py printed {line:?} for {}", file.display())
                })
            };
            let (path, kind, how) = (next(), next(), next());
            Leaf {
                path,
                kind,
                how,
                values: words.collect(),
            }
        })
        .collect()
}

fn render(device: &str, source: &str, leaves: &[Leaf]) -> String {
    let mut out = String::new();
    writeln!(
        out,
        "// Generated from {source} by clockwork/oscquery.py. Do not edit."
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(out, "#[allow(dead_code)]").unwrap();
    writeln!(out, "pub mod {device} {{").unwrap();
    for (i, leaf) in leaves.iter().enumerate() {
        let name = camel(&leaf.path.trim_start_matches('/').replace('/', "_"));
        if i > 0 {
            writeln!(out).unwrap();
        }
        if leaf.values.is_empty() {
            writeln!(out, "    /// `{}`'s address and type tag.", leaf.path).unwrap();
            writeln!(out, "    pub struct {name};").unwrap();
            writeln!(out).unwrap();
            writeln!(out, "    impl {name} {{").unwrap();
            writeln!(out, "        pub const PATH: &str = {:?};", leaf.path).unwrap();
            writeln!(out, "        pub const TAG: &str = \",{}\";", leaf.kind).unwrap();
            if let Some(rate) = leaf.rate() {
                writeln!(out, "{rate}").unwrap();
            }
            writeln!(out, "    }}").unwrap();
            continue;
        }
        let variants: Vec<String> = leaf.values.iter().map(|v| camel(v)).collect();
        writeln!(
            out,
            "    /// `{}`'s values, in the order the file lists them.",
            leaf.path
        )
        .unwrap();
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
        if let Some(rate) = leaf.rate() {
            writeln!(out, "{rate}").unwrap();
        }
        writeln!(out).unwrap();
        writeln!(
            out,
            "        /// The value a word on the wire names, if it names one."
        )
        .unwrap();
        writeln!(
            out,
            "        pub fn from_word(word: &str) -> Option<Self> {{"
        )
        .unwrap();
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
    writeln!(out).unwrap();
    writeln!(out, "    /// Every address in the file, in its order.").unwrap();
    writeln!(out, "    pub const PATHS: &[&str] = &[").unwrap();
    for leaf in leaves {
        writeln!(out, "        {:?},", leaf.path).unwrap();
    }
    writeln!(out, "    ];").unwrap();
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
