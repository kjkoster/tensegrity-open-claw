//! Ingests the scenes from the QLC+ workspace at build time.
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

use std::collections::BTreeMap;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

/// The daemon sends exactly one universe (`config::UNIVERSE`). In the workspace that is
/// the universe at 0-based index 0.
const WORKSPACE_UNIVERSE_INDEX: u32 = 0;

/// Slots in a DMX-512 universe. Spelled out rather than borrowed from the crate that
/// drives the wire: a build script may only use its build-dependencies, and pulling in a
/// serial-port crate to read one number from the protocol would be a poor trade.
const UNIVERSE_SLOTS: u32 = 512;

/// One row of the patch: what a QLC+ fixture ID means in absolute DMX slots.
struct Patched {
    name: String,
    /// `name` as a Rust constant identifier — "Yara 1" becomes `YARA_1`.
    ident: String,
    /// `ident` as a Rust type name — `YARA_1` becomes `Yara1`.
    type_name: String,
    manufacturer: String,
    model: String,
    mode: String,
    address: u32,
    channels: u32,
    /// The mode's channels, resolved against the fixture definition. Empty until
    /// `resolve_definitions` fills it in.
    resolved: Vec<ResolvedChannel>,
}

/// One channel of a patched fixture's mode, resolved through its `.qxf`.
struct ResolvedChannel {
    /// The channel's name in the definition, for doc comments.
    name: String,
    /// The Rust field name it becomes: its preset's role, or the channel name.
    field: String,
    /// The `qlc_plus::Preset` variant.
    preset: &'static str,
    /// Absolute 0-based slot.
    slot: u32,
}

/// A parsed `.qxf`: what each named channel is, and which channels each mode uses.
struct Definition {
    path: PathBuf,
    /// Channel name → its `Preset` attribute, if it has one.
    channels: BTreeMap<String, Option<String>>,
    /// Mode name → its channel names, in `Number` order.
    modes: BTreeMap<String, Vec<String>>,
}

impl Patched {
    /// Absolute 0-based slot for a fixture-relative channel offset. The inverse of the
    /// absolute slot a `qlc_plus::Channel` holds: QLC+ already stores the
    /// start address 0-based.
    fn slot(&self, offset: u32) -> u32 {
        self.address + offset
    }
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace = manifest_dir.join("..").join("open-claw.qxw");
    println!("cargo:rerun-if-changed={}", workspace.display());

    let xml = fs::read_to_string(&workspace).unwrap_or_else(|e| {
        panic!(
            "cannot read the QLC+ workspace at {}: {e}\n\
             The daemon's scenes come from that file. deploy.sh rsyncs it to the Pi \
             beside brain/; if you are building by hand, make sure it is there.",
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
    let fixtures_dir = manifest_dir.join("..").join("fixtures");
    println!("cargo:rerun-if-changed={}", fixtures_dir.display());
    resolve_definitions(&mut patch, &fixtures_dir);

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    fs::write(out_dir.join("patch.rs"), render_patch(&patch)).unwrap();
    fs::write(out_dir.join("scenes.rs"), render_scenes(&scenes)).unwrap();
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
            channels.insert(name, channel.attribute("Preset").map(str::to_string));
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

        if let Some(previous) = definitions.insert(
            (manufacturer.clone(), model.clone()),
            Definition { path: path.clone(), channels, modes },
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

        // Prefer the preset's role as the field name — a channel named "R" carrying
        // IntensityRed still becomes `red`. Channels QLC+ gives no preset (wheels, gobos,
        // maintenance) fall back to their own name.
        let mut resolved: Vec<ResolvedChannel> = mode
            .iter()
            .enumerate()
            .map(|(offset, channel_name)| {
                let preset = definition.channels.get(channel_name).unwrap_or_else(|| {
                    panic!(
                        "{}: mode {:?} lists a channel {channel_name:?} the definition does \
                         not declare",
                        definition.path.display(),
                        fixture.mode,
                    )
                });
                let (variant, role) = preset_role(preset.as_deref());
                ResolvedChannel {
                    name: channel_name.clone(),
                    field: role.map(str::to_string).unwrap_or_else(|| field_of(channel_name)),
                    preset: variant,
                    slot: fixture.slot(offset as u32),
                }
            })
            .collect();

        // Two channels can share a preset — pixel-mapped modes give every cell an
        // IntensityRed. Where that happens the role is not a unique name, so fall back to
        // the channel names, which QLC+ does keep distinct within a mode.
        let clashes: Vec<String> = resolved
            .iter()
            .filter(|c| resolved.iter().filter(|o| o.field == c.field).count() > 1)
            .map(|c| c.field.clone())
            .collect();
        for channel in resolved.iter_mut() {
            if clashes.contains(&channel.field) {
                channel.field = field_of(&channel.name);
            }
        }
        for (index, channel) in resolved.iter().enumerate() {
            if let Some(other) = resolved[..index].iter().find(|o| o.field == channel.field) {
                panic!(
                    "{:?}: channels {:?} and {:?} both map to the field `{}`. Rename one in {}.",
                    fixture.name,
                    other.name,
                    channel.name,
                    channel.field,
                    definition.path.display(),
                );
            }
        }

        fixture.resolved = resolved;
    }
}

/// Maps a QLC+ preset to its `qlc_plus::Preset` variant and, for the Intensity family, the
/// field name that role gets. `None` role means the channel is named after itself.
fn preset_role(preset: Option<&str>) -> (&'static str, Option<&'static str>) {
    match preset {
        Some("IntensityMasterDimmer") => ("IntensityMasterDimmer", Some("dimmer")),
        Some("IntensityDimmer") => ("IntensityDimmer", Some("intensity")),
        Some("IntensityRed") => ("IntensityRed", Some("red")),
        Some("IntensityGreen") => ("IntensityGreen", Some("green")),
        Some("IntensityBlue") => ("IntensityBlue", Some("blue")),
        Some("IntensityWhite") => ("IntensityWhite", Some("white")),
        Some("IntensityAmber") => ("IntensityAmber", Some("amber")),
        Some("IntensityUV") => ("IntensityUV", Some("uv")),
        Some("IntensityCyan") => ("IntensityCyan", Some("cyan")),
        Some("IntensityMagenta") => ("IntensityMagenta", Some("magenta")),
        Some("IntensityYellow") => ("IntensityYellow", Some("yellow")),
        Some("IntensityHue") => ("IntensityHue", Some("hue")),
        Some("IntensitySaturation") => ("IntensitySaturation", Some("saturation")),
        Some("IntensityValue") => ("IntensityValue", Some("value")),
        Some("IntensityLightness") => ("IntensityLightness", Some("lightness")),
        _ => ("Custom", None),
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

/// Turns that identifier into a type name: `YARA_1` → `Yara1`.
fn type_name_of(ident: &str) -> String {
    ident
        .split('_')
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
        if let Some(clash) = patch.values().find(|p| p.ident == ident) {
            panic!(
                "fixtures {:?} and {name:?} both map to the constant {ident} — the daemon \
                 could not tell them apart. Rename one in QLC+.",
                clash.name
            );
        }
        let inserted = patch.insert(
            id,
            Patched {
                type_name: type_name_of(&ident),
                name,
                ident,
                manufacturer: field("Manufacturer"),
                model: field("Model"),
                mode: field("Mode"),
                address,
                channels,
                resolved: Vec::new(),
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

/// Renders the fixture table included by `src/patch.rs`: one named constant per fixture in
/// the QLC+ patch, plus the width of the frame they span.
///
/// Naming each fixture is what makes the workspace the source of truth, because it hands
/// both directions of drift to the compiler. A fixture added in QLC+ that no code drives is
/// an unused constant — a dead-code warning. A fixture deleted in QLC+ that code still
/// drives is a missing constant — the use site stops compiling. Neither needs a checker.
fn render_patch(patch: &BTreeMap<u32, Patched>) -> String {
    let mut out = String::new();

    writeln!(out, "// Generated by build.rs from open-claw.qxw. Do not edit.").unwrap();
    writeln!(out).unwrap();

    let mut fixtures: Vec<&Patched> = patch.values().collect();
    fixtures.sort_by_key(|fixture| fixture.address);

    // Import exactly the capability traits this patch implements. Naming them all in the
    // hand-written half would warn about the ones no patched fixture happens to have, and
    // that warning would be noise rather than the signal an unused import usually is.
    let implements = |field: &str| {
        fixtures
            .iter()
            .any(|fixture| fixture.resolved.iter().any(|c| c.field == field))
    };
    let mut imports = vec!["Channel", "PatchEntry", "Preset"];
    if implements("red") && implements("green") && implements("blue") {
        imports.push("Rgb");
    }
    if implements("white") {
        imports.push("White");
    }
    if implements("dimmer") {
        imports.push("Dimmer");
    }
    imports.sort_unstable();
    writeln!(out, "use crate::qlc_plus::{{{}}};", imports.join(", ")).unwrap();
    writeln!(out).unwrap();

    for fixture in &fixtures {
        let profile = format!("{} {}, {}", fixture.manufacturer, fixture.model, fixture.mode);

        // The struct: one field per channel of the patched mode, named for what QLC+ says
        // that channel is. This is where a channel's role becomes part of the type.
        writeln!(
            out,
            "/// {} — {}, DMX address {} (slots {}–{}).",
            fixture.name,
            profile,
            fixture.address + 1,
            fixture.address + 1,
            fixture.address + fixture.channels,
        )
        .unwrap();
        writeln!(out, "pub struct {} {{", fixture.type_name).unwrap();
        for channel in &fixture.resolved {
            writeln!(out, "    /// {} (slot {}).", channel.name, channel.slot + 1).unwrap();
            writeln!(out, "    pub {}: Channel,", channel.field).unwrap();
        }
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();

        // Positional access is a convenience for fixtures whose channels have no colour to
        // name — a moving head's position channels, say. Which fixtures want it is a
        // property of the hand-written
        // code, not of the patch, so generating it everywhere and allowing it to go unused
        // is honest; the dead-code warning that matters is the one on the fixture itself.
        writeln!(out, "#[allow(dead_code)]").unwrap();
        writeln!(out, "impl {} {{", fixture.type_name).unwrap();
        writeln!(out, "    /// Channels the patched mode occupies.").unwrap();
        writeln!(out, "    pub const CHANNELS: usize = {};", fixture.channels).unwrap();
        writeln!(out).unwrap();
        writeln!(out, "    /// Every channel, in mode order.").unwrap();
        writeln!(
            out,
            "    pub fn all(&self) -> [Channel; Self::CHANNELS] {{"
        )
        .unwrap();
        writeln!(
            out,
            "        [{}]",
            fixture
                .resolved
                .iter()
                .map(|c| format!("self.{}", c.field))
                .collect::<Vec<_>>()
                .join(", ")
        )
        .unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();

        writeln!(
            out,
            "pub const {}: {} = {} {{",
            fixture.ident, fixture.type_name, fixture.type_name
        )
        .unwrap();
        for channel in &fixture.resolved {
            writeln!(out, "    {}: Channel::at({}),", channel.field, channel.slot).unwrap();
        }
        writeln!(out, "}};").unwrap();
        writeln!(out).unwrap();

        // Capability traits, implemented only where the mode actually carries the channels.
        // This is the type-safety the whole exercise is for: a fixture patched into a mode
        // without colour cannot be handed to code that mixes colour.
        let has = |field: &str| fixture.resolved.iter().any(|c| c.field == field);
        if has("red") && has("green") && has("blue") {
            writeln!(out, "impl Rgb for {} {{", fixture.type_name).unwrap();
            for colour in ["red", "green", "blue"] {
                writeln!(
                    out,
                    "    fn {colour}(&self) -> Channel {{ self.{colour} }}"
                )
                .unwrap();
            }
            writeln!(out, "}}").unwrap();
        }
        if has("white") {
            writeln!(out, "impl White for {} {{", fixture.type_name).unwrap();
            writeln!(out, "    fn white(&self) -> Channel {{ self.white }}").unwrap();
            writeln!(out, "}}").unwrap();
        }
        if has("dimmer") {
            writeln!(out, "impl Dimmer for {} {{", fixture.type_name).unwrap();
            writeln!(out, "    fn dimmer(&self) -> Channel {{ self.dimmer }}").unwrap();
            writeln!(out, "}}").unwrap();
        }
        writeln!(out).unwrap();
    }

    // The same patch as plain data, so the daemon can log what it was built against
    // without a hand-maintained list of fixture names going stale beside it.
    writeln!(out, "/// Every patched fixture, in address order.").unwrap();
    writeln!(
        out,
        "pub const PATCH: [PatchEntry; {}] = [",
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
        writeln!(
            out,
            "        channels: &[{}],",
            fixture
                .resolved
                .iter()
                .map(|c| format!("(Preset::{}, Channel::at({}))", c.preset, c.slot))
                .collect::<Vec<_>>()
                .join(", ")
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
    writeln!(out, "/// Slots the sACN frame spans: 1 through the last patched slot.").unwrap();
    writeln!(out, "pub const DMX_SLOTS: usize = {top};").unwrap();

    out
}

/// Renders the table included by `src/scenes.rs`. Only the data lives here; the `Scene`
/// type and everything that reads it stay hand-written and reviewable.
fn render_scenes(scenes: &[(String, Vec<(u32, u8)>)]) -> String {
    let mut out = String::new();

    writeln!(out, "// Generated by build.rs from open-claw.qxw. Do not edit.").unwrap();
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

    out
}
