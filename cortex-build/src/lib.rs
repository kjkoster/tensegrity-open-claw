//! Ingests the patch and the pointing presets from a QLC+ workspace at build time, and
//! resolves the show calendar (see [`calendar`]).
//!
//! One rig, one workspace: the rig crate's build script calls [`ingest`] with its own
//! `.qxw`, and the fixtures it names are generated inside that rig's crate.
//!
//! A preset is a QLC+ scene holding where every head points. QLC+ stores scene values
//! **fixture-relative**:
//!
//!     <FixtureVal ID="3">0,255,1,0</FixtureVal>
//!
//! means "on the fixture whose QLC+ ID is 3, set its channel 0 to 255 and its channel 1
//! to 0". Those numbers are meaningless without the `<Fixture>` patch and the fixture
//! definitions, which is why the patch is authoritative: this script resolves them through
//! it into a pan and a tilt in degrees per head, through each head's own travel range.
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
//!   * `<Universe>` is the 0-based index into the workspace's universe list, which is not
//!     the number on the wire. That one is the `universe` parameter of the E1.31 output
//!     patched to it in the `<InputOutputMap>`.
//!   * The E1.31 output describes the console's own transmission: where QLC+ sends when it
//!     takes the rig over. The rig has one universe and relays a takeover onto it, so the
//!     universe and the port that output names are the rig's own as well — the ones the
//!     daemon listens and sends on, and the ones a wireless fixture listens on.

mod calendar;

pub use calendar::calendar;

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

/// The daemon sends exactly one universe. In the workspace that is the universe at 0-based
/// index 0; what it is numbered on the wire is the workspace's to say.
const WORKSPACE_UNIVERSE_INDEX: u32 = 0;

/// The QLC+ plugin whose output parameters name the universe and the port.
const SACN_PLUGIN: &str = "E1.31";

/// The universe a rig lives on, the port it is carried over and the address the console
/// unicasts to, as the workspace's E1.31 output states them.
pub struct Sacn {
    pub universe: u16,
    pub port: u16,
    pub address: Ipv4Addr,
}

/// Where one patched fixture sits and what its channels are, for a build that is not the rig
/// but has to agree with it: firmware that is itself the fixture.
pub struct Addressed {
    /// The fixture's name in the workspace.
    pub name: String,
    /// The Rust constant that name becomes — the same one the rig's generated patch uses.
    pub ident: String,
    /// The `.qxf` it resolved through.
    pub definition: PathBuf,
    /// 1-based DMX start address, as it is on the wire.
    pub address: u16,
    pub channels: u16,
    /// The patched mode's channels, in mode order.
    pub fields: Vec<AddressedField>,
}

/// One channel of a patched fixture's mode, named as the rig's generated struct names it.
#[derive(PartialEq)]
pub struct AddressedField {
    /// The rig's field name.
    pub name: String,
    /// 0-based offset from the fixture's start address.
    pub offset: u16,
    /// The bands the definition declares, each with the identifier its constant gets.
    pub bands: Vec<Band>,
}

#[derive(PartialEq)]
pub struct Band {
    pub ident: String,
    pub min: u8,
    pub max: u8,
}

/// A workspace's addressing: the universe it is on, and every fixture patched into it, in
/// address order.
pub struct Addressing {
    pub sacn: Sacn,
    pub fixtures: Vec<Addressed>,
}

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
    /// The `.qxf` the fixture resolved through. Empty until `resolve_definitions` fills it in.
    definition: PathBuf,
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
    /// The channel's QLC+ group: stated, or implied by its preset.
    group: String,
    /// The channel's `<Colour>`, for an intensity channel that names its colour without a
    /// preset.
    colour: Option<String>,
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
    group: String,
    colour: Option<String>,
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

/// Generates the `patch` and `presets` modules into `OUT_DIR` from one QLC+ workspace.
///
/// The fixture definitions are taken from the `fixtures/` directory beside the workspace,
/// which is one directory for every rig: a CLF Yara definition is the same object whichever
/// sculpture it hangs on. Each rig reads every definition and resolves only what its own
/// patch names.
///
/// `presets` is the set of preset names the rig's show is written against, by their names in
/// QLC+. The workspace's scenes must be exactly that set: the show refers to every one of them
/// by its generated constant, and a scene it does not know is a look nothing will ever play.
pub fn ingest(workspace: &Path, presets: &[&str]) {
    let xml = read_workspace(workspace);
    let doc = parse_workspace(&xml, workspace);
    let engine = engine_of(&doc, workspace);

    let sacn = parse_sacn(engine, workspace);
    let mut patch = parse_patch(engine);

    // The workspace names a definition; the definition says what the channels mean. Both
    // are needed before a fixture can be given typed channels, and before a preset's bytes
    // can become a head's degrees.
    resolve_definitions(&mut patch, &fixtures_dir(workspace));
    assign_type_names(&mut patch);
    let heads = aimable(&patch);
    let presets = parse_presets(engine, &patch, &heads, presets);

    // The generated file names the workspace it came from, so a stray copy found on its own
    // still says which rig it belongs to.
    let source = workspace
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_else(|| panic!("{} has no file name", workspace.display()));

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    fs::write(out_dir.join("rig.rs"), render_rig(&sacn, &patch, &heads, &presets, source)).unwrap();
}

/// Reads one QLC+ workspace for its addressing and its fixtures' channels, and generates
/// nothing.
///
/// For a build that has to agree with the rig about a fixture without being the rig:
/// firmware that is itself a patched fixture, and needs its own address and what its channels
/// mean. It goes through the same patch and definition checks as [`ingest`], so both builds
/// refuse the same workspaces.
pub fn addressing(workspace: &Path) -> Addressing {
    let xml = read_workspace(workspace);
    let doc = parse_workspace(&xml, workspace);
    let engine = engine_of(&doc, workspace);

    let sacn = parse_sacn(engine, workspace);
    let mut patch = parse_patch(engine);
    resolve_definitions(&mut patch, &fixtures_dir(workspace));
    let mut fixtures: Vec<Addressed> = patch
        .into_values()
        .map(|fixture| {
            let mut fields = Vec::new();
            for field in &fixture.fields {
                let (head, fine) = match field {
                    Field::Single { index, .. } => (*index, None),
                    Field::Pair { coarse, fine, .. } => (*coarse, Some(*fine)),
                };
                let bands = fixture.resolved[head]
                    .capabilities
                    .iter()
                    .map(|capability| Band {
                        ident: ident_of(&capability.name),
                        min: capability.min,
                        max: capability.max,
                    })
                    .collect();
                // Offsets fit: `parse_patch` holds every fixture inside the universe.
                let offset = |index: usize| (fixture.resolved[index].slot - fixture.address) as u16;
                fields.push(AddressedField {
                    name: field.name().to_string(),
                    offset: offset(head),
                    bands,
                });
                if let Some(fine) = fine {
                    fields.push(AddressedField {
                        name: format!("{}_fine", field.name()),
                        offset: offset(fine),
                        bands: Vec::new(),
                    });
                }
            }
            Addressed {
                ident: ident_of(&fixture.name),
                name: fixture.name,
                definition: fixture.definition,
                address: (fixture.address + 1) as u16,
                channels: fixture.channels as u16,
                fields,
            }
        })
        .collect();
    fixtures.sort_by_key(|fixture| fixture.address);

    Addressing { sacn, fixtures }
}

/// The definitions directory beside a workspace, watched for changes.
fn fixtures_dir(workspace: &Path) -> PathBuf {
    let dir = workspace
        .parent()
        .unwrap_or_else(|| panic!("{} has no parent directory", workspace.display()))
        .join("fixtures");
    println!("cargo:rerun-if-changed={}", dir.display());
    dir
}

fn read_workspace(workspace: &Path) -> String {
    println!("cargo:rerun-if-changed={}", workspace.display());

    fs::read_to_string(workspace).unwrap_or_else(|e| {
        panic!(
            "cannot read the QLC+ workspace at {}: {e}\n\
             The rig's patch and presets come from that file. deploy.sh rsyncs the whole \
             repository to the Pi; if you are building by hand, make sure it is there.",
            workspace.display()
        )
    })
}

fn parse_workspace<'a>(xml: &'a str, workspace: &Path) -> roxmltree::Document<'a> {
    // QLC+ writes a `<!DOCTYPE Workspace>` declaration, and roxmltree rejects any DTD by
    // default as a hardening measure. Allowing it is safe here: the DTD is an empty
    // internal subset that declares no entities, and roxmltree keeps its billion-laughs
    // protection regardless.
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..roxmltree::ParsingOptions::default()
    };
    roxmltree::Document::parse_with_options(xml, options)
        .unwrap_or_else(|e| panic!("{} is not valid XML: {e}", workspace.display()))
}

fn engine_of<'a, 'input>(
    doc: &'a roxmltree::Document<'input>,
    workspace: &Path,
) -> roxmltree::Node<'a, 'input> {
    doc.root_element()
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == "Engine")
        .unwrap_or_else(|| {
            panic!(
                "{} has no <Engine> element — is it a QLC+ workspace?",
                workspace.display()
            )
        })
}

/// Reads the universe and the port off the E1.31 output patched to the rig's universe.
fn parse_sacn(engine: roxmltree::Node, workspace: &Path) -> Sacn {
    fn child<'a, 'input>(
        node: roxmltree::Node<'a, 'input>,
        name: &str,
    ) -> Option<roxmltree::Node<'a, 'input>> {
        node.children()
            .find(|child| child.is_element() && child.tag_name().name() == name)
    }

    let universe = child(engine, "InputOutputMap")
        .and_then(|map| {
            map.children().find(|node| {
                node.is_element()
                    && node.tag_name().name() == "Universe"
                    && node.attribute("ID") == Some(&WORKSPACE_UNIVERSE_INDEX.to_string())
            })
        })
        .unwrap_or_else(|| {
            panic!(
                "{} has no <Universe ID=\"{WORKSPACE_UNIVERSE_INDEX}\"> in its \
                 <InputOutputMap>, so nothing says which universe the rig is on.",
                workspace.display()
            )
        });
    let parameters = child(universe, "Output")
        .filter(|output| output.attribute("Plugin") == Some(SACN_PLUGIN))
        .and_then(|output| child(output, "PluginParameters"))
        .unwrap_or_else(|| {
            panic!(
                "the first universe of {} has no {SACN_PLUGIN} output. The universe number \
                 and the port come from that output, so patch one in QLC+'s Inputs/Outputs \
                 tab and set its parameters there.",
                workspace.display()
            )
        });

    let parameter = |attribute: &str| -> &str {
        parameters
            .attribute(attribute)
            .unwrap_or_else(|| {
                panic!(
                    "the {SACN_PLUGIN} output in {} has no `{attribute}` parameter. Set it \
                     in the output's configuration in QLC+ and save.",
                    workspace.display()
                )
            })
            .trim()
    };
    let number = |attribute: &str| -> u16 {
        parameter(attribute).parse().unwrap_or_else(|e| {
            panic!(
                "the {SACN_PLUGIN} output in {} has a non-numeric `{attribute}`: {e}",
                workspace.display()
            )
        })
    };

    // The daemon's receiver binds the address the console unicasts to and joins no group, so
    // a console set to multicast would be sending where nothing listens.
    if parameter("multicast") != "0" {
        panic!(
            "the {SACN_PLUGIN} output in {} is set to multicast. The daemon only receives a \
             console by unicast; untick Multicast in the output's configuration in QLC+ and \
             give it the daemon's address.",
            workspace.display()
        );
    }

    let sacn = Sacn {
        universe: number("universe"),
        port: number("ucastPort"),
        address: parameter("ucastIP").parse().unwrap_or_else(|e| {
            panic!(
                "the {SACN_PLUGIN} output in {} has a `ucastIP` that is not an IPv4 \
                 address: {e}",
                workspace.display()
            )
        }),
    };
    // E1.31 §9.1.1 reserves universe 0 and everything from 64000 up.
    if !(1..=63999).contains(&sacn.universe) {
        panic!(
            "the {SACN_PLUGIN} output in {} is set to universe {}, outside the 1–63999 \
             sACN allows.",
            workspace.display(),
            sacn.universe
        );
    }
    if sacn.port == 0 {
        panic!(
            "the {SACN_PLUGIN} output in {} is set to port 0.",
            workspace.display()
        );
    }
    sacn
}

/// Renders the whole of a rig's generated half: the patch and the presets, as one file
/// included at the rig crate's root.
fn render_rig(
    sacn: &Sacn,
    patch: &BTreeMap<u32, Patched>,
    heads: &[(u32, &Patched)],
    presets: &[Preset],
    source: &str,
) -> String {
    let mut out = String::new();
    writeln!(out, "// Generated from {source}. Do not edit.").unwrap();
    writeln!(out).unwrap();
    write_module(&mut out, "patch", &render_patch(sacn, patch));
    write_module(&mut out, "presets", &render_presets(heads, presets));
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

            let element_text = |tag: &str| {
                channel
                    .children()
                    .find(|n| n.is_element() && n.tag_name().name() == tag)
                    .and_then(|n| n.text())
                    .map(|text| text.trim().to_string())
            };
            let preset = channel.attribute("Preset").map(str::to_string);
            // QLC+ writes a preset channel without a `<Group>`: the preset implies it.
            let group = element_text("Group")
                .or_else(|| preset.as_deref().and_then(group_of_preset).map(str::to_string))
                .unwrap_or_else(|| {
                    panic!(
                        "{}: channel {name:?} has neither a <Group> nor a preset that implies one",
                        path.display()
                    )
                });

            channels.insert(
                name,
                ChannelDef {
                    preset,
                    group,
                    colour: element_text("Colour"),
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
                    group: channel.group.clone(),
                    colour: channel.colour.clone(),
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
        fixture.definition = definition.path.clone();
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

/// The group a channel preset implies, as QLC+ assigns it: the preset's attribute is its
/// leading word, and only position splits into two groups.
fn group_of_preset(preset: &str) -> Option<&'static str> {
    let group = match preset {
        "NoFunction" => "Nothing",
        p if p.starts_with("PositionPan") || p.starts_with("PositionXAxis") => "Pan",
        p if p.starts_with("PositionTilt") || p.starts_with("PositionYAxis") => "Tilt",
        p if p.starts_with("Intensity") => "Intensity",
        p if p.starts_with("Color") => "Colour",
        p if p.starts_with("Speed") => "Speed",
        p if p.starts_with("Gobo") => "Gobo",
        p if p.starts_with("Shutter") => "Shutter",
        p if p.starts_with("Beam") => "Beam",
        p if p.starts_with("Prism") => "Prism",
        _ => return None,
    };
    Some(group)
}

/// Intensity-group presets for which zero is not dark: hue and saturation pick a colour, and
/// subtractive cyan, magenta and yellow at zero let the whole beam through.
const NOT_DARK_AT_ZERO: [&str; 5] = [
    "IntensityHue",
    "IntensitySaturation",
    "IntensityCyan",
    "IntensityMagenta",
    "IntensityYellow",
];

/// The value the e-stop holds one channel at, and why; `None` where it holds the channel at
/// whatever it last carried. Panics, naming the band to add, where the rule calls for a value
/// the definition does not state: a guessed stop value is exactly the one that turns out to
/// be a visible event.
///
/// The order matters. Intensity comes first, so light is never mistaken for anything else.
/// Rotation comes before the groups, because a spinning wheel or motor can sit in any of
/// them and stopping it beats whatever its group would do.
fn stop_override(fixture: &Patched, channel: &ResolvedChannel) -> Option<(u8, String)> {
    let band = |preset: &str| {
        channel
            .capabilities
            .iter()
            .find(|capability| capability.preset.as_deref() == Some(preset))
            .map(|capability| (capability.min, capability.name.clone()))
    };
    let missing = |wanted: &str| -> ! {
        panic!(
            "{:?}: channel {:?} of {} is in the {} group, which the e-stop sends to its {wanted} \
             band, and it declares none. Mark that band with its preset in the .qxf.",
            fixture.name,
            channel.name,
            fixture.definition.display(),
            channel.group,
        )
    };

    let coarse = channel.preset.as_deref().map(|p| p.strip_suffix("Fine").unwrap_or(p));
    if channel.group == "Intensity" {
        let subtractive = matches!(channel.colour.as_deref(), Some("Cyan" | "Magenta" | "Yellow"));
        let not_dark = coarse.is_some_and(|p| NOT_DARK_AT_ZERO.contains(&p)) || (coarse.is_none() && subtractive);
        return (!not_dark).then(|| (0, "blackout".to_string()));
    }
    if let Some(stop) = band("RotationStop") {
        return Some(stop);
    }
    match channel.group.as_str() {
        "Shutter" if coarse.is_some_and(|p| p.starts_with("ShutterIris")) => None,
        // Closed where the shutter has it, otherwise open: the light is already dark, and
        // what matters is that the strobe stops.
        "Shutter" => Some(
            band("ShutterClose")
                .or_else(|| band("ShutterOpen"))
                .unwrap_or_else(|| missing("ShutterClose or ShutterOpen")),
        ),
        "Effect" | "Maintenance" => Some(band("NoFunction").unwrap_or_else(|| missing("NoFunction"))),
        _ => None,
    }
}

/// Renders the e-stop's mask: one override per channel it overrides, in address order. Every
/// channel not listed is held.
///
/// Channels are written as paths into the instance constants, like the patch table, so the
/// mask carries no slot numbers of its own to disagree with them.
fn render_stop(out: &mut String, fixtures: &[&Patched]) {
    writeln!(out, "use cortex::estop::StopOverride;").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "/// What the e-stop overrides in the frame it holds. Every channel not listed is held.").unwrap();
    writeln!(out, "pub static STOP: &[StopOverride] = &[").unwrap();
    for fixture in fixtures {
        writeln!(out, "    // {}", fixture.name).unwrap();
        let instance = ident_of(&fixture.name);
        for field in &fixture.fields {
            let channel = &fixture.resolved[field.head()];
            let Some((value, why)) = stop_override(fixture, channel) else {
                continue;
            };
            let path = format!("{instance}.{}", field.name());
            let mut row = |path: &str, value: u8, comment: &str| {
                writeln!(out, "    StopOverride {{ channel: {path}, value: {value} }}, // {comment}").unwrap();
            };
            match field {
                Field::Single { .. } => row(&path, value, &format!("{} — {why}", channel.name)),
                // The band states the coarse value only, so the fine half goes to the band's
                // own start; blackout is zero in both halves.
                Field::Pair { .. } => {
                    row(&format!("{path}.coarse"), value, &format!("{} — {why}", channel.name));
                    row(&format!("{path}.fine"), 0, &format!("{} fine", channel.name));
                }
            }
        }
    }
    writeln!(out, "];").unwrap();
    writeln!(out).unwrap();
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
                 The daemon sends exactly one universe (index {WORKSPACE_UNIVERSE_INDEX}); \
                 teaching the daemon about universes is a deliberate change, not something to \
                 fold in silently."
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
                definition: PathBuf::new(),
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
             daemon's presets are resolved through it — so an empty one is never right."
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

/// The generated `presets` module's one value that is not a preset, so no preset may take
/// its name.
const SUMMARIES: &str = "SUMMARIES";

/// One preset as the show gets it: where every aimable head points.
struct Preset {
    /// The scene's name in QLC+.
    name: String,
    /// One pose per aimable head, in address order.
    poses: Vec<HeadPose>,
}

/// One head's pose in a preset, in the unreferenced degrees `cortex`'s `Position` speaks.
struct HeadPose {
    /// The head's field in the generated `Preset` struct.
    field: String,
    pan_deg: f64,
    tilt_deg: f64,
}

/// Resolves the workspace's scenes into the presets the rig prescribes, in the rig's order.
///
/// The workspace must hold exactly the prescribed set. A missing preset is a look the show
/// asks for and would not get; an extra one is a look nothing plays, and as likely as not a
/// typo of one that is missing. Both are reported together, so one build says everything
/// that is wrong with the set.
///
/// A preset carries pan and tilt, coarse and fine, for every head and nothing else. A missing
/// channel is the dangerous case: it would resolve to 0°, which is the end of the head's
/// travel rather than "no opinion". An extra one is the quiet case: a dump still carrying a
/// dimmer or a motor speed holds a value the show never sends, and whoever recorded it would
/// expect it to matter.
fn parse_presets(
    engine: roxmltree::Node,
    patch: &BTreeMap<u32, Patched>,
    heads: &[(u32, &Patched)],
    prescribed: &[&str],
) -> Vec<Preset> {
    for (index, name) in prescribed.iter().enumerate() {
        let ident = ident_of(name);
        if !ident.starts_with(|c: char| c.is_ascii_alphabetic()) {
            panic!(
                "prescribed preset {name:?} does not yield a usable Rust name (got {ident:?}). \
                 Rename it to something starting with a letter."
            );
        }
        if let Some(other) = prescribed[..index].iter().find(|other| ident_of(other) == ident) {
            panic!("prescribed presets {other:?} and {name:?} both map to the constant {ident}. Rename one.");
        }
        if ident == SUMMARIES {
            panic!(
                "prescribed preset {name:?} maps to the constant {ident}, which the presets module \
                 already uses for its log lines. Rename it."
            );
        }
    }

    let mut scenes: BTreeMap<&str, roxmltree::Node> = BTreeMap::new();
    for element in engine.children().filter(|node| {
        node.is_element()
            && node.tag_name().name() == "Function"
            && node.attribute("Type") == Some("Scene")
    }) {
        let name = element.attribute("Name").unwrap_or_default().trim();
        if scenes.insert(name, element).is_some() {
            panic!(
                "duplicate scene name {name:?}. QLC+ does not enforce unique Function \
                 names, but the daemon needs them unique — rename one in the Function \
                 Manager. (Note a Simple Desk dump always creates a new scene; it never \
                 updates an existing one.)"
            );
        }
    }

    let missing: Vec<&str> = prescribed
        .iter()
        .copied()
        .filter(|name| !scenes.contains_key(name))
        .collect();
    let extra: Vec<&str> = scenes
        .keys()
        .copied()
        .filter(|name| !prescribed.contains(name))
        .collect();
    if !missing.is_empty() || !extra.is_empty() {
        let mut problems = String::new();
        for name in &extra {
            match nearest(name, &missing) {
                Some(meant) => writeln!(
                    problems,
                    "  scene {name:?} is not a preset the show knows — did you mean {meant:?}? \
                     Rename it in QLC+'s Function Manager."
                ),
                None => writeln!(
                    problems,
                    "  scene {name:?} is not a preset the show knows. Delete it in QLC+'s \
                     Function Manager, or add it to the rig's presets."
                ),
            }
            .unwrap();
        }
        for name in &missing {
            writeln!(
                problems,
                "  preset {name:?} has no scene. Record it in QLC+ with only the heads' Pan, \
                 Pan Fine, Tilt and Tilt Fine ticked."
            )
            .unwrap();
        }
        panic!("the workspace's scenes are not the presets the show is written against:\n{problems}");
    }

    prescribed
        .iter()
        .map(|name| {
            let values = scene_values(name, scenes[name], patch);

            for &(id, offset) in values.keys() {
                let fixture = &patch[&id];
                let positional = aims(fixture)
                    && ["pan", "tilt"].iter().any(|axis| {
                        let (coarse, fine) = pair_of(fixture, axis);
                        offset == coarse || offset == fine
                    });
                if !positional {
                    panic!(
                        "preset {name:?} sets {:?} on {:?}. A preset holds only the heads' Pan, \
                         Pan Fine, Tilt and Tilt Fine — untick that channel in QLC+'s Scene \
                         Editor.",
                        fixture.resolved[offset].name, fixture.name,
                    );
                }
            }

            let poses = heads
                .iter()
                .map(|(id, head)| {
                    let focus = head.focus.as_ref().expect("aims() checked the focus");
                    // The same conversion as `pose_of` in `cortex`, so a preset recorded from a
                    // console and replayed through `aim` lands on the bytes it was saved as.
                    let degrees = |axis: &str, range_deg: f64| {
                        let (coarse, fine) = pair_of(head, axis);
                        let byte = |index: usize| {
                            *values.get(&(*id, index)).unwrap_or_else(|| {
                                panic!(
                                    "preset {name:?} has no {:?} for {:?}. Every preset holds Pan, \
                                     Pan Fine, Tilt and Tilt Fine for every head — tick it in \
                                     QLC+'s Scene Editor.",
                                    head.resolved[index].name, head.name,
                                )
                            })
                        };
                        let value = (u16::from(byte(coarse)) << 8) | u16::from(byte(fine));
                        f64::from(value) / 65535.0 * range_deg
                    };
                    HeadPose {
                        field: field_of(&head.name),
                        pan_deg: degrees("pan", focus.pan_max_deg),
                        tilt_deg: degrees("tilt", focus.tilt_max_deg),
                    }
                })
                .collect();

            Preset {
                name: name.to_string(),
                poses,
            }
        })
        .collect()
}

/// Every fixture that can be aimed, with its QLC+ ID, in address order: the heads a preset
/// holds a pose for.
fn aimable(patch: &BTreeMap<u32, Patched>) -> Vec<(u32, &Patched)> {
    let mut heads: Vec<(u32, &Patched)> = patch
        .iter()
        .filter(|(_, fixture)| aims(fixture))
        .map(|(id, fixture)| (*id, fixture))
        .collect();
    heads.sort_by_key(|(_, head)| head.address);
    heads
}

/// A scene's values, keyed by QLC+ fixture ID and channel offset within that fixture's mode.
fn scene_values(name: &str, element: roxmltree::Node, patch: &BTreeMap<u32, Patched>) -> BTreeMap<(u32, usize), u8> {
    // A QLC+ channel group is a console-side abstraction — one fader driving several
    // channels — with no fixture-relative address to resolve through. Refuse it rather
    // than drop it silently and lose what the operator set.
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

    // A map de-duplicates repeated channels within a scene, which is serialisation noise:
    // the last one wins.
    let mut values = BTreeMap::new();
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
            values.insert((id, offset as usize), value);
        }
    }
    values
}

/// The mode indices of a fixture's 16-bit field `name`, coarse then fine.
fn pair_of(fixture: &Patched, name: &str) -> (usize, usize) {
    fixture
        .fields
        .iter()
        .find_map(|field| match field {
            Field::Pair { coarse, fine, name: field_name } if field_name == name => Some((*coarse, *fine)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{:?} has no 16-bit {name} field", fixture.name))
}

/// The candidate a scene name is most likely a typo of, if any is close enough to suggest.
fn nearest<'a>(name: &str, candidates: &[&'a str]) -> Option<&'a str> {
    let name = name.to_lowercase();
    candidates
        .iter()
        .map(|candidate| (edit_distance(&name, &candidate.to_lowercase()), *candidate))
        .min()
        .filter(|(distance, candidate)| *distance <= 2.max(candidate.chars().count() / 3))
        .map(|(_, candidate)| candidate)
}

/// Levenshtein distance, by characters.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1).min(row[j] + 1).min(diagonal + usize::from(ca != *cb));
            diagonal = above;
        }
    }
    row[b.len()]
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
/// fixture *type* with a named field per channel, and the e-stop's mask over them.
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
/// That last one is why channels are typed rather than numbered. `WASH_RED_LEG.red` names the red
/// emitter because the definition says that channel of that mode is `IntensityRed`; there is
/// no offset in the source to get wrong, and a fixture with no red simply has no `red`.
fn render_patch(sacn: &Sacn, patch: &BTreeMap<u32, Patched>) -> String {
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
    let mut imports = vec!["Channel", "PatchEntry", "Sacn"];
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

    writeln!(
        out,
        "/// The universe the rig is on, the port it is carried over and where a console \
         reaches it."
    )
    .unwrap();
    let [a, b, c, d] = sacn.address.octets();
    writeln!(out, "pub const SACN: Sacn = Sacn {{").unwrap();
    writeln!(out, "    universe: {},", sacn.universe).unwrap();
    writeln!(out, "    port: {},", sacn.port).unwrap();
    writeln!(out, "    address: core::net::Ipv4Addr::new({a}, {b}, {c}, {d}),").unwrap();
    writeln!(out, "}};").unwrap();
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

    render_stop(&mut out, &fixtures);

    out
}

/// Renders the presets: a `Preset` struct with a pose per aimable head, one constant per
/// preset, and a line per preset for the startup log.
///
/// The struct is generated rather than shared from `cortex` because its fields are this rig's
/// heads, so a preset is read as `LOOK_AT_BALL.head_red_leg` and a head that leaves the patch
/// takes its field with it.
fn render_presets(heads: &[(u32, &Patched)], presets: &[Preset]) -> String {
    let mut out = String::new();

    if !heads.is_empty() {
        writeln!(out, "use cortex::moving_head::Pose;").unwrap();
        writeln!(out).unwrap();
    }
    writeln!(out, "/// Where every head points in one preset.").unwrap();
    writeln!(out, "pub struct Preset {{").unwrap();
    for (_, head) in heads {
        writeln!(out, "    /// {}.", head.name).unwrap();
        writeln!(out, "    pub {}: Pose,", field_of(&head.name)).unwrap();
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    for preset in presets {
        writeln!(out, "/// {:?} in the workspace.", preset.name).unwrap();
        writeln!(out, "pub const {}: Preset = Preset {{", ident_of(&preset.name)).unwrap();
        for pose in &preset.poses {
            writeln!(
                out,
                "    {}: Pose::new({:?}, {:?}),",
                pose.field, pose.pan_deg, pose.tilt_deg
            )
            .unwrap();
        }
        writeln!(out, "}};").unwrap();
        writeln!(out).unwrap();
    }

    // The poses and not just the names, which are fixed: a re-aimed preset the build never
    // saw would otherwise log exactly like one it did.
    writeln!(
        out,
        "/// Every preset as the startup log prints it — its name in QLC+ and each head's pose — \
         in the order the rig prescribes them."
    )
    .unwrap();
    writeln!(out, "pub static {SUMMARIES}: [&str; {}] = [", presets.len()).unwrap();
    for preset in presets {
        let poses: Vec<String> = preset
            .poses
            .iter()
            .zip(heads)
            .map(|(pose, (_, head))| format!("{} {:.1}°/{:.1}°", head.name, pose.pan_deg, pose.tilt_deg))
            .collect();
        writeln!(out, "    {:?},", format!("{} — {}", preset.name, poses.join(", "))).unwrap();
    }
    writeln!(out, "];").unwrap();

    out
}
