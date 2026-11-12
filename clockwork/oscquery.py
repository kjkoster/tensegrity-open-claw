#!/usr/bin/env python3
"""Reads a device's OSCQuery file in `fixtures/`, checking it as DESIGN.md's OSC section states.

    python3 -I clockwork/oscquery.py <device.oscquery.json>

Prints one line per leaf, an OSC address, in the file's order: `<path> <type> [<value>...]`,
the values being a string leaf's `VALS`, if it lists them. How each is sent is the sender's
business, so only `leaves()` returns it, for the encoders.

Wrist's `generate.py` imports it for the firmware's encoders, and `cortex-build` runs it for the
brain's decoders, so both builds read the file through one reader and refuse the same files.
"""

import json
import os
import re
import sys

# The types the encoders and decoders know.
TYPES = ("i", "f", "s", "T", "F")
# A listed value names an enumerator in C++ and, capitalised, a variant in Rust.
VALUE = re.compile(r"[a-z_][a-z0-9_]*")


def fail(message):
    sys.exit(f"{os.path.basename(sys.argv[0])}: {message}")


def leaves(path):
    """Every leaf of the file at `path`, as dicts of `path`, `name`, `type`, `how` and `values`."""
    with open(path, encoding="utf-8") as f:
        try:
            tree = json.load(f)
        except json.JSONDecodeError as e:
            fail(f"{path}: {e}")
    found = []
    walk(path, tree, None, None, found)
    return found


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
    found.append({"path": full, "name": full.strip("/").replace("/", "_"), "type": kind, "how": how, "values": values})


def main(path):
    for leaf in leaves(path):
        print(" ".join([leaf["path"], leaf["type"]] + (leaf["values"] or [])))


if __name__ == "__main__":
    if len(sys.argv) != 2:
        fail(__doc__.strip().splitlines()[2].strip())
    main(sys.argv[1])
