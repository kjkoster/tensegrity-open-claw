//! Ingests the patch and the scenes from a QLC+ workspace at build time.
//!
//! One rig, one workspace: each rig crate's build script calls [`ingest`] with its own
//! `.qxw`, and the fixtures it names are generated inside that rig's crate. A rig can
//! therefore only reach the fixtures it patched — the other rig's constants do not exist
//! in its address space to reach for.
//!
//! QLC+ stores scene values **fixture-relative**:
//!
//!     <FixtureVal ID="3">0,255,1,0</FixtureVal>
//!
//! means "on the fixture whose QLC+ ID is 3, set its channel 0 to 255 and its channel 1
//! to 0". Those numbers are meaningless without the `<Fixture>` patch in the same file,
//! which is why the patch is authoritative: this script resolves them through it —
//! universe, start address, channel count — into absolute DMX slot indices, the only
//! thing the daemon speaks.
//!
//! Ingesting at build time rather than committing a generated file means the workspace
//! and the daemon cannot drift apart: there is no stale copy to go stale. `cargo` is
//! told to watch the workspace, so saving in QLC+ is enough to make the next build pick
//! the change up.
//!
//! Every malformed case below panics. A build script that panics fails the build, which
//! is the loudest and earliest place to catch a workspace that says something the daemon
//! cannot honour.
//!
//! Format notes, verified against QLC+ 5.2.1:
//!
//!   * QLC+ 5 writes the workspace with a **default XML namespace**
//!     (`http://www.qlcplus.org/Workspace`). A lookup that does not account for it
//!     matches nothing, which reads as "empty workspace" and would yield a valid, empty,
//!     wrong table. Every lookup here therefore goes through `tag_name().name()`, which
//!     is the local name with the namespace already stripped, and a missing `<Engine>`
//!     or an empty patch is fatal rather than empty.
//!   * `<Address>` is **0-based**: DMX address 1 on the wire is `<Address>0</Address>`.
//!   * `<Universe>` is the 0-based index into the workspace's universe list. Index 0 is
//!     the universe whose E1.31 output carries universe 1 on the wire.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

/// The daemon sends exactly one universe (`cortex::config::UNIVERSE`). In the workspace that
/// is the universe at 0-based index 0.
const WORKSPACE_UNIVERSE_INDEX: u32 = 0;

/// Slots in a DMX-512 universe. Spelled out rather than borrowed from the crate that
/// drives the wire: a build script may only use its build-dependencies, and pulling in a
/// serial-port crate to read one number from the protocol would be a poor trade.
const UNIVERSE_SLOTS: u32 = 512;

/// One row of the patch: what a QLC+ fixture ID means in absolute DMX slots.
struct Patched {
    name: String,
    /// The Rust type this fixture is an instance of, shared by every fixture patched to the
    /// same model and mode: three Yaras are three `Yara` constants, not three types.
    /// Assigned by `assign_type_names`, once the whole patch is known.
    type_name: String,
    manufacturer: String,
    model: String,
    mode: String,
    address: u32,
    channels: u32,
    /// The mode's channels, resolved against the fixture definition, in mode order. Empty
    /// until `resolve_definitions` fills it in. This stays the wire's view of the fixture:
    /// one entry per DMX slot it occupies, whatever the generated struct groups together.
    resolved: Vec<ResolvedChannel>,
    /// The struct's fields, over the channels above. A fine channel has no field of its own
    /// — it belongs to the coarse channel it extends.
    fields: Vec<Field>,
    /// How far the head moves, where the definition says it is a head at all.
    focus: Option<Focus>,
}

/// One channel of a patched fixture's mode, resolved through its `.qxf`.
struct ResolvedChannel {
    /// The channel's name in the definition, for doc comments.
    name: String,
    /// Absolute 0-based slot.
    slot: u32,
    /// The channel's `Preset` attribute, if it has one.
    preset: Option<String>,
    /// What the definition says the channel does, band by band.
    capabilities: Vec<Capability>,
}

/// One field of the generated struct, over the channels it is made of.
enum Field {
    /// A channel that stands alone, at `resolved[index]`.
    Single { index: usize, name: String },
    /// A coarse channel and the fine channel extending it, as one 16-bit field.
    Pair {
        coarse: usize,
        fine: usize,
        name: String,
    },
}

impl Field {
    /// The index of the channel the field is named after, which carries its preset and its
    /// capabilities.
    fn head(&self) -> usize {
        match self {
            Field::Single { index, .. } => *index,
            Field::Pair { coarse, .. } => *coarse,
        }
    }

    fn name(&self) -> &str {
        match self {
            Field::Single { name, .. } | Field::Pair { name, .. } => name,
        }
    }

    fn set_name(&mut self, new: String) {
        match self {
            Field::Single { name, .. } | Field::Pair { name, .. } => *name = new,
        }
    }
}

/// One band of a channel's range, as the definition states it.
#[derive(Clone)]
struct Capability {
    min: u8,
    max: u8,
    preset: Option<String>,
    name: String,
}

/// A fixture's travel, from `<Physical><Focus>`. Only heads have one: `Type="Fixed"` (or a
/// zero range, which is how QLC+ writes "not applicable") yields `None`, and a fixture with
/// no focus generates no position at all — the same way one with no red channel gets no
/// `red`.
#[derive(Clone)]
struct Focus {
    pan_max_deg: f64,
    tilt_max_deg: f64,
}

/// A parsed `.qxf`: what each named channel is, and which channels each mode uses.
struct Definition {
    path: PathBuf,
    /// Channel name → what the definition says about it.
    channels: BTreeMap<String, ChannelDef>,
    /// Mode name → its channel names, in `Number` order.
    modes: BTreeMap<String, Vec<String>>,
    focus: Option<Focus>,
}

/// One `<Channel>` of a definition.
struct ChannelDef {
    preset: Option<String>,
    capabilities: Vec<Capability>,
}

impl Patched {
    /// Absolute 0-based slot for a fixture-relative channel offset. The inverse of the
    /// absolute slot a `qlc_plus::Channel` holds: QLC+ already stores the
    /// start address 0-based.
    fn slot(&self, offset: u32) -> u32 {
        self.address + offset
    }
}

/// Generates `patch.rs` and `scenes.rs` into `OUT_DIR` from one QLC+ workspace.
///
/// The fixture definitions are taken from the `fixtures/` directory beside the workspace,
/// which is one directory for every rig: a CLF Yara definition is the same object whichever
/// sculpture it hangs on. Each rig reads every definition and resolves only what its own
/// patch names.
pub fn ingest(workspace: &Path) {
    println!("cargo:rerun-if-changed={}", workspace.display());

    let xml = fs::read_to_string(workspace).unwrap_or_else(|e| {
        panic!(
            "cannot read the QLC+ workspace at {}: {e}\n\
             The rig's patch and scenes come from that file. deploy.sh rsyncs the whole \
             repository to the Pi; if you are building by hand, make sure it is there.",
            workspace.display()
        )
    });
    // QLC+ writes a `<!DOCTYPE Workspace>` declaration, and roxmltree rejects any DTD by
    // default as a hardening measure. Allowing it is safe here: the DTD is an empty
    // internal subset that declares no entities, and roxmltree keeps its billion-laughs
    // protection regardless.
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..roxmltree::ParsingOptions::default()
    };
    let doc = roxmltree::Document::parse_with_options(&xml, options)
        .unwrap_or_else(|e| panic!("{} is not valid XML: {e}", workspace.display()));

    let engine = doc
        .root_element()
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == "Engine")
        .unwrap_or_else(|| {
            panic!(
                "{} has no <Engine> element — is it a QLC+ workspace?",
                workspace.display()
            )
        });

    let mut patch = parse_patch(engine);
    let scenes = parse_scenes(engine, &patch);

    // The workspace names a definition; the definition says what the channels mean. Both
    // are needed before a fixture can be given typed channels, so this runs before render.
    let fixtures_dir = workspace
        .parent()
        .unwrap_or_else(|| panic!("{} has no parent directory", workspace.display()))
        .join("fixtures");
    println!("cargo:rerun-if-changed={}", fixtures_dir.display());
    resolve_definitions(&mut patch, &fixtures_dir);
    assign_type_names(&mut patch);

    // The generated file names the workspace it came from, so a stray copy found on its own
    // still says which rig it belongs to.
    let source = workspace
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_else(|| panic!("{} has no file name", workspace.display()));

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    fs::write(out_dir.join("rig.rs"), render_rig(&patch, &scenes, source)).unwrap();
}

/// Renders the whole of a rig's generated half: both modules and the check between them.
///
/// One file rather than two, included at the rig crate's root, because the two halves are
/// cross-checked against each other and that check is generated from numbers only this
/// function has. Splitting them would put the assertion in a hand-written file, where it is
/// a line each rig has to remember to copy.
fn render_rig(
    patch: &BTreeMap<u32, Patched>,
    scenes: &[(String, Vec<(u32, u8)>)],
    source: &str,
) -> String {
    let mut out = String::new();
    writeln!(out, "// Generated from {source}. Do not edit.").unwrap();
    writeln!(out).unwrap();
    write_module(&mut out, "patch", &render_patch(patch));
    write_module(&mut out, "scenes", &render_scenes(scenes));
    out
}

/// Wraps a rendered body in `pub mod <name> { … }`, indenting it so the generated file still
/// reads like Rust when someone opens it under `target/`.
fn write_module(out: &mut String, name: &str, body: &str) {
    writeln!(out, "pub mod {name} {{").unwrap();
    for line in body.lines() {
        if line.is_empty() {
            writeln!(out).unwrap();
        } else {
            writeln!(out, "    {line}").unwrap();
        }
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

/// Loads every `.qxf` in `dir`, keyed by manufacturer and model.
fn load_definitions(dir: &Path) -> BTreeMap<(String, String), Definition> {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| {
        panic!(
            "cannot read the fixture definitions at {}: {e}\n\
             Every fixture patched in the workspace needs its .qxf committed there — the \
             Pi has no QLC+ installed to borrow them from.",
            dir.display()
        )
    });

    let mut definitions = BTreeMap::new();
    for entry in entries {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("qxf") {
            continue;
        }

        let xml = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let options = roxmltree::ParsingOptions {
            allow_dtd: true,
            ..roxmltree::ParsingOptions::default()
        };
        let doc = roxmltree::Document::parse_with_options(&xml, options)
            .unwrap_or_else(|e| panic!("{} is not valid XML: {e}", path.display()));
        let root = doc.root_element();

        let text = |name: &str| -> String {
            root.children()
                .find(|n| n.is_element() && n.tag_name().name() == name)
                .and_then(|n| n.text())
                .unwrap_or_else(|| panic!("{} has no <{name}>", path.display()))
                .trim()
                .to_string()
        };
        let manufacturer = text("Manufacturer");
        let model = text("Model");

        let mut channels = BTreeMap::new();
        for channel in root
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "Channel")
        {
            let name = channel
                .attribute("Name")
                .unwrap_or_else(|| panic!("{}: a <Channel> has no Name", path.display()))
                .trim()
                .to_string();

            // A channel either carries a Preset and no capabilities, or spells its bands
            // out. Both forms say what the channel does; only the second says where.
            let mut capabilities = Vec::new();
            for capability in channel
                .children()
                .filter(|n| n.is_element() && n.tag_name().name() == "Capability")
            {
                let bound = |attribute: &str| -> u8 {
                    capability
                        .attribute(attribute)
                        .unwrap_or_else(|| {
                            panic!(
                                "{}: a <Capability> of {name:?} has no {attribute}",
                                path.display()
                            )
                        })
                        .trim()
                        .parse()
                        .unwrap_or_else(|e| {
                            panic!(
                                "{}: {name:?} has a <Capability> with a non-byte {attribute}: {e}",
                                path.display()
                            )
                        })
                };
                let (min, max) = (bound("Min"), bound("Max"));
                if min > max {
                    panic!(
                        "{}: {name:?} has a <Capability> running {min}..{max}, backwards",
                        path.display()
                    );
                }
                capabilities.push(Capability {
                    min,
                    max,
                    preset: capability.attribute("Preset").map(str::to_string),
                    name: capability.text().unwrap_or_default().trim().to_string(),
                });
            }

            channels.insert(
                name,
                ChannelDef {
                    preset: channel.attribute("Preset").map(str::to_string),
                    capabilities,
                },
            );
        }

        let mut modes = BTreeMap::new();
        for mode in root
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "Mode")
        {
            let name = mode
                .attribute("Name")
                .unwrap_or_else(|| panic!("{}: a <Mode> has no Name", path.display()))
                .trim()
                .to_string();
            // A mode lists its channels with an explicit Number; sort by it rather than
            // trusting document order, because the offset is what addresses the slot.
            let mut numbered: Vec<(u32, String)> = mode
                .children()
                .filter(|n| n.is_element() && n.tag_name().name() == "Channel")
                .map(|n| {
                    let number = n
                        .attribute("Number")
                        .unwrap_or_else(|| {
                            panic!("{}: mode {name:?} has a <Channel> with no Number", path.display())
                        })
                        .parse()
                        .unwrap_or_else(|e| panic!("{}: bad Channel Number: {e}", path.display()));
                    (number, n.text().unwrap_or_default().trim().to_string())
                })
                .collect();
            numbered.sort_by_key(|(number, _)| *number);
            modes.insert(name, numbered.into_iter().map(|(_, name)| name).collect());
        }

        // A head's travel, where the definition declares one. QLC+ writes `Type="Fixed"` with
        // zero maxima for everything that does not move, so both tests are the same test.
        let focus = root
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "Physical")
            .and_then(|physical| {
                physical
                    .children()
                    .find(|n| n.is_element() && n.tag_name().name() == "Focus")
            })
            .and_then(|element| {
                let degrees = |attribute: &str| -> f64 {
                    element
                        .attribute(attribute)
                        .unwrap_or("0")
                        .trim()
                        .parse()
                        .unwrap_or_else(|e| {
                            panic!("{}: <Focus> has a non-numeric {attribute}: {e}", path.display())
                        })
                };
                let (pan_max_deg, tilt_max_deg) = (degrees("PanMax"), degrees("TiltMax"));
                (pan_max_deg > 0.0 && tilt_max_deg > 0.0).then_some(Focus {
                    pan_max_deg,
                    tilt_max_deg,
                })
            });

        if let Some(previous) = definitions.insert(
            (manufacturer.clone(), model.clone()),
            Definition { path: path.clone(), channels, modes, focus },
        ) {
            panic!(
                "{manufacturer} {model} is defined twice: {} and {}",
                previous.path.display(),
                path.display()
            );
        }
    }
    definitions
}

/// Resolves every patched fixture against its definition, filling in typed channels.
fn resolve_definitions(patch: &mut BTreeMap<u32, Patched>, dir: &Path) {
    let definitions = load_definitions(dir);

    for fixture in patch.values_mut() {
        let key = (fixture.manufacturer.clone(), fixture.model.clone());
        let definition = definitions.get(&key).unwrap_or_else(|| {
            let mut known: Vec<String> = definitions
                .keys()
                .map(|(m, model)| format!("{m} / {model}"))
                .collect();
            known.sort();
            panic!(
                "no fixture definition for {} / {} (patched as {:?}).\n\
                 {} holds: {}\n\
                 Export the .qxf from QLC+ into that directory and commit it — the Pi \
                 builds the daemon and has no QLC+ library to fall back on.",
                fixture.manufacturer,
                fixture.model,
                fixture.name,
                dir.display(),
                if known.is_empty() { "nothing".into() } else { known.join(", ") },
            )
        });

        let mode = definition.modes.get(&fixture.mode).unwrap_or_else(|| {
            let mut known: Vec<&str> = definition.modes.keys().map(String::as_str).collect();
            known.sort();
            panic!(
                "{} / {} has no mode {:?} (patched as {:?}). {} defines: {}",
                fixture.manufacturer,
                fixture.model,
                fixture.mode,
                fixture.name,
                definition.path.display(),
                known.join(", "),
            )
        });

        // The workspace stores the channel count too. QLC+ derives it from the mode, so a
        // mismatch means the workspace was hand-edited into disagreeing with the
        // definition — exactly the drift this resolution exists to catch.
        if mode.len() as u32 != fixture.channels {
            panic!(
                "{:?} is patched as {} channels but mode {:?} of {} / {} has {}. \
                 Re-pick the mode in QLC+'s Fixture Manager so the two agree.",
                fixture.name,
                fixture.channels,
                fixture.mode,
                fixture.manufacturer,
                fixture.model,
                mode.len(),
            )
        }

        let resolved: Vec<ResolvedChannel> = mode
            .iter()
            .enumerate()
            .map(|(offset, channel_name)| {
                let channel = definition.channels.get(channel_name).unwrap_or_else(|| {
                    panic!(
                        "{}: mode {:?} lists a channel {channel_name:?} the definition does \
                         not declare",
                        definition.path.display(),
                        fixture.mode,
                    )
                });
                ResolvedChannel {
                    name: channel_name.clone(),
                    slot: fixture.slot(offset as u32),
                    preset: channel.preset.clone(),
                    capabilities: channel.capabilities.clone(),
                }
            })
            .collect();

        let mut fields = pair_fine_channels(&resolved);

        // Prefer the preset's role as the field name — a channel named "R" carrying
        // IntensityRed still becomes `red`. Channels QLC+ gives no preset (wheels, gobos,
        // maintenance) fall back to their own name.
        for field in fields.iter_mut() {
            let channel = &resolved[field.head()];
            let name = role_of(channel.preset.as_deref())
                .map(str::to_string)
                .unwrap_or_else(|| field_of(&channel.name));
            field.set_name(name);
        }

        // Two channels can share a preset — pixel-mapped modes give every cell an
        // IntensityRed. Where that happens the role is not a unique name, so fall back to
        // the channel names, which QLC+ does keep distinct within a mode.
        let clashes: Vec<String> = fields
            .iter()
            .filter(|f| fields.iter().filter(|o| o.name() == f.name()).count() > 1)
            .map(|f| f.name().to_string())
            .collect();
        for field in fields.iter_mut() {
            if clashes.contains(&field.name().to_string()) {
                let name = field_of(&resolved[field.head()].name);
                field.set_name(name);
            }
        }
        for (index, field) in fields.iter().enumerate() {
            if let Some(other) = fields[..index].iter().find(|o| o.name() == field.name()) {
                panic!(
                    "{:?}: channels {:?} and {:?} both map to the field `{}`. Rename one in {}.",
                    fixture.name,
                    resolved[other.head()].name,
                    resolved[field.head()].name,
                    field.name(),
                    definition.path.display(),
                );
            }
        }

        fixture.focus = definition.focus.clone();
        fixture.resolved = resolved;
        fixture.fields = fields;
    }
}

/// Groups a mode's channels into the fields the generated struct gets, folding every fine
/// channel into the coarse one it extends.
///
/// The pairing is read off the presets rather than the channel names: QLC+ spells a fine
/// channel as its coarse preset with `Fine` on the end, across every attribute that has one,
/// so this holds for a dimmer or a colour wheel as much as for pan. Names are no use for it
/// — "Pan Fine", "Pan fine" and "PAN FINE" are all real, and a definition is free to call it
/// something else entirely.
///
/// A fine channel whose coarse partner is not in the same mode keeps a field of its own. It
/// is then an 8-bit channel that happens to be called fine, which is what the definition
/// said, and the alternative is a build that fails on a fixture that works.
///
/// Which channel comes first in the mode does not matter. Definitions conventionally list a
/// coarse channel and then its fine one, but nothing enforces that, and a pairing that only
/// looked forward would fold a channel it had already emitted a field for.
fn pair_fine_channels(resolved: &[ResolvedChannel]) -> Vec<Field> {
    // The coarse channel each fine channel extends, if that channel is in this mode too.
    let extends = |index: usize| -> Option<usize> {
        let coarse = resolved[index]
            .preset
            .as_deref()?
            .strip_suffix("Fine")
            .filter(|coarse| !coarse.is_empty())?;
        resolved
            .iter()
            .position(|channel| channel.preset.as_deref() == Some(coarse))
    };

    let fine_of: BTreeMap<usize, usize> = (0..resolved.len())
        .filter_map(|index| extends(index).map(|coarse| (coarse, index)))
        .collect();
    let folded: BTreeSet<usize> = fine_of.values().copied().collect();

    // Fields land in the order the mode puts the *coarse* channel, which is the order the
    // fixture's manual lists them in.
    (0..resolved.len())
        .filter(|index| !folded.contains(index))
        .map(|index| match fine_of.get(&index) {
            Some(&fine) => Field::Pair {
                coarse: index,
                fine,
                name: String::new(),
            },
            None => Field::Single {
                index,
                name: String::new(),
            },
        })
        .collect()
}

/// Names the Rust type each fixture is an instance of.
///
/// The type describes a *kind* of fixture, so it comes from the model — every Yara in the
/// patch is a `Yara`, however many there are. A model patched in two different modes is two
/// different shapes, though, so those get the mode appended to tell them apart; the common
/// case of one mode per model keeps the short name.
fn assign_type_names(patch: &mut BTreeMap<u32, Patched>) {
    let mut modes_per_model: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for fixture in patch.values() {
        modes_per_model
            .entry(&fixture.model)
            .or_default()
            .insert(&fixture.mode);
    }
    let ambiguous: BTreeSet<String> = modes_per_model
        .iter()
        .filter(|(_, modes)| modes.len() > 1)
        .map(|(model, _)| (*model).to_string())
        .collect();

    let mut named: BTreeMap<String, (String, String)> = BTreeMap::new();
    for fixture in patch.values_mut() {
        let mut type_name = type_name_of(&fixture.model);
        if ambiguous.contains(&fixture.model) {
            type_name.push_str(&type_name_of(&fixture.mode));
        }
        if !type_name.starts_with(|c: char| c.is_ascii_alphabetic()) {
            panic!(
                "model {:?} does not yield a usable Rust type name (got {type_name:?}). \
                 Rename the model in its .qxf to something starting with a letter.",
                fixture.model,
            );
        }

        // Two different models can still collide once punctuation is stripped. Catch it here
        // rather than emit a file that fails to compile for a reason nobody can read.
        let shape = (fixture.model.clone(), fixture.mode.clone());
        match named.get(&type_name) {
            Some(other) if *other != shape => panic!(
                "{} {:?} and {} {:?} both map to the Rust type `{type_name}`. \
                 Rename one model in its .qxf.",
                other.0, other.1, fixture.model, fixture.mode,
            ),
            _ => {
                named.insert(type_name.clone(), shape);
            }
        }
        fixture.type_name = type_name;
    }
}

/// The field name a QLC+ preset earns, for the Intensity family — the roles a colour-mixing
/// fixture is built from. `None` means the channel is named after itself instead.
///
/// Everything else QLC+ can express (colour wheels, gobos, strobes, maintenance channels) is
/// deliberately unmapped: those are indexed bands rather than continuous levels, so naming
/// one `red` would be a lie. They keep their own channel name as the field.
fn role_of(preset: Option<&str>) -> Option<&'static str> {
    match preset {
        Some("IntensityMasterDimmer") => Some("dimmer"),
        Some("IntensityDimmer") => Some("intensity"),
        Some("IntensityRed") => Some("red"),
        Some("IntensityGreen") => Some("green"),
        Some("IntensityBlue") => Some("blue"),
        Some("IntensityWhite") => Some("white"),
        Some("IntensityAmber") => Some("amber"),
        Some("IntensityUV") => Some("uv"),
        Some("IntensityCyan") => Some("cyan"),
        Some("IntensityMagenta") => Some("magenta"),
        Some("IntensityYellow") => Some("yellow"),
        Some("IntensityHue") => Some("hue"),
        Some("IntensitySaturation") => Some("saturation"),
        Some("IntensityValue") => Some("value"),
        Some("IntensityLightness") => Some("lightness"),
        _ => None,
    }
}

/// Turns a channel name into a Rust field name: "Gobo rotation" → `gobo_rotation`.
fn field_of(name: &str) -> String {
    let mut field = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            field.push(c.to_ascii_lowercase());
        } else if !field.ends_with('_') && !field.is_empty() {
            field.push('_');
        }
    }
    while field.ends_with('_') {
        field.pop();
    }
    // A field may not start with a digit, and may not be a keyword. Both are rare enough
    // in channel names that a prefix is friendlier than refusing to build.
    if field.starts_with(|c: char| c.is_ascii_digit()) {
        field.insert(0, 'c');
    }
    if RUST_KEYWORDS.contains(&field.as_str()) {
        field.push('_');
    }
    field
}

/// Keywords a channel name could plausibly collide with — "Move" and "Loop" are real
/// channel names on moving heads.
const RUST_KEYWORDS: [&str; 12] = [
    "move", "loop", "type", "ref", "box", "match", "where", "in", "fn", "mod", "self", "static",
];

/// Turns a QLC+ fixture name into a Rust constant identifier: "Yara 1" → `YARA_1`.
///
/// The name the operator types into QLC+ is the name the daemon knows the fixture by, so
/// this mapping has to be total and collision-free — `parse_patch` rejects any name that
/// does not survive it.
fn ident_of(name: &str) -> String {
    let mut ident = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            ident.push(c.to_ascii_uppercase());
        } else if !ident.ends_with('_') && !ident.is_empty() {
            ident.push('_');
        }
    }
    while ident.ends_with('_') {
        ident.pop();
    }
    ident
}

/// Turns free text into a Rust type name: "Yara" → `Yara`, "Space-4 Laser" → `Space4Laser`.
/// The caller checks the result is a usable identifier.
fn type_name_of(text: &str) -> String {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + &chars.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect()
}

/// Builds the fixture ID → patch-row table from the `<Fixture>` elements.
fn parse_patch(engine: roxmltree::Node) -> BTreeMap<u32, Patched> {
    // Annotated because the collision check below reads the map before anything is inserted.
    let mut patch: BTreeMap<u32, Patched> = BTreeMap::new();

    for element in engine
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "Fixture")
    {
        let field = |name: &str| -> String {
            element
                .children()
                .find(|node| node.is_element() && node.tag_name().name() == name)
                .and_then(|node| node.text())
                .unwrap_or_else(|| panic!("a <Fixture> in the workspace is missing <{name}>"))
                .trim()
                .to_string()
        };
        let number = |name: &str| -> u32 {
            field(name)
                .parse()
                .unwrap_or_else(|e| panic!("<Fixture> has a non-numeric <{name}>: {e}"))
        };

        let id = number("ID");
        let name = field("Name");
        let universe = number("Universe");
        let address = number("Address");
        let channels = number("Channels");

        if universe != WORKSPACE_UNIVERSE_INDEX {
            panic!(
                "fixture {id} {name:?} is patched to workspace universe index {universe}. \
                 The daemon sends exactly one universe (index {WORKSPACE_UNIVERSE_INDEX}, \
                 wire universe 1); teaching Scene about universes is a deliberate change, \
                 not something to fold in silently."
            );
        }
        if channels == 0 {
            // Nothing downstream expects an empty fixture: the daemon's slot span is derived
            // from the channels a fixture occupies, and one occupying none is a patch entry
            // for a fixture that cannot be driven. The mode-vs-count check further down
            // cannot catch it either, because zero equals zero.
            panic!(
                "fixture {id} {name:?} is patched with zero channels. Re-pick its mode in \
                 QLC+'s Fixture Manager."
            );
        }
        if address + channels > UNIVERSE_SLOTS {
            panic!(
                "fixture {id} {name:?} spans wire slots {}..{}, past the end of the universe",
                address + 1,
                address + channels
            );
        }
        let ident = ident_of(&name);
        if !ident.starts_with(|c: char| c.is_ascii_alphabetic()) {
            panic!(
                "fixture {id} {name:?} does not yield a usable Rust name (got {ident:?}). \
                 Fixture names become the constants the daemon drives them by, so rename it \
                 in QLC+ to something starting with a letter."
            );
        }
        if let Some(clash) = patch.values().find(|p| ident_of(&p.name) == ident) {
            panic!(
                "fixtures {:?} and {name:?} both map to the constant {ident} — the daemon \
                 could not tell them apart. Rename one in QLC+.",
                clash.name
            );
        }
        let inserted = patch.insert(
            id,
            Patched {
                type_name: String::new(), // assign_type_names fills this in
                name,
                manufacturer: field("Manufacturer"),
                model: field("Model"),
                mode: field("Mode"),
                address,
                channels,
                resolved: Vec::new(),
                fields: Vec::new(),
                focus: None,
            },
        );
        if inserted.is_some() {
            panic!("duplicate fixture ID {id} in the workspace patch");
        }
    }

    if patch.is_empty() {
        panic!(
            "the workspace has no <Fixture> elements. The patch is authoritative — the \
             daemon's scenes are resolved through it — so an empty one is never right."
        );
    }

    // Overlapping fixtures are the one patch error the generated names cannot catch. Every
    // constant is well-formed on its own, so the build stays clean and only the wire is
    // wrong — two fixtures answering to the same slots, which reads as a broken fixture
    // rather than a broken patch. It takes the whole patch to see, so it cannot live in the
    // per-fixture loop above: the neighbour has not been parsed yet.
    let mut spans: Vec<&Patched> = patch.values().collect();
    spans.sort_by_key(|fixture| fixture.address);
    for pair in spans.windows(2) {
        let (first, second) = (pair[0], pair[1]);
        let first_end = first.address + first.channels;
        if first_end > second.address {
            panic!(
                "fixtures {:?} and {:?} overlap: {:?} spans wire slots {}..{} in {} mode, and \
                 {:?} starts at {}. Re-address one of them in QLC+.",
                first.name,
                second.name,
                first.name,
                first.address + 1,
                first_end,
                first.mode,
                second.name,
                second.address + 1
            );
        }
    }

    patch
}

/// Resolves every `<Function Type="Scene">` into a name and its absolute (slot, value)
/// pairs, in document order.
fn parse_scenes(engine: roxmltree::Node, patch: &BTreeMap<u32, Patched>) -> Vec<(String, Vec<(u32, u8)>)> {
    let mut scenes: Vec<(String, Vec<(u32, u8)>)> = Vec::new();

    for element in engine.children().filter(|node| {
        node.is_element()
            && node.tag_name().name() == "Function"
            && node.attribute("Type") == Some("Scene")
    }) {
        let name = element.attribute("Name").unwrap_or_default().trim();

        // Scene names are the stable keys: they are what crosses into Rust, while QLC+'s
        // numeric Function IDs shift whenever functions are deleted. Enforce the shape at
        // the boundary so a name can never need escaping or disambiguating downstream.
        if !is_snake_case(name) {
            panic!(
                "scene {name:?} is not snake_case. Scene names are the stable keys the \
                 daemon knows scenes by, so keep them to [a-z][a-z0-9_]* — rename it in \
                 QLC+'s Function Manager."
            );
        }
        if scenes.iter().any(|(seen, _)| seen == name) {
            panic!(
                "duplicate scene name {name:?}. QLC+ does not enforce unique Function \
                 names, but the daemon needs them unique — rename one in the Function \
                 Manager. (Note a Simple Desk dump always creates a new scene; it never \
                 updates an existing one.)"
            );
        }

        // A QLC+ channel group is a console-side abstraction — one fader driving several
        // channels — with no fixture-relative address to resolve through. Refuse it rather
        // than drop it silently and export a scene that is missing what the operator set.
        if element.children().any(|node| {
            node.is_element()
                && node.tag_name().name() == "ChannelGroupsVal"
                && node.text().is_some_and(|text| !text.trim().is_empty())
        }) {
            panic!(
                "scene {name:?} uses a QLC+ channel group, which has no DMX address to \
                 resolve through — set the fixture channels directly instead."
            );
        }

        // A BTreeMap both de-duplicates repeated channels within a scene (serialisation
        // noise, last one wins) and leaves the pairs sorted by slot, which keeps the
        // generated table stable and readable.
        let mut values: BTreeMap<u32, u8> = BTreeMap::new();

        for fixture_val in element
            .children()
            .filter(|node| node.is_element() && node.tag_name().name() == "FixtureVal")
        {
            let id: u32 = fixture_val
                .attribute("ID")
                .unwrap_or_else(|| panic!("scene {name:?} has a <FixtureVal> with no ID"))
                .parse()
                .unwrap_or_else(|e| panic!("scene {name:?} has a non-numeric FixtureVal ID: {e}"));
            let fixture = patch.get(&id).unwrap_or_else(|| {
                panic!("scene {name:?} references fixture ID {id}, which is not in the patch")
            });

            let body = fixture_val.text().unwrap_or_default().trim();
            if body.is_empty() {
                continue;
            }
            let numbers: Vec<u32> = body
                .split(',')
                .map(|part| {
                    part.trim().parse().unwrap_or_else(|e| {
                        panic!("scene {name:?}, fixture {id}: {part:?} is not a number: {e}")
                    })
                })
                .collect();
            if numbers.len() % 2 != 0 {
                panic!(
                    "scene {name:?}, fixture {id}: {} channel/value entries, which cannot \
                     pair up",
                    numbers.len()
                );
            }

            for pair in numbers.chunks_exact(2) {
                let (offset, value) = (pair[0], pair[1]);
                if offset >= fixture.channels {
                    panic!(
                        "scene {name:?} sets channel {offset} on fixture {id} {:?}, which \
                         the patch gives only {} channels — the scene and the patch \
                         disagree about that fixture's mode",
                        fixture.name, fixture.channels
                    );
                }
                let value = u8::try_from(value).unwrap_or_else(|_| {
                    panic!(
                        "scene {name:?}, fixture {id}, channel {offset}: {value} is not a \
                         DMX byte"
                    )
                });
                values.insert(fixture.slot(offset), value);
            }
        }

        scenes.push((name.to_string(), values.into_iter().collect()));
    }

    scenes
}

fn is_snake_case(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Whether a fixture can be aimed: its definition declares travel, and its mode carries pan
/// and tilt as 16-bit pairs.
///
/// Both halves are required, and a head failing either gets no position rather than a
/// half-working one. A `<Focus>` with no fine channels would be a head driven in 8-bit steps,
/// which is visible on any slow move; fine channels with no `<Focus>` would be a head whose
/// degrees mean nothing. Either way the fix is in the `.qxf`, and a missing `Position` is
/// what sends whoever hits it there.
fn aims(fixture: &Patched) -> bool {
    let pair = |name: &str| {
        fixture
            .fields
            .iter()
            .any(|field| matches!(field, Field::Pair { .. }) && field.name() == name)
    };
    fixture.focus.is_some() && pair("pan") && pair("tilt")
}

/// The expression reaching each of a mode's channels, in mode order, through `receiver`.
///
/// The generated file's one view of a fixture's addressing is the instance constant; every
/// other listing of its channels is written as a path into that constant rather than as the
/// slot again, so there is nothing for a second copy to disagree with.
fn channel_paths(fixture: &Patched, receiver: &str) -> Vec<String> {
    let mut paths = vec![String::new(); fixture.resolved.len()];
    for field in &fixture.fields {
        match field {
            Field::Single { index, name } => paths[*index] = format!("{receiver}.{name}"),
            Field::Pair { coarse, fine, name } => {
                paths[*coarse] = format!("{receiver}.{name}.coarse");
                paths[*fine] = format!("{receiver}.{name}.fine");
            }
        }
    }
    paths
}

/// The constant holding each channel's capability table, indexed in mode order. `None` where
/// the definition declares no bands for that channel.
///
/// Named after the struct field rather than the channel, because field names are already
/// checked unique within a type while two channel names punctuating down to one identifier
/// would quietly generate two constants with the same name.
fn capability_tables(shape: &Patched) -> Vec<Option<String>> {
    let mut labels = vec![String::new(); shape.resolved.len()];
    for field in &shape.fields {
        match field {
            Field::Single { index, name } => labels[*index] = name.clone(),
            Field::Pair { coarse, fine, name } => {
                labels[*coarse] = name.clone();
                labels[*fine] = format!("{name}_fine");
            }
        }
    }
    labels
        .iter()
        .enumerate()
        .map(|(index, label)| {
            let declared = !shape.resolved[index].capabilities.is_empty();
            declared.then(|| format!("CAPS_{}_{}", ident_of(&shape.type_name), ident_of(label)))
        })
        .collect()
}

/// Writes the values a type's channels are worth knowing by name.
///
/// Two kinds, answering two different questions. A **capability** constant names a band the
/// definition spells out, so the show can say which one it means and let the compiler check
/// that the fixture still has it — that is the vocabulary the wheels are selected from. A
/// **speed** constant resolves which end of a speed channel is fast, which the preset states
/// and no two manufacturers agree on; it is derived here rather than looked up at run time
/// because the answer cannot change while the binary runs.
fn write_value_constants(out: &mut String, shape: &Patched) {
    let tables = capability_tables(shape);

    // Every constant this type could carry, gathered before any is written, because whether
    // a name is usable depends on what else claims it. Repeated capability labels are real —
    // a shutter with four "shutter open" gaps between its strobe speeds, a maintenance
    // channel with six dead bands — and so are distinct labels that punctuate down to one
    // identifier. A name landing on more than one band would silently mean a particular one
    // of them, so the whole group goes unnamed and the table stays there to be read. The
    // check spans the type rather than the channel, since that is where the names collide.
    let mut constants: Vec<(usize, String, String, String)> = Vec::new();
    for field in &shape.fields {
        let index = field.head();
        let channel = &shape.resolved[index];
        let prefix = field.name().to_ascii_uppercase();

        // Which end of a speed channel is fast. QLC+ spells the direction into the preset
        // name, which is the only reason this transfers to a fixture nobody has measured.
        if let Some(preset) = channel.preset.as_deref().filter(|p| p.starts_with("Speed")) {
            let ends = if preset.ends_with("FastSlow") {
                Some((0, 255))
            } else if preset.ends_with("SlowFast") {
                Some((255, 0))
            } else {
                None
            };
            if let Some((fastest, slowest)) = ends {
                let doc = format!("{} at its fastest ({preset}).", channel.name);
                let value = format!("u8 = {fastest}");
                constants.push((index, format!("{prefix}_FASTEST"), doc, value));
                let doc = format!("{} at its slowest.", channel.name);
                let value = format!("u8 = {slowest}");
                constants.push((index, format!("{prefix}_SLOWEST"), doc, value));
            }
        }

        let Some(table) = &tables[index] else {
            continue;
        };
        for (position, capability) in channel.capabilities.iter().enumerate() {
            let ident = ident_of(&capability.name);
            if ident.is_empty() {
                continue;
            }
            let doc = format!(
                "{} — {} ({}–{}).",
                channel.name, capability.name, capability.min, capability.max
            );
            let value = format!("Capability = {table}[{position}]");
            constants.push((index, format!("{prefix}_{ident}"), doc, value));
        }
    }

    let mut claims: BTreeMap<&str, usize> = BTreeMap::new();
    for (_, name, ..) in &constants {
        *claims.entry(name).or_default() += 1;
    }

    let mut previous: Option<usize> = None;
    for (index, name, doc, value) in &constants {
        if claims[name.as_str()] > 1 {
            continue;
        }
        // A blank line between channels, so the block reads as one group per channel.
        if previous != Some(*index) {
            writeln!(out).unwrap();
            previous = Some(*index);
        }
        writeln!(out, "    /// {doc}").unwrap();
        writeln!(out, "    pub const {name}: {value};").unwrap();
    }
}

/// Renders the fixture table: one named constant per fixture in the QLC+ patch, a struct per
/// fixture *type* with a named field per channel, and the width of the frame they span.
///
/// Naming each fixture is what makes the workspace the source of truth, because it hands
/// every direction of drift to the compiler:
///
///   * Patch a **new** fixture in QLC+ that no code drives → its constant is unused, and the
///     build warns about dead code.
///   * Delete or rename a fixture that code drives → its constant is gone, and every use site
///     stops compiling.
///   * Repatch a fixture into a mode without a channel the code writes → the field is gone,
///     or the capability trait is no longer implemented, and again the use site fails.
///
/// That last one is why channels are typed rather than numbered. `PINSPOT.red` names the red
/// emitter because the definition says that channel of that mode is `IntensityRed`; there is
/// no offset in the source to get wrong, and a fixture with no red simply has no `red`.
fn render_patch(patch: &BTreeMap<u32, Patched>) -> String {
    let mut out = String::new();

    let mut fixtures: Vec<&Patched> = patch.values().collect();
    fixtures.sort_by_key(|fixture| fixture.address);

    // Import exactly the capability traits this patch implements. Naming them all in the
    // hand-written half would warn about the ones no patched fixture happens to have, and
    // that warning would be noise rather than the signal an unused import usually is.
    let implements = |field: &str| {
        fixtures
            .iter()
            .any(|fixture| fixture.fields.iter().any(|f| f.name() == field))
    };
    let mut imports = vec!["Channel", "PatchEntry"];
    if implements("red") && implements("green") && implements("blue") {
        imports.push("Rgb");
    }
    if implements("white") {
        imports.push("White");
    }
    if implements("dimmer") {
        imports.push("Dimmer");
    }
    let pairs = |fixture: &Patched| fixture.fields.iter().any(|f| matches!(f, Field::Pair { .. }));
    if fixtures.iter().any(|fixture| pairs(fixture)) {
        imports.push("Channel16");
    }
    if fixtures.iter().any(|fixture| aims(fixture)) {
        imports.push("Position");
    }
    if fixtures
        .iter()
        .any(|fixture| fixture.resolved.iter().any(|c| !c.capabilities.is_empty()))
    {
        imports.push("Capability");
    }
    imports.sort_unstable();
    writeln!(out, "use cortex::qlc_plus::{{{}}};", imports.join(", ")).unwrap();
    writeln!(out).unwrap();

    // One struct per fixture *type*, shared by every instance of it: three Yaras patched
    // the same way are three constants of one `Yara`, not three identical types. Grouped by
    // the type name `assign_type_names` worked out, keyed on model and mode.
    let mut types: BTreeMap<&str, Vec<&&Patched>> = BTreeMap::new();
    for fixture in &fixtures {
        types.entry(&fixture.type_name).or_default().push(fixture);
    }

    for (type_name, instances) in &types {
        // Every instance of a type came from the same model and mode, so they share a
        // channel layout; the first one describes the shape for all of them.
        let shape = instances[0];
        writeln!(
            out,
            "/// {} {}, {} — {} channel{}.",
            shape.manufacturer,
            shape.model,
            shape.mode,
            shape.channels,
            if shape.channels == 1 { "" } else { "s" },
        )
        .unwrap();
        writeln!(out, "pub struct {type_name} {{").unwrap();
        for field in &shape.fields {
            match field {
                Field::Single { index, name } => {
                    writeln!(out, "    /// {}.", shape.resolved[*index].name).unwrap();
                    writeln!(out, "    pub {name}: Channel,").unwrap();
                }
                Field::Pair { coarse, fine, name } => {
                    writeln!(
                        out,
                        "    /// {} and {}, as one 16-bit value.",
                        shape.resolved[*coarse].name, shape.resolved[*fine].name,
                    )
                    .unwrap();
                    writeln!(out, "    pub {name}: Channel16,").unwrap();
                }
            }
        }
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();

        // Positional access is a convenience for fixtures whose channels have no role to
        // name — a moving head's position channels, say. Which types want it is a property
        // of the hand-written code, not of the patch, so generating it everywhere and
        // letting it go unused is honest; the dead-code warning that matters is the one on
        // the fixture constant.
        writeln!(out, "#[allow(dead_code)]").unwrap();
        writeln!(out, "impl {type_name} {{").unwrap();
        writeln!(out, "    /// Channels the patched mode occupies.").unwrap();
        writeln!(out, "    pub const CHANNELS: usize = {};", shape.channels).unwrap();
        writeln!(out).unwrap();
        writeln!(out, "    /// Every channel, in mode order.").unwrap();
        writeln!(out, "    pub fn all(&self) -> [Channel; Self::CHANNELS] {{").unwrap();
        writeln!(out, "        [{}]", channel_paths(shape, "self").join(", ")).unwrap();
        writeln!(out, "    }}").unwrap();
        write_value_constants(&mut out, shape);
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();

        // Capability traits, implemented only where the mode actually carries the channels.
        // This is the type-safety the whole exercise is for: a fixture patched into a mode
        // without colour cannot be handed to code that mixes colour. One impl per type, so
        // it covers every instance.
        let has = |field: &str| shape.fields.iter().any(|f| f.name() == field);
        if aims(shape) {
            let focus = shape.focus.as_ref().expect("aims() checked the focus");
            writeln!(out, "impl Position for {type_name} {{").unwrap();
            writeln!(
                out,
                "    const PAN_RANGE_DEG: f64 = {:?};",
                focus.pan_max_deg
            )
            .unwrap();
            writeln!(
                out,
                "    const TILT_RANGE_DEG: f64 = {:?};",
                focus.tilt_max_deg
            )
            .unwrap();
            writeln!(out, "    fn pan(&self) -> Channel16 {{ self.pan }}").unwrap();
            writeln!(out, "    fn tilt(&self) -> Channel16 {{ self.tilt }}").unwrap();
            writeln!(out, "}}").unwrap();
        }
        if has("red") && has("green") && has("blue") {
            writeln!(out, "impl Rgb for {type_name} {{").unwrap();
            for colour in ["red", "green", "blue"] {
                writeln!(out, "    fn {colour}(&self) -> Channel {{ self.{colour} }}").unwrap();
            }
            writeln!(out, "}}").unwrap();
        }
        if has("white") {
            writeln!(out, "impl White for {type_name} {{").unwrap();
            writeln!(out, "    fn white(&self) -> Channel {{ self.white }}").unwrap();
            writeln!(out, "}}").unwrap();
        }
        if has("dimmer") {
            writeln!(out, "impl Dimmer for {type_name} {{").unwrap();
            writeln!(out, "    fn dimmer(&self) -> Channel {{ self.dimmer }}").unwrap();
            writeln!(out, "}}").unwrap();
        }
        writeln!(out).unwrap();
    }

    // The bands each channel of each *type* declares, hoisted out of the instances: four
    // heads of one model say the same thing about their colour wheel, and one table they all
    // point at is both smaller and impossible to have four opinions about.
    for instances in types.values() {
        let shape = instances[0];
        for (index, table) in capability_tables(shape).iter().enumerate() {
            let Some(table) = table else { continue };
            let channel = &shape.resolved[index];
            writeln!(
                out,
                "/// {} {} — the bands its definition declares.",
                shape.model, channel.name,
            )
            .unwrap();
            writeln!(
                out,
                "const {table}: [Capability; {}] = [",
                channel.capabilities.len(),
            )
            .unwrap();
            for capability in &channel.capabilities {
                writeln!(
                    out,
                    "    Capability {{ min: {}, max: {}, preset: {}, name: {:?} }},",
                    capability.min,
                    capability.max,
                    match &capability.preset {
                        Some(preset) => format!("Some({preset:?})"),
                        None => "None".to_string(),
                    },
                    capability.name,
                )
                .unwrap();
            }
            writeln!(out, "];").unwrap();
            writeln!(out).unwrap();
        }
    }

    // The instances: one constant per fixture in the workspace, each an instance of its
    // type with its own start address folded into every channel.
    for fixture in &fixtures {
        writeln!(
            out,
            "/// {} — DMX address {} (slots {}–{}).",
            fixture.name,
            fixture.address + 1,
            fixture.address + 1,
            fixture.address + fixture.channels,
        )
        .unwrap();
        writeln!(
            out,
            "pub const {}: {} = {} {{",
            ident_of(&fixture.name),
            fixture.type_name,
            fixture.type_name,
        )
        .unwrap();
        let tables = capability_tables(fixture);
        let channel = |index: usize| {
            let capabilities = match &tables[index] {
                Some(table) => format!("&{table}"),
                None => "&[]".to_string(),
            };
            format!("Channel::at({}, {capabilities})", fixture.resolved[index].slot)
        };
        for field in &fixture.fields {
            match field {
                Field::Single { index, name } => {
                    writeln!(out, "    {name}: {},", channel(*index)).unwrap();
                }
                Field::Pair { coarse, fine, name } => {
                    writeln!(
                        out,
                        "    {name}: Channel16::pair({}, {}),",
                        channel(*coarse),
                        channel(*fine),
                    )
                    .unwrap();
                }
            }
        }
        writeln!(out, "}};").unwrap();
        writeln!(out).unwrap();
    }

    // The same patch as plain data, so the daemon can log what it was built against
    // without a hand-maintained list of fixture names going stale beside it.
    // A static rather than a const: the rig hands this table to the frame loop as a
    // `&'static [PatchEntry]`, and a static has that lifetime outright instead of leaning on
    // a promotion rule.
    writeln!(out, "/// Every patched fixture, in address order.").unwrap();
    writeln!(
        out,
        "pub static PATCH: [PatchEntry; {}] = [",
        fixtures.len()
    )
    .unwrap();
    for fixture in &fixtures {
        writeln!(out, "    PatchEntry {{").unwrap();
        writeln!(out, "        name: {:?},", fixture.name).unwrap();
        writeln!(
            out,
            "        profile: {:?},",
            format!("{} {}, {}", fixture.manufacturer, fixture.model, fixture.mode)
        )
        .unwrap();
        writeln!(out, "        address: {},", fixture.address + 1).unwrap();
        // Read the channels back off the instance rather than restating the slots. The
        // constant above is the one place a fixture's addressing exists; a second literal
        // here would be a copy able to disagree with it.
        writeln!(
            out,
            "        channels: &[{}],",
            channel_paths(fixture, &ident_of(&fixture.name)).join(", ")
        )
        .unwrap();
        writeln!(out, "    }},").unwrap();
    }
    writeln!(out, "];").unwrap();
    writeln!(out).unwrap();

    let top = fixtures
        .iter()
        .map(|fixture| fixture.address + fixture.channels)
        .max()
        .unwrap_or(0);
    // The daemon derives the frame width from the channels the fixtures occupy, so this is
    // not what it sends; it exists for the scene bound below, where a compile-time number is
    // the only thing an assertion can be written against.
    writeln!(out, "/// Slots the sACN frame spans: 1 through the last patched slot.").unwrap();
    writeln!(out, "pub const DMX_SLOTS: usize = {top};").unwrap();

    out
}

/// Renders the scene table, and the one check that spans both halves of a rig.
///
/// Only the data is generated; the `Scene` type and everything that reads it stay
/// hand-written and reviewable in `cortex`.
fn render_scenes(scenes: &[(String, Vec<(u32, u8)>)]) -> String {
    let mut out = String::new();

    writeln!(out, "use cortex::scenes::Scene;").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "pub static SCENES: [Scene; {}] = [", scenes.len()).unwrap();
    for (name, values) in scenes {
        writeln!(out, "    Scene {{").unwrap();
        writeln!(out, "        name: \"{name}\",").unwrap();
        if values.is_empty() {
            writeln!(out, "        values: &[],").unwrap();
        } else {
            writeln!(out, "        values: &[").unwrap();
            // Six pairs per line, so a typical fixture's channels read as one row.
            for row in values.chunks(6) {
                let row: Vec<String> = row
                    .iter()
                    .map(|(slot, value)| format!("({slot}, {value}),"))
                    .collect();
                writeln!(out, "            {}", row.join(" ")).unwrap();
            }
            writeln!(out, "        ],").unwrap();
        }
        writeln!(out, "    }},").unwrap();
    }
    writeln!(out, "];").unwrap();
    writeln!(out).unwrap();

    let highest = scenes
        .iter()
        .flat_map(|(_, values)| values.iter().map(|(slot, _)| *slot))
        .max()
        .unwrap_or(0);
    writeln!(out, "/// Highest slot index any scene above touches.").unwrap();
    writeln!(out, "const HIGHEST_SLOT: usize = {highest};").unwrap();
    writeln!(out).unwrap();

    // A scene reaching past the end of the frame the daemon sends would be silently truncated
    // on the wire. This is the one place a rig's patch and its scenes are checked against each
    // other, and the compiler does it for free.
    writeln!(out, "const _: () = assert!(HIGHEST_SLOT < super::patch::DMX_SLOTS);").unwrap();

    out
}
