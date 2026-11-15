#!/usr/bin/env python3
"""Reads a device's OSCQuery file in `fixtures/`, checking it as DESIGN.md's OSC section states.

    python3 -I clockwork/oscquery.py <device.oscquery.json> [--shared]

Prints one line per leaf, an OSC address, in the file's order: `<path> <type> <how>
[<value>...]`, how being `connect`, `data` or a rate in Hz, and the values being a string
leaf's `VALS`, if it lists them. The shared files the device
implements are merged in, so their leaves are printed under the device's name. With
`--shared`, prints only `shared` or `device`: whether the file is a shared subtree.

Wrist's `generate.py` imports it for the firmware's encoders, and `cortex-build` runs it for the
brain's decoders, so both builds read the file through one reader and refuse the same files.
"""

import json
import os
import re
import sys

# What a shared file names the device that implements it by.
DEVICE = "{device}"
# The types the encoders and decoders know.
TYPES = ("i", "f", "s", "T", "F")
# A listed value names an enumerator in C++ and, capitalised, a variant in Rust.
VALUE = re.compile(r"[a-z_][a-z0-9_]*")


def fail(message):
    sys.exit(f"{os.path.basename(sys.argv[0])}: {message}")


def load(path):
    with open(path, encoding="utf-8") as f:
        try:
            return json.load(f)
        except json.JSONDecodeError as e:
            fail(f"{path}: {e}")


def is_shared(path):
    """Whether the file at `path` is a shared subtree, such as `dmx` or `health`, that devices
    implement, rather than a device's own: whether a node in it is keyed `{device}`. Only the
    keys count, so a description that mentions `{device}` does not make a device's file
    shared."""
    def keyed(node):
        contents = node.get("CONTENTS", {})
        return DEVICE in contents or any(keyed(child) for child in contents.values())
    return keyed(load(path))


def leaves(path):
    """Every leaf of the file at `path`, as dicts of `path`, `name`, `type`, `how`, `values` and
    `sent_by`: the device that sends it, where that is not the file's own.

    Each shared file the device's HOST_INFO lists under CLOCKWORK_CHAOS_IMPLEMENTS is read from
    beside it, `{device}` replaced by the device's name, and merged into its tree."""
    tree = load(path)
    host = tree.get("HOST_INFO", {})
    device = host.get("NAME")
    for shared in host.get("CLOCKWORK_CHAOS_IMPLEMENTS", []):
        shared_path = os.path.join(os.path.dirname(path), f"{shared}.oscquery.json")
        text = json.dumps(load(shared_path)).replace(DEVICE, device)
        merge(path, tree, json.loads(text))
    found = []
    walk(path, tree, None, None, found)
    return found


def merge(file, into, shared):
    """Adds `shared`'s subtrees to `into`'s, refusing a leaf that both have."""
    for key, child in shared.get("CONTENTS", {}).items():
        mine = into.setdefault("CONTENTS", {}).get(key)
        if mine is None:
            into["CONTENTS"][key] = child
        elif "CONTENTS" in mine and "CONTENTS" in child:
            merge(file, mine, child)
        else:
            fail(f"{file}: {child.get('FULL_PATH')} is in a shared file as well")


def walk(file, node, path, key, found):
    full = node.get("FULL_PATH")
    want = "/" if key is None else ("" if path == "/" else path) + "/" + key
    if full != want:
        fail(f"{file}: FULL_PATH {full!r} should be {want!r}")
    if "CONTENTS" in node:
        if "TYPE" in node:
            fail(f"{file}: {full} has both CONTENTS and a TYPE")
        for child_key, child in node["CONTENTS"].items():
            walk(file, child, full, child_key, found)
        return
    kind = node.get("TYPE")
    if kind not in TYPES:
        fail(f"{file}: {full} has TYPE {kind!r}; the encoders know {', '.join(TYPES)}")
    if "ACCESS" not in node:
        fail(f"{file}: {full} has no ACCESS")
    ways = [w for w in ("CLOCKWORK_CHAOS_ON_EVENT", "CLOCKWORK_CHAOS_AT_RATE_HZ") if w in node]
    if len(ways) != 1:
        fail(f"{file}: {full} is sent {len(ways)} ways; each leaf is sent exactly one")
    how = node[ways[0]]
    if ways[0] == "CLOCKWORK_CHAOS_ON_EVENT":
        if how not in ("connect", "data"):
            fail(f"{file}: {full} is sent on {how!r}; the events are connect and data")
    elif not isinstance(how, (int, float)) or isinstance(how, bool) or how <= 0:
        fail(f"{file}: {full} has a rate of {how!r}")
    values = None
    for r in node.get("RANGE", []):
        if kind == "s" and "VALS" in r:
            values = r["VALS"]
            for v in values:
                if not isinstance(v, str) or not VALUE.fullmatch(v):
                    fail(f"{file}: {full}: the value {v!r} cannot name an enumerator")
            if len(set(values)) != len(values):
                fail(f"{file}: {full} lists a value twice")
    found.append({"path": full, "name": full.strip("/").replace("/", "_"), "type": kind, "how": how, "values": values,
                  "sent_by": node.get("CLOCKWORK_CHAOS_SENT_BY")})


def main(path):
    for leaf in leaves(path):
        print(" ".join([leaf["path"], leaf["type"], f"{leaf['how']:g}" if not isinstance(leaf["how"], str) else leaf["how"]]
                       + (leaf["values"] or [])))


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[2] == "--shared":
        print("shared" if is_shared(sys.argv[1]) else "device")
    elif len(sys.argv) == 2:
        main(sys.argv[1])
    else:
        fail(__doc__.strip().splitlines()[2].strip())
