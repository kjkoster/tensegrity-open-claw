#!/usr/bin/env python3
"""Prints where a service sends one kind of traffic: one line per edge from it to a port of
that name, `<address>:<port>`, in the order the dot lists the edges.

    python3 -I clockwork/recipients.py <clockwork.dot> <node> <port>

`chaos_pi_brain sacn` prints every fixture the brain sends sACN to. A fact `facts.py` cannot
hold, because it is a list: two edges to two `sacn` ports would be one placeholder filled two
ways. None is a list of none, not an error. `cortex-build` runs it.
"""

import pathlib
import sys

# The dot's reader, beside this file. Put on the path by hand: run with -I, Python adds no
# directory of its own, not even this one.
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from dot import Dot, fail, networks, reach  # noqa: E402


def main(dot_path, node, port):
    dot = Dot(dot_path)
    if node not in dot.nodes:
        fail(f"{dot_path}: no node {node!r}")
    nets = networks(dot)
    for tail, tail_port, head, head_port, _ in dot.edges:
        if tail == node and head_port == port:
            address, number = reach(dot, nets, tail, tail_port, head, head_port)
            print(f"{address}:{number}")


if __name__ == "__main__":
    if len(sys.argv) != 4:
        fail(__doc__.strip().splitlines()[3].strip())
    main(*sys.argv[1:])
