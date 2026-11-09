#!/usr/bin/env python3
"""Prints facts `clockwork.dot` holds about one device, for a build that is not Python.

    python3 -I clockwork/facts.py <clockwork.dot> <device> <NAME>...

The names are `pi_network.py`'s placeholders, without the `@`s: `BRAIN_MQTT_ADDRESS`,
`BRAIN_SACN_BIND_PORT`. One line per name, in the order asked, `NAME value`. A name the dot
does not fill, or fills two ways, is an error, as it is in a template, so the build that asked
fails rather than compile in a guess.

`cortex-build` runs it, so the Rust build reads the dot through the same reader as the Pis'
deploys and Wrist's build, and refuses the same files.
"""

import pathlib
import sys

# The dot's reader and the facts, beside this file. Put on the path by hand: run with -I,
# Python adds no directory of its own, not even this one.
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from dot import Dot, fail  # noqa: E402
from pi_network import facts  # noqa: E402


def main(dot_path, device, *names):
    dot = Dot(dot_path)
    if f"{device}_if" not in dot.nodes:
        fail(f"{dot_path}: no device {device!r}")
    known = facts(dot, device)
    for name in names:
        if name not in known:
            fail(f"{dot_path}: {name} is not a fact it holds about {device}")
        print(name, known[name]())


if __name__ == "__main__":
    if len(sys.argv) < 4:
        fail(__doc__.strip().splitlines()[2].strip())
    main(*sys.argv[1:])
