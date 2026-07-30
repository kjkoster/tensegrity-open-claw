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
use std::path::PathBuf;

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
    address: u32,
    channels: u32,
}

impl Patched {
    /// Absolute 0-based slot for a fixture-relative channel offset. The inverse of the
    /// daemon's `fixture::Fixture::slot()`, minus its `- 1`: QLC+ already stores the
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

    let patch = parse_patch(engine);
    let scenes = parse_scenes(engine, &patch);

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    fs::write(out_dir.join("patch.rs"), render_patch(&patch)).unwrap();
    fs::write(out_dir.join("scenes.rs"), render_scenes(&scenes)).unwrap();
}

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
        if patch
            .insert(id, Patched { name, ident, address, channels })
            .is_some()
        {
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

    for fixture in &fixtures {
        writeln!(
            out,
            "/// {} — {} channels at DMX address {}, filling slots {}–{}.",
            fixture.name,
            fixture.channels,
            fixture.address + 1,
            fixture.address + 1,
            fixture.address + fixture.channels,
        )
        .unwrap();
        writeln!(
            out,
            "pub const {}: Fixture = Fixture {{ start_address: {}, channels: {} }};",
            fixture.ident,
            fixture.address + 1,
            fixture.channels,
        )
        .unwrap();
        writeln!(out).unwrap();
    }

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
