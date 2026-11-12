"""Reads `clockwork.dot`, the rig's IP space, for the generators that compile config from it:
`wrist/generate.py` for Wrist's firmware, `pi_network.py` for the Pis, and `facts.py` for the
Rust build.

Only the subset of the DOT language the file uses: subgraphs, node, edge and attribute
statements, quoted strings and HTML-like labels. A statement this parser does not know is an
error, so the file cannot drift into a shape a generator silently misreads. The grammar the
labels follow is in the comment at the top of `clockwork.dot`.

Every failure exits naming the script that ran, the file and what is wrong, rather than let a
generator write config from a guess.
"""

import ipaddress
import os
import re
import shlex
import sys


def fail(message):
    sys.exit(f"{os.path.basename(sys.argv[0])}: {message}")


TOKEN = re.compile(
    r"""
      (?P<space>\s+|//[^\n]*)
    | (?P<string>"(?:[^"\\]|\\.)*")
    | (?P<html><)
    | (?P<arrow>->)
    | (?P<id>[A-Za-z0-9_.]+)
    | (?P<punct>[{}\[\];,=:])
    """,
    re.VERBOSE,
)


def tokens(text, path):
    at = 0
    while at < len(text):
        m = TOKEN.match(text, at)
        if not m:
            fail(f"{path}: cannot read from {text[at:at + 30]!r}")
        kind = m.lastgroup
        if kind == "space":
            at = m.end()
        elif kind == "string":
            yield "id", m.group()[1:-1].replace('\\"', '"'), False
            at = m.end()
        elif kind == "html":
            # An HTML-like label runs to the `>` that balances its opening `<`.
            depth, end = 0, at
            while True:
                if end == len(text):
                    fail(f"{path}: an HTML-like label is never closed")
                depth += {"<": 1, ">": -1}.get(text[end], 0)
                end += 1
                if depth == 0:
                    break
            yield "id", text[at + 1:end - 1], True
            at = end
        else:
            yield kind if kind != "punct" else m.group(), m.group(), False
            at = m.end()


class Dot:
    """The devices, the nodes on each, and the edges, as the file states them."""

    def __init__(self, path):
        self.path = path
        self.nodes = {}  # name -> (device or None, label, is_html)
        self.edges = []  # (tail, tail_port, head, head_port, attrs)
        with open(path, encoding="utf-8") as f:
            self.toks = list(tokens(f.read(), path))
        self.at = 0
        self.expect("id", "digraph")
        self.take("id")
        self.expect("{")
        self.statements(None)
        # A stray `}` part-way through closes the graph early, and without this everything
        # after it would be silently dropped.
        if self.at < len(self.toks):
            fail(f"{self.path}: {self.toks[self.at][1]!r} after the graph's closing brace")

    def peek(self, offset=0):
        i = self.at + offset
        return self.toks[i] if i < len(self.toks) else (None, None, False)

    def take(self, kind):
        tok = self.peek()
        if tok[0] != kind:
            fail(f"{self.path}: expected {kind}, found {tok[1]!r}")
        self.at += 1
        return tok

    def expect(self, kind, value=None):
        tok = self.take(kind)
        if value is not None and tok[1] != value:
            fail(f"{self.path}: expected {value!r}, found {tok[1]!r}")

    def attrs(self):
        found = {}
        if self.peek()[0] != "[":
            return found
        self.take("[")
        while self.peek()[0] != "]":
            key = self.take("id")[1]
            self.expect("=")
            _, value, html = self.take("id")
            found[key] = (value, html)
            if self.peek()[0] in (",", ";"):
                self.at += 1
        self.take("]")
        return found

    def endpoint(self):
        name = self.take("id")[1]
        port = None
        if self.peek()[0] == ":":
            self.take(":")
            port = self.take("id")[1]
        return name, port

    def statements(self, device):
        while True:
            kind, value, _ = self.peek()
            if kind == "}":
                self.take("}")
                return
            if kind == "id" and value == "subgraph":
                self.take("id")
                name = self.take("id")[1]
                self.expect("{")
                inner = name[len("cluster_"):] if name.startswith("cluster_") else device
                self.statements(inner)
            elif kind == "id" and value in ("graph", "node", "edge") and self.peek(1)[0] == "[":
                self.take("id")
                self.attrs()
            elif kind == "id" and self.peek(1)[0] == "=":
                self.at += 3
            elif kind == "id":
                tail, tail_port = self.endpoint()
                if self.peek()[0] == "arrow":
                    self.take("arrow")
                    head, head_port = self.endpoint()
                    self.edges.append((tail, tail_port, head, head_port, self.attrs()))
                else:
                    label, html = self.attrs().get("label", (tail, False))
                    self.nodes[tail] = (device, label, html)
            else:
                fail(f"{self.path}: unexpected {value!r}")
            if self.peek()[0] == ";":
                self.take(";")

    def record(self, name):
        """A record label's fields after the first, each without its `<port>`."""
        if name not in self.nodes:
            fail(f"{self.path}: no node {name!r}")
        _, label, _ = self.nodes[name]
        return [re.sub(r"^<\w+>\s*", "", f).strip() for f in label.split("|")[1:]]

    def cell(self, name, port):
        """The text of a service's cell at `port`."""
        if name not in self.nodes:
            fail(f"{self.path}: no node {name!r}")
        _, label, html = self.nodes[name]
        if not html:
            fail(f"{self.path}: {name} is a service, and has no table for its ports")
        m = re.search(rf'<td port="{re.escape(port)}">(.*?)</td>', label)
        if not m:
            fail(f"{self.path}: {name} has no port {port!r}")
        return re.sub(r"<[^>]*>", "", m.group(1)).strip()

    def ports(self, name):
        """A service's ports, in the order its table lists them, without its title."""
        if name not in self.nodes:
            fail(f"{self.path}: no node {name!r}")
        _, label, html = self.nodes[name]
        if not html:
            return []
        return [p for p in re.findall(r'<td port="([^"]+)">', label) if p != "title"]

    def device_of(self, name):
        if name not in self.nodes:
            fail(f"{self.path}: no node {name!r}")
        return self.nodes[name][0]


def parse_address(dot, text, what):
    """`text` as an IPv4 address, or a failure naming the file and `what` it was to be."""
    try:
        return ipaddress.IPv4Address(text)
    except ValueError:
        fail(f"{dot.path}: {what} {text!r} is not an IPv4 address")


def parse_network(dot, text, what):
    """`text` as an IPv4 network with no host bits set, or a failure naming the file and `what`
    it was to be."""
    try:
        return ipaddress.IPv4Network(text)
    except ValueError as e:
        fail(f"{dot.path}: {what} {text!r} is not an IPv4 network: {e}")


def networks(dot):
    """Every network by name: its CIDR, its DHCP range as a (first, last) pair, and the other
    attributes after it as text."""
    found = {}
    for field in dot.record("networks"):
        words = shlex.split(field)
        if len(words) < 2:
            fail(f"{dot.path}: the network {field!r} has no CIDR, nor `dhcp`")
        net = {"name": words[0]}
        if words[1] != "dhcp":
            net["cidr"] = parse_network(dot, words[1], f"the {net['name']} network's CIDR")
        rest = words[2:]
        while rest:
            key = rest.pop(0)
            if not rest:
                fail(f"{dot.path}: the {net['name']} network's {key!r} has no value")
            net[key] = rest.pop(0)
        if "dhcp" in net:
            ends = re.split(r"[–-]", net["dhcp"])
            if len(ends) != 2 or "cidr" not in net:
                fail(f"{dot.path}: the {net['name']} network's DHCP range {net['dhcp']!r} is not `first–last` in a CIDR")
            first, last = (parse_address(dot, e, f"the {net['name']} network's DHCP range end") for e in ends)
            if first not in net["cidr"] or last not in net["cidr"] or first > last:
                fail(f"{dot.path}: the {net['name']} network's DHCP range {net['dhcp']!r} is not inside {net['cidr']}")
            net["dhcp"] = (first, last)
        found[net["name"]] = net
    return found


MAC = re.compile(r"[0-9a-f]{2}(?::[0-9a-f]{2}){5}")

# The chips esptool flashes, by the names its `--chip` takes.
CHIPS = {"esp8266", "esp32", "esp32s2", "esp32s3", "esp32c2", "esp32c3", "esp32c6", "esp32h2"}


def interface_field(dot, device, field):
    """One field of a device's `<dev>_if`: its interface, its address, the words after them
    without `ap`, `mac <mac>` and `chip <chip>`, and the MAC and the chip, each None where it
    names none. A MAC comes with its chip, and a chip with its MAC: they say together which
    board this is and how it is flashed."""
    words = field.split()
    if len(words) < 2:
        fail(f"{dot.path}: {device}'s address {field!r} is not `interface address [network]`")
    rest, mac, chip = [], None, None
    words_after = iter(words[2:])
    for word in words_after:
        if word == "ap":
            continue
        if word == "mac":
            mac = next(words_after, None)
            if mac is None or not MAC.fullmatch(mac):
                fail(f"{dot.path}: {device}'s {words[0]} has `mac {mac or ''}`, not a MAC in lower case, `aa:bb:cc:dd:ee:ff`")
            continue
        if word == "chip":
            chip = next(words_after, None)
            if chip not in CHIPS:
                fail(f"{dot.path}: {device}'s {words[0]} has `chip {chip or ''}`, not one of esptool's: {', '.join(sorted(CHIPS))}")
            continue
        rest.append(word)
    if (mac is None) != (chip is None):
        fail(f"{dot.path}: {device}'s {words[0]} names a {'MAC' if mac else 'chip'} without a {'chip' if mac else 'MAC'}")
    return words[0], words[1], rest, mac, chip


def macs(dot):
    """Every MAC the dot names, each to the (device, interface, chip) it belongs to. A MAC two
    interfaces share is an error: it is what tells one board from another."""
    found = {}
    for node in dot.nodes:
        if not node.endswith("_if"):
            continue
        device = node[: -len("_if")]
        for field in dot.record(node):
            name, _, _, mac, chip = interface_field(dot, device, field)
            if mac is None:
                continue
            if mac in found:
                fail(f"{dot.path}: {mac} is both {found[mac][0]}'s {found[mac][1]} and {device}'s {name}")
            found[mac] = (device, name, chip)
    return found


def addresses(dot, device, nets):
    """A device's addresses by interface: (address, network). A lease has no address. A fixed
    address must lie in its network's CIDR and outside its DHCP range, which a lease could
    otherwise hand to another device."""
    found = {}
    for field in dot.record(f"{device}_if"):
        interface, address, rest, _, _ = interface_field(dot, device, field)
        if address == "dhcp":
            if not rest:
                fail(f"{dot.path}: {device}'s {interface} is a lease, and names no network")
            found[interface] = (None, rest[0])
            continue
        address = parse_address(dot, address, f"{device}'s {interface} address")
        if rest:
            network = rest[0]
        else:
            network = next((n for n, v in nets.items() if "cidr" in v and address in v["cidr"]), None)
            if network is None:
                fail(f"{dot.path}: {device}'s {interface} {address} lies in no network's CIDR")
        if network not in nets:
            fail(f"{dot.path}: {device}'s {interface} is on {network!r}, which the networks do not list")
        net = nets[network]
        if "cidr" in net and address not in net["cidr"]:
            fail(f"{dot.path}: {device}'s {interface} {address} lies outside {network}'s {net['cidr']}")
        if "dhcp" in net and net["dhcp"][0] <= address <= net["dhcp"][1]:
            fail(f"{dot.path}: {device}'s {interface} {address} lies in {network}'s DHCP range")
        found[interface] = (address, network)
    return found


def binding(dot, node, port):
    """The interface and the port number a service's cell at `port` names."""
    text = dot.cell(node, port)
    m = re.fullmatch(r"\S+ (udp|tcp) (\S+):(\d+)", text)
    if not m:
        fail(f"{dot.path}: {node}:{port} is {text!r}, not `protocol transport interface:port`")
    return m.group(2), int(m.group(3))


def bound(dot, nets, node, port):
    """The address and port a service binds at `port`: its interface's fixed address, the
    wildcard 0.0.0.0 for `*`, or 127.0.0.1 for `lo`."""
    interface, number = binding(dot, node, port)
    if interface == "*":
        return ipaddress.IPv4Address("0.0.0.0"), number
    if interface == "lo":
        return ipaddress.IPv4Address("127.0.0.1"), number
    device = dot.device_of(node)
    held = addresses(dot, device, nets)
    if interface not in held:
        fail(f"{dot.path}: {node}:{port} binds {interface}, which {device} does not list")
    address, _ = held[interface]
    if address is None:
        fail(f"{dot.path}: {node}:{port} binds {interface}, which is a lease and has no fixed address")
    return address, number


def reach(dot, nets, tail, tail_port, head, head_port):
    """The address and port an edge from `tail` to `head:head_port` sends to: the head's
    listener, on the one network the two devices share."""
    if tail_port != "title":
        fail(f"{dot.path}: the edge from {tail} must run from its title")
    ours = {net for _, net in addresses(dot, dot.device_of(tail), nets).values()}
    theirs = {net for _, net in addresses(dot, dot.device_of(head), nets).values()}
    shared = sorted(ours & theirs)
    if len(shared) != 1:
        fail(f"{dot.path}: {dot.device_of(tail)} and {dot.device_of(head)} share {len(shared)} networks, not one, for {tail} -> {head}:{head_port}")
    return listener(dot, nets, head, head_port, shared[0])


def listener(dot, nets, node, port, network):
    """The address and port a service listens on at `port`, as reached over `network`."""
    interface, number = binding(dot, node, port)
    device = dot.device_of(node)
    held = addresses(dot, device, nets)
    if interface == "*":
        reachable = [a for a, n in held.values() if n == network and a is not None]
        if not reachable:
            fail(f"{dot.path}: {device} has no fixed address on {network} for {node}:{port}")
        return reachable[0], number
    if interface not in held:
        fail(f"{dot.path}: {node}:{port} listens on {interface}, which {device} does not list")
    address, on = held[interface]
    if address is None or on != network:
        fail(f"{dot.path}: {node}:{port} on {interface} is not a fixed address on {network}")
    return address, number
