#!/usr/bin/env python3
"""Writes `generated.h`, the facts Wrist's firmware takes from the rig's source files.

    python3 -I generate.py <clockwork.dot> <clockwork.qxw> <wrist.qxf> <wrist.oscquery.json> <generated.h>

Each fact is read from the one file that records it, and only what the firmware uses is read:

  clockwork.dot          the WiFi: SSID, passphrase, channel; Wrist's address, the gateway and
                         the netmask; the port Wrist listens on for sACN; where its OSC goes.
  clockwork.qxw          the sACN universe, Wrist's DMX start address and the mode it is
                         patched in.
  wrist.qxf              that mode's channels, in order, and each channel's bands.
  wrist.oscquery.json    every OSC address Wrist sends, and how: the encoders.

Fails, naming the file and what is wrong, rather than writing a header from a guess. The header
is git-ignored: it is rebuilt from these files on every build, so it is never edited by hand.
"""

import ipaddress
import pathlib
import re
import sys
import xml.etree.ElementTree as ET

# The dot's reader, in clockwork/, shared with pi_network.py there, and the OSCQuery file's,
# shared with cortex-build. Put on the path by hand: run with -I, Python adds no directory of
# its own.
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent / "clockwork"))
from dot import Dot, addresses, fail, listener, networks  # noqa: E402
import oscquery  # noqa: E402

DEVICE = "wrist"
SERVICE = "wrist_firmware"


# ── clockwork.dot ────────────────────────────────────────────────────────────────────────────


def read_dot(path):
    dot = Dot(path)
    nets = networks(dot)
    held = addresses(dot, DEVICE, nets)
    if len(held) != 1:
        fail(f"{path}: {DEVICE} should have one address, and has {len(held)}")
    (address, network), = held.values()
    if address is None:
        fail(f"{path}: {DEVICE}'s address is a lease; the firmware needs a fixed one")
    net = nets[network]
    for key in ("ssid", "psk", "channel", "gateway"):
        if key not in net:
            fail(f"{path}: the {network} network has no {key}")

    _, sacn_port = listener(dot, nets, SERVICE, "sacn", network)
    recipients = []
    for tail, tail_port, head, head_port, attrs in dot.edges:
        if tail != SERVICE:
            continue
        what = attrs.get("xlabel", attrs.get("label", ("", False)))[0]
        if not what.startswith("OSC"):
            fail(f"{path}: {SERVICE} sends {what!r}, which the firmware does not know")
        if tail_port != "title" or head_port is None:
            fail(f"{path}: an edge from {SERVICE} must run from its title to a port")
        to, port = listener(dot, nets, head, head_port, network)
        device = dot.device_of(head)
        service = head.removeprefix(f"{device}_")
        recipients.append((f"{device} {service}".replace("_", "-"), to, port))
    if not recipients:
        fail(f"{path}: {SERVICE} sends OSC to nobody")
    return {
        "ssid": net["ssid"],
        "psk": net["psk"],
        "channel": int(net["channel"]),
        "address": address,
        "gateway": ipaddress.IPv4Address(net["gateway"]),
        "netmask": net["cidr"].netmask,
        "sacn_port": sacn_port,
        "recipients": recipients,
    }


# ── clockwork.qxw ────────────────────────────────────────────────────────────────────────────

QLC = "{http://www.qlcplus.org/Workspace}"

# The rig has one universe, as cortex-build reads it: the workspace's universe at 0-based index
# 0. Its number on the wire is the `universe` parameter of the E1.31 output patched to it.
UNIVERSE_INDEX = "0"


def read_workspace(path):
    root = ET.parse(path).getroot()
    params = root.find(
        f"{QLC}Engine/{QLC}InputOutputMap/{QLC}Universe[@ID='{UNIVERSE_INDEX}']"
        f"/{QLC}Output[@Plugin='E1.31']/{QLC}PluginParameters"
    )
    if params is None:
        fail(f"{path}: the universe at index {UNIVERSE_INDEX} has no E1.31 output")
    universe = params.get("universe", "").strip()
    if not universe.isdigit() or not 1 <= int(universe) <= 63999:
        fail(f"{path}: the E1.31 output's universe is {universe!r}, not one of the 1–63999 sACN allows")
    for fixture in root.iter(f"{QLC}Fixture"):
        name = fixture.findtext(f"{QLC}Name", "")
        # A networked fixture may carry its address and MAC after its name: `Wrist [ip mac]`.
        if name.split(" [")[0] == "Wrist":
            if fixture.findtext(f"{QLC}Universe", "").strip() != UNIVERSE_INDEX:
                fail(f"{path}: Wrist is patched to universe index {fixture.findtext(f'{QLC}Universe')!r}, not {UNIVERSE_INDEX}")
            # QLC+ counts addresses from 0, DMX from 1.
            return {
                "universe": int(universe),
                "address": int(fixture.findtext(f"{QLC}Address")) + 1,
                "manufacturer": fixture.findtext(f"{QLC}Manufacturer", ""),
                "model": fixture.findtext(f"{QLC}Model", ""),
                "mode": fixture.findtext(f"{QLC}Mode", ""),
            }
    fail(f"{path}: no fixture named Wrist")


# ── wrist.qxf ────────────────────────────────────────────────────────────────────────────────

QXF = "{http://www.qlcplus.org/FixtureDefinition}"


def ident(text):
    """A band's or channel's name as a C++ identifier: what comes before its first comma, in
    words joined by `_`. `Clockwise, in Manual only` is `clockwise`."""
    words = re.findall(r"[a-z0-9]+", text.split(",")[0].lower())
    if not words or words[0][0].isdigit():
        fail(f"{text!r} cannot name a C++ constant")
    return "_".join(words)


def read_definition(path, patch):
    """The patched mode's channels, in order, each with its bands, from the fixture definition
    the workspace names. Every channel's bands must cover 0–255, in order, with no gap and no
    overlap, so any level the firmware reads falls in exactly one."""
    root = ET.parse(path).getroot()
    found = (root.findtext(f"{QXF}Manufacturer", ""), root.findtext(f"{QXF}Model", ""))
    if found != (patch["manufacturer"], patch["model"]):
        fail(f"{path} is {found[0]} {found[1]}, and the workspace patches Wrist as {patch['manufacturer']} {patch['model']}")
    mode = root.find(f"{QXF}Mode[@Name='{patch['mode']}']")
    if mode is None:
        fail(f"{path} has no mode {patch['mode']!r}, which the workspace patches Wrist in")
    channels = []
    for number, entry in enumerate(sorted(mode.findall(f"{QXF}Channel"), key=lambda c: int(c.get("Number")))):
        if int(entry.get("Number")) != number:
            fail(f"{path}: mode {patch['mode']!r} has no channel {number}")
        name = entry.text.strip()
        channel = root.find(f"{QXF}Channel[@Name='{name}']")
        if channel is None:
            fail(f"{path}: mode {patch['mode']!r} names a channel {name!r} the file does not define")
        bands, expect = [], 0
        for capability in channel.findall(f"{QXF}Capability"):
            low, high = int(capability.get("Min")), int(capability.get("Max"))
            if low != expect or high < low:
                fail(f"{path}: {name}'s band {capability.text!r} is {low}–{high}, and the bands before it end at {expect - 1}")
            bands.append((ident(capability.text), low, high))
            expect = high + 1
        if expect != 256:
            fail(f"{path}: {name}'s bands end at {expect - 1}, not 255")
        if len({b[0] for b in bands}) != len(bands):
            fail(f"{path}: two of {name}'s bands have the same name")
        channels.append((ident(name), bands))
    return channels


# ── wrist.oscquery.json ──────────────────────────────────────────────────────────────────────

TYPES = {"i": "int32_t", "f": "float", "s": "const char *", "T": "bool", "F": "bool"}
KEYWORDS = {"default", "delete", "new", "class", "int", "float", "bool", "char", "true", "false", "return"}


def camel(name):
    return "".join(p.capitalize() for p in name.split("_"))


def rate_name(hz):
    return f"{hz:g}".replace(".", "_") + "hz"


def cpp_type(leaf):
    return camel(leaf["name"]) if leaf["values"] else TYPES[leaf["type"]]


def declare(kind, name):
    """`kind name`, with a pointer's `*` against the name, as the hand-written code has it."""
    return f"{kind}{name}" if kind.endswith("*") else f"{kind} {name}"


def write_leaf(leaf, value):
    """The lines that write one message for `leaf`, whose value is the expression `value`."""
    p = leaf["path"]
    if leaf["type"] in ("T", "F"):
        return [f'w.address("{p}");', f'w.string({value} ? ",T" : ",F");']
    lines = [f'w.address("{p}");', f'w.string(",{leaf["type"]}");']
    if leaf["values"]:
        lines.append(f"w.string({camel(leaf['name'])}Names[static_cast<uint8_t>({value})]);")
    else:
        lines.append({"i": f"w.int32({value});", "f": f"w.float32({value});", "s": f"w.string({value});"}[leaf["type"]])
    return lines


def bundle(struct, fn, group, out):
    out.append(f"struct {struct} {{")
    for leaf in group:
        out.append(f"  {declare(cpp_type(leaf), leaf['name'])};")
    out.append("};")
    out.append("")
    out.append(f"inline size_t {fn}(const {struct} &v, uint8_t *buf, size_t cap) {{")
    out.append("  Writer w(buf, cap);")
    out.append("  w.beginBundle();")
    for leaf in group:
        out.append("  {")
        out.append("    const size_t at = w.beginElement();")
        out += ["    " + line for line in write_leaf(leaf, f"v.{leaf['name']}")]
        out.append("    w.endElement(at);")
        out.append("  }")
    out.append("  return w.length();")
    out.append("}")
    out.append("")


def encoders(path):
    found = oscquery.leaves(path)
    for leaf in found:
        for v in leaf["values"] or []:
            if v in KEYWORDS:
                fail(f"{path}: {leaf['path']}: the value {v!r} cannot name a C++ enumerator")

    out = ["namespace osc {", ""]
    for leaf in found:
        if leaf["values"]:
            name = camel(leaf["name"])
            out.append(f"// {leaf['path']}, in the order the file lists its values.")
            out.append(f"enum class {name} : uint8_t {{ {', '.join(leaf['values'])} }};")
            out.append(f"constexpr const char *{name}Names[] = {{{', '.join(chr(34) + v + chr(34) for v in leaf['values'])}}};")
            out.append("")

    connect = [l for l in found if l["how"] == "connect"]
    if connect:
        out.append("// Everything sent each time Wrist joins the WiFi, as one bundle.")
        bundle("OnConnect", "encodeOnConnect", connect, out)

    rates = sorted({l["how"] for l in found if not isinstance(l["how"], str)})
    for hz in rates:
        group = [l for l in found if l["how"] == hz]
        name = rate_name(hz)
        out.append(f"// Everything sent {hz:g} times a second, as one bundle.")
        out.append(f"constexpr uint32_t AT_{name.upper()}_PERIOD_US = {round(1e6 / hz)};")
        bundle(f"At{name.capitalize()}", f"encodeAt{name.capitalize()}", group, out)

    for leaf in (l for l in found if l["how"] == "data"):
        out.append(f"// {leaf['path']}: one message, sent once when a new reading arrives.")
        out.append(f"inline size_t encode{camel(leaf['name'])}({declare(cpp_type(leaf), 'value')}, uint8_t *buf, size_t cap) {{")
        out.append("  Writer w(buf, cap);")
        out += ["  " + line for line in write_leaf(leaf, "value")]
        out.append("  return w.length();")
        out.append("}")
        out.append("")
    out.append("}  // namespace osc")
    return out


# ── The header ───────────────────────────────────────────────────────────────────────────────


def c_string(text):
    return '"' + text.replace("\\", "\\\\").replace('"', '\\"') + '"'


def c_ip(address):
    return "{" + ", ".join(str(b) for b in address.packed) + "}"


def main(dot_path, qxw_path, qxf_path, oscquery_path, out_path):
    net = read_dot(dot_path)
    patch = read_workspace(qxw_path)
    channels = read_definition(qxf_path, patch)
    osc = encoders(oscquery_path)

    out = [
        "// Generated by generate.py from clockwork.dot, clockwork.qxw, wrist.qxf and",
        "// wrist.oscquery.json.",
        "// Do not edit: it is rebuilt on every build, and git ignores it.",
        "#pragma once",
        "",
        "#include <stddef.h>",
        "#include <stdint.h>",
        "",
        '#include "osc_writer.h"',
        "",
        "namespace net {",
        "",
        "// The WiFi, from clockwork.dot's networks.",
        f"constexpr const char *WIFI_SSID = {c_string(net['ssid'])};",
        f"constexpr const char *WIFI_PSK = {c_string(net['psk'])};",
        f"constexpr uint8_t WIFI_CHANNEL = {net['channel']};",
        "",
        "// Wrist's own address on it.",
        f"constexpr uint8_t ADDRESS[4] = {c_ip(net['address'])};",
        f"constexpr uint8_t GATEWAY[4] = {c_ip(net['gateway'])};",
        f"constexpr uint8_t NETMASK[4] = {c_ip(net['netmask'])};",
        "",
        "// The port Wrist listens on for sACN.",
        f"constexpr uint16_t SACN_PORT = {net['sacn_port']};",
        "",
        "// Where Wrist's OSC goes: one entry per OSC edge from it.",
        "struct Recipient {",
        "  const char *name;",
        "  uint8_t address[4];",
        "  uint16_t port;",
        "};",
        "constexpr Recipient OSC_RECIPIENTS[] = {",
    ]
    for name, address, port in net["recipients"]:
        out.append(f"    {{{c_string(name)}, {c_ip(address)}, {port}}},")
    out += [
        "};",
        "",
        "}  // namespace net",
        "",
        "namespace patch {",
        "",
        "// From clockwork.qxw: the universe its E1.31 output sends, and Wrist's start address.",
        f"constexpr uint16_t SACN_UNIVERSE = {patch['universe']};",
        f"constexpr uint16_t DMX_ADDRESS = {patch['address']};",
        "",
        "// A range of levels on one channel, as wrist.qxf names it.",
        "struct Band {",
        "  uint8_t min;",
        "  uint8_t max;",
        "  constexpr bool holds(uint8_t level) const { return min <= level && level <= max; }",
        "};",
        "",
        f"// From wrist.qxf, the mode \"{patch['mode']}\": each channel's offset from DMX_ADDRESS, and",
        "// its bands, which cover 0–255 between them.",
        f"constexpr uint8_t CHANNELS = {len(channels)};",
    ]
    for offset, (name, bands) in enumerate(channels):
        out.append(f"namespace {name} {{")
        out.append(f"constexpr uint8_t OFFSET = {offset};")
        for band, low, high in bands:
            out.append(f"constexpr Band {band.upper()} = {{{low}, {high}}};")
        out.append(f"}}  // namespace {name}")
    out += [
        "",
        "}  // namespace patch",
        "",
    ]
    out += osc
    with open(out_path, "w", encoding="utf-8") as f:
        f.write("\n".join(out) + "\n")


if __name__ == "__main__":
    if len(sys.argv) != 6:
        fail(__doc__.strip().splitlines()[2].strip())
    main(*sys.argv[1:])
