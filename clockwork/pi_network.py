#!/usr/bin/env python3
"""Renders a Pi's network configuration from `clockwork.dot`, filling a template's `@…@`
placeholders with what the dot says about the device.

    python3 -I clockwork/pi_network.py <clockwork.dot> <device> <out-dir> <template>...

Each template is written to `<out-dir>` under its own name. The placeholders, for the device's
interface `<if>` and each edge from one of its services, `<service>`, to a port `<port>`:

  @<IF>_ADDRESS@        the interface's fixed address, with its prefix: `10.0.10.2/24`.
  @<IF>_GATEWAY@        its network's gateway.
  @<IF>_DHCP_RANGE@     its network's DHCP range, as NetworkManager's shared mode takes it:
                        `first,last`. Only where the interface is the gateway, the one device
                        that hands out the leases.
  @<IF>_SSID@, @<IF>_PSK@, @<IF>_CHANNEL@
                        its network's WiFi, where the network is one.
  @<SERVICE>_<PORT>_ADDRESS@, @<SERVICE>_<PORT>_PORT@
                        where the service sends to that port, on the network the two devices
                        share.

Upper case, `-` as `_`: `@WLAN0_ADDRESS@`, `@STEM_MQTT_ADDRESS@`. A placeholder the dot does
not fill is an error, as is one it fills two ways, so a template cannot install with a hole in
it. A fact is only worked out when a template asks for it: the dot is checked for what the Pi
uses, and a node no template names, like a hub whose port is not known yet, is no obstacle.
"""

import pathlib
import re
import sys

# The dot's reader, beside this file, shared with wrist/generate.py. Put on the path by hand:
# run with -I, Python adds no directory of its own, not even this one.
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from dot import Dot, addresses, fail, listener, networks  # noqa: E402

PLACEHOLDER = re.compile(r"@([A-Z0-9_]+)@")


def key(*parts):
    return "_".join(parts).upper().replace("-", "_")


def facts(dot, device):
    """Every placeholder the dot can fill for `device`, each as a function that works it out."""
    nets = networks(dot)
    held = addresses(dot, device, nets)
    found = {}

    def add(name, value):
        # Filled two ways: an error only if a template asks for it.
        found[name] = (lambda: fail(f"{dot.path}: @{name}@ means more than one thing on {device}")) if name in found else value

    for interface, (address, network) in held.items():
        net = nets[network]
        if address is not None and "cidr" in net:
            add(key(interface, "address"), lambda a=address, n=net: f"{a}/{n['cidr'].prefixlen}")
        if "gateway" in net:
            add(key(interface, "gateway"), lambda n=net: n["gateway"])
        if "dhcp" in net and "gateway" in net and str(address) == net["gateway"]:
            add(key(interface, "dhcp_range"), lambda n=net: f"{n['dhcp'][0]},{n['dhcp'][1]}")
        for wifi in ("ssid", "psk", "channel"):
            if wifi in net:
                add(key(interface, wifi), lambda n=net, w=wifi: n[w])

    own = {net for _, net in held.values()}
    for tail, tail_port, head, head_port, _ in dot.edges:
        if dot.device_of(tail) != device or head_port is None:
            continue
        service = tail.removeprefix(f"{device}_")

        def where(tail=tail, head=head, head_port=head_port):
            if tail_port != "title":
                fail(f"{dot.path}: the edge from {tail} must run from its title")
            theirs = {net for _, net in addresses(dot, dot.device_of(head), nets).values()}
            shared = sorted(own & theirs)
            if len(shared) != 1:
                fail(f"{dot.path}: {device} and {dot.device_of(head)} share {len(shared)} networks, not one, for {tail} -> {head}:{head_port}")
            return listener(dot, nets, head, head_port, shared[0])

        add(key(service, head_port, "address"), lambda w=where: w()[0])
        add(key(service, head_port, "port"), lambda w=where: w()[1])
    return found


def render(text, known, template, device, dot):
    def fill(m):
        if m.group(1) not in known:
            fail(f"{template}: @{m.group(1)}@ is not a fact {dot.path} holds about {device}")
        return str(known[m.group(1)]())

    return PLACEHOLDER.sub(fill, text)


def main(dot_path, device, out_dir, *templates):
    dot = Dot(dot_path)
    if f"{device}_if" not in dot.nodes:
        fail(f"{dot_path}: no device {device!r}")
    known = facts(dot, device)
    out = pathlib.Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    for template in templates:
        text = pathlib.Path(template).read_text(encoding="utf-8")
        (out / pathlib.Path(template).name).write_text(render(text, known, template, device, dot), encoding="utf-8")


if __name__ == "__main__":
    if len(sys.argv) < 5:
        fail(__doc__.strip().splitlines()[3].strip())
    main(*sys.argv[1:])
