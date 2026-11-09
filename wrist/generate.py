#!/usr/bin/env python3
"""Writes `generated.h`, the facts Wrist's firmware takes from the rig's source files.

    python3 -I generate.py <clockwork.dot> <clockwork.qxw> <wrist.oscquery.json> <generated.h>

Each fact is read from the one file that records it, and only what the firmware uses is read:

  clockwork.dot          the WiFi: SSID, passphrase, channel; Wrist's address, the gateway and
                         the netmask; the port Wrist listens on for sACN; where its OSC goes.
  clockwork.qxw          the sACN universe and Wrist's DMX start address.
  wrist.oscquery.json    every OSC address Wrist sends, and how: the encoders.

Fails, naming the file and what is wrong, rather than writing a header from a guess. The header
is git-ignored: it is rebuilt from these files on every build, so it is never edited by hand.
"""

import ipaddress
import json
import pathlib
import re
import sys
import xml.etree.ElementTree as ET

# The dot's reader, in clockwork/, shared with pi_network.py there. Put on the path by hand:
# run with -I, Python adds no directory of its own.
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent / "clockwork"))
from dot import Dot, addresses, fail, listener, networks  # noqa: E402

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
            return {"universe": int(universe), "address": int(fixture.findtext(f"{QLC}Address")) + 1}
    fail(f"{path}: no fixture named Wrist")


# ── wrist.oscquery.json ──────────────────────────────────────────────────────────────────────

TYPES = {"i": "int32_t", "f": "float", "s": "const char *", "T": "bool", "F": "bool"}
KEYWORDS = {"default", "delete", "new", "class", "int", "float", "bool", "char", "true", "false", "return"}


def leaves(node, path, key, found):
    """Walks the tree, checking it as DESIGN.md's OSC section states, and collects the leaves."""
    full = node.get("FULL_PATH")
    want = "/" if key is None else ("" if path == "/" else path) + "/" + key
    if full != want:
        fail(f"FULL_PATH {full!r} should be {want!r}")
    if "CONTENTS" in node:
        if "TYPE" in node:
            fail(f"{full} has both CONTENTS and a TYPE")
        for child_key, child in node["CONTENTS"].items():
            leaves(child, full, child_key, found)
        return found
    kind = node.get("TYPE")
    if kind not in TYPES:
        fail(f"{full} has TYPE {kind!r}; the encoders know {', '.join(TYPES)}")
    if "ACCESS" not in node:
        fail(f"{full} has no ACCESS")
    ways = [w for w in ("CLOCKWORK_CHAOS_ON_EVENT", "CLOCKWORK_CHAOS_AT_RATE_HZ") if w in node]
    if len(ways) != 1:
        fail(f"{full} is sent {len(ways)} ways; each leaf is sent exactly one")
    if ways[0] == "CLOCKWORK_CHAOS_ON_EVENT":
        how = node[ways[0]]
        if how not in ("connect", "data"):
            fail(f"{full} is sent on {how!r}; the events are connect and data")
    else:
        how = node[ways[0]]
        if not isinstance(how, (int, float)) or how <= 0:
            fail(f"{full} has a rate of {how!r}")
    values = None
    for r in node.get("RANGE", []):
        if kind == "s" and "VALS" in r:
            values = r["VALS"]
            for v in values:
                if not re.fullmatch(r"[a-z_][a-z0-9_]*", v) or v in KEYWORDS:
                    fail(f"{full}: the value {v!r} cannot name a C++ enumerator")
    found.append({"path": full, "name": full.strip("/").replace("/", "_"), "type": kind, "how": how, "values": values})
    return found


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
    with open(path, encoding="utf-8") as f:
        try:
            tree = json.load(f)
        except json.JSONDecodeError as e:
            fail(f"{path}: {e}")
    try:
        found = leaves(tree, None, None, [])
    except SystemExit as e:
        fail(f"{path}: {str(e.code).removeprefix('generate.py: ')}")

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


def main(dot_path, qxw_path, oscquery_path, out_path):
    net = read_dot(dot_path)
    patch = read_workspace(qxw_path)
    osc = encoders(oscquery_path)

    out = [
        "// Generated by generate.py from clockwork.dot, clockwork.qxw and wrist.oscquery.json.",
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
        "}  // namespace patch",
        "",
    ]
    out += osc
    with open(out_path, "w", encoding="utf-8") as f:
        f.write("\n".join(out) + "\n")


if __name__ == "__main__":
    if len(sys.argv) != 5:
        fail(__doc__.strip().splitlines()[2].strip())
    main(*sys.argv[1:])
