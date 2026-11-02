# Tensegrity Open Claw — Design Document

**What this file is for.** Where code goes and why: the principles behind the system, its
basic structure, and a section per component — including components no longer in the code,
so this file is a knowledge base of parts a show can be built from. Which of them a rig has
changes from show to show; the rest of the file does not assume any. Fixtures are not
components and have no section here. It deliberately does *not* describe what the code
currently does — the code says that better — and it does not restate fixture addressing,
which lives in the QLC+ workspace and changes too often to mirror in prose.

[`README.md`](README.md) is the system's setup and operation, whatever the show: the Pi, the
cabinet, wiring, networking, build and deploy. [`SHOW.md`](SHOW.md) is one show: what the
audience sees and why, and the fixtures it uses and how each is set up.
[`OWNED-DEVICES.md`](OWNED-DEVICES.md) is the inventory of the physical devices we own — the
hardware these components are built on and around.

---

## 1. Principles

### 1.1 The QLC+ workspace is the patch

**None of the fixture addressing is written in Rust.** A rig's QLC+ workspace — the claw's is
`claw.qxw` — is the source of truth for *where* its fixtures sit, and the `.qxf` definitions
committed in `fixtures/` are the source of truth for *what their channels mean*. The build
ingests both and generates a struct per fixture, with a named field per channel, into that
rig's own crate.

**The universe and the sACN port come from the same file**, off the E1.31 output patched to
the workspace's first universe. That output describes the console's own transmission, but a
rig has one universe and relays a takeover onto it, so those two numbers are the rig's as
well: what the brain listens for, what it sends, and what a wireless fixture listens for. The
address that output unicasts to is the one the brain's receiver binds.

The workspace has two users: QLC+, which opens it to take the rig over, and the build. **The running brain never reads it**: the build consumes it, and it never
crosses into the running program, not even as a filename. A rig has exactly one, so naming the rig in
the log already says which file the binary was built from.

The decision being made here is to spend a build script in order to have **no second copy of
the patch**. The alternative — channel offsets written out in Rust — is a copy, and a copy of
something edited in another program by hand, at load-in, under time pressure. It would be
right until the first repatch and wrong silently thereafter, because nothing about a wrong
offset looks wrong until the rig is lit.

Generating named fields instead hands every direction of drift to the compiler, so there is
no drift checker because none is needed. What that catches, and how to read the build's
complaints when it does, is in [`README.md`](README.md) — it is part of the save-and-deploy
workflow rather than a thing to know in advance.

**Scenes are QLC+'s own.** The build reads the patch and not the workspace's functions, so a
scene is a look for a takeover and nothing more. While the rig had moving heads, scenes were
its pointing presets: where a head points is a measurement taken on the rig with a console,
so it lived in the workspace like the patch, and the build resolved each into a pose per head
in degrees. That ingest, and `cortex`'s moving-head code, are on the `last-version-with-mouth`
tag, the last show with heads.

**The same rule runs past addressing into values.** A `.qxf` says a great deal more than
which slot a channel occupies: which value opens the shutter, which end of a speed channel is
the fast one, which capabilities of a colour wheel are single filters and which are split
positions, how far the head pans. Every one of those is a fact about the fixture, and every
one of them written into Rust is the same second copy this section exists to refuse — worse
than an address, because a wrong address is at least wrong on every fixture equally, while a
wrong speed polarity is wrong on one model and right on the next.

So the ingest keeps growing to read more of the definition, and the code asks the patch
rather than spelling a number out. Where the definition does not carry the fact yet — a band
that still lumps several gobos together, a strobe channel with no capabilities at all —
the fix is to measure it and put it in the `.qxf`, not to add a constant next to the code
that wanted it. Landing a measurement makes the code that reads it work on every fixture
sharing the definition at once.

There is one class of exception, and it is worth naming so it does not get used as an excuse.
A `.qxf` describes what a fixture's channels *are* — ranges, bands, travel — and says nothing
about how it *behaves*: the slowest speed at which a head still moves smoothly is a fact about
that model, measured like any other, with no field in the format to hold it. Facts like that
live in `cortex`, beside the code that enforces them, because the alternative is a per-rig
copy of a number that describes neither rig. The test is whether the format could carry it: if
it could, the definition gets fixed and this section applies unchanged.

What stays written down is what is genuinely a choice: how fast we are willing to slew, how
long a breath lasts, how long a turn runs. Those live with whoever is choosing — the rig's own
crate for a rig's numbers, `cortex`'s `config.rs` for the cabinet's. What the fixture *is* is
not a choice, and belongs to neither.

### 1.2 External takeover: strict priority, whole universe

A console (QLC+ on the Mac) can take the universe live and hand it back, so a human can
drive the rig during a soundcheck or an emergency without stopping the daemon or touching
the Pi.

The rule is one line: **the internal engine is a source at priority 100, and an external
stream takes the universe only if its priority is strictly greater.** Strict is the whole
design. Every sACN source ships defaulted to 100, so any laptop that joins the network with
a live universe would otherwise seize the rig; taking over must be a deliberate act. Equal
priority is not a tie-break, it is a no-op.

Takeover is **whole-universe**. There is no HTP or LTP merge and no per-fixture claim:
whoever owns the universe owns all 512 slots. Merging is not a missing feature but a wrong
one — a fixture's channels are positions, wheel indices and modes, and `max()` of two wheel
indices is a position neither source asked for. Slots a short external frame does not carry
go to zero, not to the engine's last value; half a console's look is worse than none of it.

Both outputs are driven from a **single arbitration decision** taken once per frame, so the
wire and the network cannot disagree about who is driving. The relay goes out under the
brain's **own CID at its own priority**, on its own continuous sequence counter: only the
payload changes, so a takeover is invisible to anything downstream watching the stream's
identity.

The show keeps running while overridden, its timers and slews continuous, so handback
resumes mid-stride rather than springing out of a frozen frame.

The cabinet's e-stop comes **after** the arbitration: it acts on the frame as it goes out,
so it stops a console's frame as surely as the show's. A stop button that a laptop on the
network can override is not one. While the e-stop is pressed, or its two contacts agree (a
broken cable or a contact fault), the frame loop holds the last frame under a mask compiled
from the fixture definitions; what it overrides is in the README's E-stop section.

The mask is static and decided per channel at build time, from the channel's group, preset
and bands. Light goes to blackout by preset rather than by group, because hue, saturation and
subtractive CMY share the Intensity group and are not dark at zero. Shutters go to closed
where the definition has it and to open otherwise: most definitions mark no closed band, the
light is already out, and what matters is that the strobe stops. Every Effect and
Maintenance channel goes to its no-function band, settings included, because channel names
are free text and the channel guessed to be a harmless setting is the one that turns out to
be a program or a reset that fires when held. A forced channel whose definition marks no
such band fails the build: a guessed stop value is exactly the one that becomes a visible
event.

The receiver binds the address the workspace's console unicasts to — the Pi's WireGuard
address — alone rather than the wildcard. That is not
just narrowing: it is also what stops the brain's own multicast being delivered back to its
own receiver.

### 1.3 MQTT is telemetry, never show

Every topic on the broker reports; nothing the show does reads one. The show's inputs are
its sensors and the clock, and a future remote input arrives as state handed to the show
(§1.4), not as a subscription. That keeps the broker optional — a broker that is down,
slow or restarting costs the telemetry and never a frame — and keeps the show's behaviour
explainable from its inputs alone.

### 1.4 Inputs are state, handed in

A part that senses the world publishes a snapshot of what it senses, each on its own
latest-value seam, and the state machines take those snapshots as arguments each frame.
Neither machine reaches into a part for anything else, so every input each depends on is
visible in its signature. There is no common input type: one would only be unpacked again
into the snapshots it was made from, and the frame loop would have to learn the shape of every
sensor a rig has.

A snapshot that can miss an event between two frames counts instead of flagging: a button
press is a counter that moves, so a reader at frame rate sees it however short it was.

A snapshot carries the time it was taken, so a sensor that has gone quiet is a snapshot that
has gone stale, and what stale means is the show's decision, not the sensor's.

**A sensor on another machine would arrive over OSC.** None does today; this is the shape
reserved for one, so that adding it changes the transport and nothing after it. A receiver
thread would decode each message into that sensor's snapshot and the time it arrived, on the
same kind of seam an in-process sensor publishes on. UDP, because a late reading is worth nothing
and the newest one wins; OSC, because it is the smallest typed message there is, and anything
on the network — a laptop with `oscsend` — can stand in for the sensor. Not MQTT (§1.3): the
show reads no topic, and a broker in the path would make the broker part of the show. The
sender would send when it has a new reading and never repeat one, so silence goes stale like
any other sensor's. The receiver would bind the one address the sensors' network reaches,
like the sACN receiver, because a wildcard would also answer the 4G uplink's public address.

### 1.5 Time is resolved at build time

The show table names a city and lists local start/stop pairs; the build turns that into
coordinates, a zone, UTC instants with their offsets, and the civil dusk-to-dawn stretches
inside each show. The brain only ever sees a complete set of timestamps — no time-zone
database, no solar arithmetic, nothing that can disagree between the Mac and the Pi at run
time. It also makes the hard cases build errors rather than show-night surprises: a show
night can straddle the autumn clock change, and a local time in the hour that happens twice,
or in the hour that never happens, has no single meaning.

**The brain does not wait for NTP.** The Pi has no RTC and the 4G link can take a minute.
After a reboot the restored clock is behind, never ahead — by the reboot on a clean restart,
by the outage plus up to an hour on a power cut — and behind is the safe direction: the
startup listing keeps too much rather than dropping a show to come, and the per-frame
decision corrects itself when NTP steps the clock.

### 1.6 What may fail runs in its own process

Anything the show must survive losing, and that has no reason to be inside the brain, runs
outside it: Stem, and Memory on a Pi of its own (§3.9). A daemon that crashes, stalls or is restarted mid-tune then costs what it
provides and never a frame, and a process driving fixtures and motors beside people does not share an
address space with anything that can take it down.

The exceptions are parts with a reason to be inside. The face, and the ear and the mouth when
a rig had them, are ones the frame has to stay in step with to the millisecond: a process
boundary there is latency the audience sees — a cut that lands a hop after the light, a lamp
a hop behind the frame. The eyeball was one whose reading would otherwise have needed a
socket between two processes on the same Pi, or a second machine: inside, it was a thread and
a seam.

Those run inside the brain, and are written so a dead device costs what it provides: they
report, retry with backoff, and never block the frame loop; what cannot come back stops the
brain instead (§1.7). A part that does heavy work drops
out of the brain's real-time scheduling, so it slows itself and never a frame. Living and
dying with the brain, they hold no broker connection of their own and publish their subtrees
through the brain's (§3.1).

Each thread sets its own core and scheduling as its first act, from `cortex::sched`, rather
than the unit file setting one policy for the whole process. A thread spawned afterwards
inherits both, which is what puts embassy-time's timer thread beside the frame loop and an
inference runtime's worker pool on the background cores. A failure to set them is a deploy
error and stops the start.

| thread | cores | scheduling |
|---|---|---|
| the frame loop (the executor, with the face's tasks) and embassy-time's timer thread | 3 | FIFO 50 |
| `sacn-in`, telemetry, signals | 0–2 | `SCHED_OTHER` |
| Stem, in its own unit | 0–2 | `Nice=10` |

No core is isolated on the kernel's command line: with every other thread of the brain and
Stem pinned off core 3, what is left there is preempted by the frame loop anyway.

What inside costs is the one thing a process boundary bought: a crash in a part is a crash
of the brain, which systemd restarts in seconds of signal loss. That is a fair price for Rust,
whose failures are panics the brain is built to end on. It is a real one for native code a
part links, such as a C++ inference runtime, and it is why a pure-Rust alternative is worth
its cost in speed when there is one.

### 1.7 Recoverable faults are soft, unrecoverable faults are hard and fast

Recoverable faults may be soft-handled; unrecoverable faults fail hard and fast.

A fault is recoverable when it can go away by itself: a device unplugged and plugged back
in, a network link that drops and returns, another machine that is down and comes back up.
Those are reported, retried with backoff, and cost only what they provide while they last.
The show runs on through them.

Every other fault cannot recover. A missing boot overlay, a serial port that will not take a
write, a device that opens but refuses the format the build was made for, a port another
process holds, a thread that cannot be pinned, a library the deploy should have installed:
retrying any of them only repeats the failure, and logging it and carrying on hides a broken
rig behind one that seems to run. They panic at the point of failure, preferably at startup,
so the journal names the cause and systemd's restart loop makes it impossible to miss at
the bench, long before a show. The workspace builds with `panic = "abort"`, so a panic in any
thread ends the process; unwinding would end only that thread, and the brain would run on
without it, looking healthy.

The test is not how much the thing matters to the show, but whether waiting could ever fix
it.

### 1.8 Everything at its latest, at the start of every show

The first step of every new show is upgrading everything to its newest release and making it
work again: the Rust toolchain and every crate, the Python packages, GreptimeDB, Grafana,
QLC+, Raspberry Pi OS on every Pi, and the firmware of every device whose maker still ships
one. Pre-releases count where a dependency has only those.
An older version stays only for a reason written beside its pin.

Upgrading at the start is when it is cheap: there is a whole build ahead to find what broke,
where upgrading the week of a show is a risk, and never upgrading is a pile of breaking changes
that arrives all at once, the day something forces it.

### 1.9 `no_std` and no allocator, wherever it can be

Anything shared between the Pi and a microcontroller is `no_std` — not when a firmware
happens to need it, always. `stem` set the pattern; `dendrite-wire` and `innerear` follow it. The
reason is not the firmware of today but the brain of tomorrow: the frame loop, the estimator
and the show may one day run on an ESP, and code that was `std` only out of habit is the
code that stops that. A crate reaches for `std` only for what the Pi alone has — sockets,
threads, the serial port, the filesystem — and keeps that in the thinnest shell it can.

The same goes for the heap. A microcontroller may have no allocator at all, so portable code
works in fixed-size buffers its caller owns: `stem` writes each packet into one and returns
its length, and the sACN encoder and source table are arrays sized by the protocol and the
configuration. An allocation in the frame loop is a porting job waiting to happen.

---

## 2. Structure

### 2.1 System overview

The rig is a **Raspberry Pi driving one DMX-512 universe of fixtures**. The Pi runs a Rust
program on Linux (`brain`) that reads the rig's sensors, runs the show over them, and clocks the resulting universe out of an RS-485 HAT at 40 Hz to the wired fixtures. The
same universe also goes out as E1.31 sACN over the network — wireless fixtures listen to it,
it is what a monitor or a console works against, and it is the path an external override
arrives on.

The Pi acts as a WiFi access point, and is reachable from the programming Mac over a
WireGuard tunnel — which is how both deployment and console traffic get there.

Telemetry — the broker, and the recordings of everything published on it — lives on a second
Pi on that access point, Memory (§3.9), so the Pi that drives the show carries none of it.

```
    ┌── Mac ──────────────────────────────────────────┐
    │  (SSH / deploy; QLC+ for override)              │
    └───────────────── WireGuard ─────────────────────┘
                                  │
    ┌── Raspberry Pi ──────────────┴──────────────────┐
    │  (builds brain; reads the rig's sensors)        │
    │  ◄── GPIO                                       │
    │  The show                                       │
    │  ├── RS-485 HAT (40 Hz) ──── wired DMX-512      │
    │  └── sACN sender ──────────── network           │
    └─────────────────────────────────────────────────┘
                                  │
                        one universe of fixtures
                   (the rig's .qxw holds the patch)
```

### 2.2 One universe, two transports

The daemon emits exactly one universe, and the ingest rejects any fixture patched outside the
workspace's first universe rather than silently dropping it. Every rig so far fits in 512
slots; a second universe would need a routing layer between the engine and the wire.

**Wired DMX-512 out of the RS-485 HAT drives the wired fixtures.** sACN (E1.31) carries the
same universe over the network in parallel: it drives the wireless fixtures, it costs nothing, it lets a
monitor or a console see what the daemon is doing, and it is the path an external override
arrives on.

Art-Net is not supported: we control every device on the network, so compatibility with older
rigs is not a requirement.

**The wired frame is padded to a full 512-slot universe.** A short frame is a known way to
lose cheap DMX receivers: they ignore the data and fall back to their own internal show
while still reporting a healthy signal, which reads as a wiring fault and is not one. The
padding costs a fixed 512 slots per frame, which is what sets the ~43 Hz ceiling the 40 Hz
frame rate sits under.

### 2.3 The crates

| | |
|---|---|
| `cortex/` | what every rig shares under its show: both transports, the frame loop, the show calendar, the latest-value seam |
| `cortex-build/` | the QLC+ ingest and the calendar's resolution, a build-dependency of the rig crate |
| `brain/` | the rig crate: the rig's patch, its show times, and its show |
| `stem/` | Stem as the Rust programs see it (§3.1): the health contract and the MQTT packets, purely `no_std` and allocation-free for the firmware. The brain's client, which needs std, is `cortex::telemetry`. It shares its directory with Stem's daemon and broker configuration |
| `innerear/` | the inner ear, the sway estimator (§3.12): a phase-locked loop over acceleration samples and a free-decay estimator for f₁ and Q, `no_std`, a workspace member the rig crate wires in |
| `dendrite-wire/` | Dendrite's wire contract (§3.11): the GATT service and its packed samples, and the OSC messages the relay emits; `no_std`; a workspace member (the brain decodes the OSC) shared by path with the firmware like `stem` |
| `wrist/` | Wrist's firmware (§3.10): ESP32-S3, not a workspace member, built on the Mac for its target |
| `dendrite/` | Dendrite's firmware (§3.11): nRF52833, not a workspace member, built on the Mac for its target |

**Beside the crates**, what the Pis run that is not Rust: `stem/` also holds Stem's daemon,
its broker configuration and the Python modules every health daemon shares; `etc/` is what
`claw-pi`'s deploy installs that belongs to no crate — its network profiles, the rig's WiFi
in `claw-wifi.env`, the 4G watchdog, the boot configuration; `etc-memory-pi/` is everything
`memory-pi` runs (§3.9). A daemon's systemd unit lives in that daemon's directory
(`brain/brain.service`, `stem/stem.service`). Each Pi has its own deploy script, `deploy.sh`
and `deploy-memory-pi.sh`, sharing `etc/deploy-lib.sh`, and either deploys while the other is
down.

The rig crate's `build.rs` names its workspace and reads `shows.rs`, the show table; its
`main.rs` includes the generated patch and calendar, wires in the components it uses, and
writes the show.

**`cortex` holds only what every rig shares.** A component a rig may or may not have — a
sound output, a sensor — is its own crate with its own configuration, and the rig crate that
uses it depends on it and wires it in. `cortex::run` and the `Rig` it takes then never grow a
field for a part one rig has.

Firmware for a wireless fixture is not a member of this workspace: it is built on the Mac for
its own target, reads its universe, port and address out of the workspace through
`cortex-build`'s `addressing()`, and shares the `stem` crate with the brain by path, which is
why that crate stays `no_std`. Everything in the workspace is built on the Pi.

Wrist's and Dendrite's firmwares restore what the ball had — the sACN front end, WiFi
provisioning from `etc/claw-wifi.env`, the MQTT client, the deploy integration — from the
`last-version-with-ball` tag, ported from the C3's RISC-V to the S3's Xtensa where Wrist needs
it. `deploy.sh` builds both on the Mac; `remote-deploy.sh` flashes Wrist over its USB-C on the
hub, as it flashed the balls; Dendrite is flashed from the Mac over its own debug interface
when it is plugged in, and the deploy says so when it is not.

**Assets are compiled in.** A recording or a model a component needs goes into the binary at
build time, decoded by the component's own build crate or with `include_bytes!`, so the binary
on the Pi is the whole show: no file to be missing at startup, no decoder in the running
program, and an asset the build cannot read is a build error naming it. The mouth's
recordings and the eyeball's model were built this way (§3.6, §3.8).

### 2.4 Software stack

Standard Rust on Trixie Raspbian Linux, `std`.

- **Hand-rolled sACN E1.31 encoder** writing to a UDP socket. The `sacn` and
  `sacn-unofficial` crates were not a good fit; the packet is small enough that owning it
  costs less than working around them.
- **`zihatec-rs-485-dmx`** for DMX-512 framing — BREAK/MAB timing and full-512 padding. Only
  the device path is ours.
- **Embassy** (`executor-thread` + `embassy-time`) for the frame loop, with blocking
  producers — sensors, the sACN receiver — on their own OS threads feeding it, or fed
  by it, through a lock-free latest-value seam. A blocking `recv` on the executor's thread
  would stall the frame loop, so anything that blocks stays off it, and anything that
  computes for long runs below it in scheduling (§1.6).

### 2.5 The rig state machine

The brain runs **two state machines, layered**, and keeps their states apart by name:

- The **rig state** — boot hold, running, e-stop, shutdown — says whether, and what, goes
  out. Its machine is in `cortex`'s frame loop, after the arbitration, and it is published on
  `brain/rig`.
- The **show state** says what the show asks for. Its machine is the rig crate's Director,
  and [`SHOW.md`](SHOW.md) describes it.

The rig state gates the show's output; the show state machine never sees the rig state, so
it has no e-stop, boot or shutdown cases, and runs on unchanged underneath a stop.

The rig state machine runs boot hold → running ⇄ e-stop → shutdown. Boot hold is where it
starts, before anything has gone out, and it leaves on the first frame: to running if the
e-stop is released, or to an e-stop holding nothing, which stays silent until release. The
e-stop is read once when its pins are claimed, so the first frame already knows it. Until a frame has gone out the brain does
not know what the fixtures are doing, so silence, which their signal-loss behaviour answers
by holding, is the only honest stop frame. Shutdown releases the sACN source on the frame
loop's own send path, so there is only ever one sender. Neither boot hold nor shutdown can be
returned to, and neither needs a special case anywhere else.

### 2.6 The Pi controller

The deployed unit is a **Raspberry Pi 3B+** running standard 64-bit Debian / Raspberry Pi OS. A Pi 4
has more headroom and runs the same software unchanged.

The Pi runs in **AP mode** (NetworkManager) so fixtures and laptops join its own network
rather than depending on whatever WiFi exists at the site, uses its **Ethernet port** as a
wired segment for a laptop, and runs `brain` as a systemd service for unattended operation.

**Everything that must be found by address has a fixed one, below `.100`.** The access point
and the wired segment each give the Pi a fixed address, `.1`, and hand out addresses by DHCP
from `.100`–`.199`, as a convenience for laptops. A device the rig talks to by address sets
one below `.100` on itself: a second Pi, a wireless fixture. Both networks route their
clients out over 4G.

Settings — SSID, serial console, DIP switches, `config.txt` — are in
[`README.md`](README.md).

---

## 3. Components

**The rig is a fixed core and the components a show chooses.** The core stays in the code
whatever the show, used or not. A component is in the code only while a show uses it: when a
show is done without one, the component is tagged `last-version-with-<component>` and
removed, with everything that existed only to serve it, and its section here stays as what
was learned building it. A later show that wants it back restores it from its tag. Fixture
definitions in `fixtures/` are neither: they are a library, kept whether or not a workspace
patches them.

| The core | Where |
|---|---|
| the frame loop and the rig state machine (§2.5) | `cortex`'s orchestrator |
| wired DMX out through the RS-485 HAT | `cortex::dmx` |
| sACN out | `cortex::sacn_out` |
| sACN in: takeover by priority (§1.2) | `cortex::sacn_in` |
| the e-stop | `cortex::estop`, read by the face |
| the face (§3.3) | `brain/src/face.rs`, `fixtures/face.qxf` |
| the Director, the show's state machine, with test mode | the rig crate's `show.rs` |
| the calendar, with each show's civil dusk and dawn (§1.5) | `cortex::calendar`, `cortex-build`, the rig crate's `shows.rs` |
| the QLC+ ingest (§1.1) | `cortex-build`, `cortex::qlc_plus` |
| the latest-value seam and thread scheduling (§1.4, §1.6) | `cortex::latest`, `cortex::sched` |
| the telemetry client and the health contract | `cortex::telemetry`, the `stem` crate |
| Stem (§3.1) | `stem/` |
| Memory (§3.9) | `etc-memory-pi/` |
| the 4G watchdog (§3.2), the access point, the wired segment, WireGuard | `etc/` |

| Component | In the code |
|---|---|
| the ear (§3.4) | no: `last-version-with-ear` |
| the ball (§3.5) | no: `last-version-with-ball` |
| the eyeball (§3.6) | no: `last-version-with-eyeball-for-pose-estimation`, `last-version-with-eyeball-for-group-detection` |
| the retina (§3.6.1) | no: `last-version-with-retina` |
| the ponytail (§3.7) | no: `last-version-with-ponytails` |
| the mouth (§3.8) | no: `last-version-with-mouth` |
| moving heads: their aiming and the QLC+ pointing presets (§1.1) | no: `last-version-with-mouth` |
| Wrist (§3.10) | coming with the Headless Hug PoC |
| Dendrite (§3.11) | coming with the Headless Hug PoC |
| the inner ear (§3.12) | coming with the Headless Hug PoC |

### 3.1 Stem

The backbone for telemetry: the MQTT broker, Mosquitto, and a health daemon in Python. All of
the rig's telemetry is read-only and goes through it (§1.3). It is separate from `brain`
(§1.6) so it stays alive and reporting when `brain` is down — which is precisely when health
matters — and, being Python and a broker rather than Rust, it has no place inside the brain
anyway.

**The broker belongs to Stem, and runs on `memory-pi`** (§3.9), beside the recordings that
read it, so the Pi that drives the show carries none of the telemetry. The transport the
reports travel on is part of Stem's job rather than a service the brain happens to offer. It
is Debian's Mosquitto, configured by an overlay in its include directory (`stem/stem.conf`),
so a package upgrade cannot quietly revert the rig's settings. It keeps nothing on disk, and
every client replays its retained topics on each connect instead. Stem's daemon stays on
`claw-pi`, the hardware it reads, and reaches the broker across the access point like every
other client there.

**The daemon is deliberately the least capable process on the rig**: it reads the hardware and
how it is holding up, publishes it, and does nothing else. Keeping duties out of it means its
death costs telemetry and never uptime.

Every process holds its own connection to the broker, announces itself on `health/<service>`,
and leaves a will that says `gone`: `brain`, Stem's daemon, and Memory's health daemon and
recorder. A part inside the brain, like
the face, lives and dies with it, so a will of its own would only repeat
`health/brain`; it publishes its own subtree through the brain's connection (§1.6).

**The Rust side of Stem is the `stem` crate**, in Stem's own directory: how every Rust
program sees Stem, the process it reports to. It carries the health contract and the packets,
purely `no_std` so the firmware can use them; each program brings its own transport, the
brain's being `cortex::telemetry`. `stem/rig_mqtt.py` is the Python half, and
`stem/rig_board.py` the board readers every Python health daemon shares.

### 3.2 The 4G watchdog

A shell script on a systemd timer that keeps the 4G uplink, and the WireGuard tunnel over it,
alive.

**It finds the modem itself, every run**, because the cabinet is rewired for every show: the
modem on the USB bus by its vendor, then its network interface and its control port under
that USB device in sysfs, then the ModemManager modem whose physical device it is. Nothing in
the script names a port, a hub or an interface.

**It touches the modem alone**, so the show is never disturbed. There is no reboot and no
power cut. A reboot breaks the show for a link the show does not need. A per-port power cut
can only be trusted on a hub that really switches its ports individually, which a hub's own
claim does not prove, and on a ganged hub it would also cut whatever shares the hub.

### 3.3 The face

The cabinet door — e-stop, two buttons, a green lamp in each — is wired to the Pi's GPIO, but
it is treated like any fixture in both directions rather than as a side channel.

**Its lamps are patched.** `fixtures/face.qxf` gives them two dimmable channels, and the
workspace patches them like any fixture. The show writes them into the frame; the face drives
the pins from the frame that actually went out, after arbitration. So a console that takes the
rig over drives the lamps too, and the e-stop darkens them with everything else, with
no special case in either. They dim through the Pi's two hardware PWM channels, because the
software alternative is a busy-waiting real-time thread per pin, competing with the DMX
timing.

**Its inputs are a `FaceState`.** The face polls and debounces, and publishes the e-stop and
counts of each gesture — chords, presses — once per poll, on its own latest-value seam.
Counts rather than flags, so a show that reads at frame rate misses none (§1.4). The show
state machine reads only the buttons; the e-stop goes to the rig state machine (§2.5), read
once when the pins are claimed so the first frame already knows it.

**It cannot fall silent on its own.** The face polls as a task on the frame loop's own
executor, so it stops only when the frame loop does: a panic in either ends the process. That
is why there is no rule for a face that has stopped reporting, and why it has no broker
connection of its own — its `face/…` topics go out through the brain's (§3.1).

### 3.4 The ear

The rig's microphone input: capture, feature extraction, and the `AudioFeatures` it handed
the show — onsets, and how much of what it heard was music — which drove the washes'
sparkle and the heads' dance. It drove the claw at ADE, and was removed after it; the code is
on the `last-version-with-ear` tag (`cortex/src/ear/`, and `cortex/src/sparkle.rs` for
the mapping).

It had no DMX channels and no place in the workspace, but it was a part of the rig the way
the face is — its own thread, its own state handed to the show, and its own topics on the
broker, so a dead or missing interface showed up as `ear/…` rather than as a show that had
gone quiet for no visible reason.

**Fast and lean.** Every feature was computed on every block, so the snapshot carried only
what a show read, plus the diagnostics it reported; a feature nobody read was work done for
nothing on a 1.2 GHz Pi. Its capture thread — open, negotiated parameters, xrun recovery in
place, reopen with backoff, and a USB probe published on every reopen — is the mouth's
(§3.8) turned round.

The interface itself is incidental — an Alesis io|2, because it was the cheapest used one to
be had — so the code named it by role. Swapping it was its ALSA card name and USB identity in
the config, nothing more.

### 3.5 The ball

An ESP32-C3 on the Pi's WiFi, following the sACN universe. The firmware is `no_std` Rust on
Embassy. It was removed from the code after the ADE show; the last version, with the deploy's
firmware build and board flashing, is on the `last-version-with-ball` tag (`ball/`,
`ball-dmx/`, `attach.sh`). Its definition, `fixtures/ball.qxf`, stays in the fixture library.

**A ball is a fixture in the patch like any other**, and its firmware is built from the
workspace too. A ball is whatever the workspace patches through `fixtures/ball.qxf`: the
firmware build turns every one into a constant carrying its address, and reads the channel
layout and bands out of the definition; a board is provisioned by naming the one it is.

**The ball draws its own waveforms.** At 40 Hz a frame is 25 ms, longer than the pulse's
hold, and multicast over WiFi arrives bunched. So the brain sends a pattern and its
parameters, and the ball renders them once per period of its 500 Hz PWM. The firmware's
strobe takes its flash and gap lengths in a unit a `.qxf` cannot express, so that unit lives
in `ball-dmx`; the show does not use the strobe. Everything the `.qxf` can express — which
channel sits where, and the bands of the motor and the pattern — the firmware's build reads
out of the definition, as §1.1 asks.

**The ball sleeps outside shows.** Its radio, receiving all the time, is most of its draw, and
outside a show it has nothing to do. The brain asks it to sleep over DMX, like everything else
it asks of the ball, and the ball deep-sleeps in five-minute stretches, waking each time to
report and to ask again. Deep sleep is a reset on waking, so every wake is a boot.

Every boot, a wake from sleep or a power-on alike, runs the same way:

- The outputs start held off and the motor tripped, as every boot already does.
- sACN is processed as normal from the moment the network is up: the ball follows the
  universe straight away, whatever comes next.
- Alongside it, telemetry goes out as always, the pack voltage among it. The pack is read at
  boot, without waiting for anything, which is how it is read on every boot. Publishing never
  holds up the sACN processing.
- Five seconds after the network is up the ball decides. Timed from the lease rather than
  from boot, because joining and the lease take seconds of their own, and five seconds of
  listening is what has to pass. That is plenty for a broker that is there to take the
  telemetry, and waiting longer for one that is not would not make it come: the operator
  reads the ball's liveness off MQTT — `health/ball` going `connected` every five minutes and
  a fresh `ball/pack_v` — and a wake that did not reach it shows as exactly that.
- If the universe asks it to sleep, or if no universe has been heard at all in those five
  seconds, it goes to sleep. Otherwise it keeps running, as a ball does today.
- The access point is on the Pi, so a rig switched off has none to join, and the five
  seconds would never start. A join that has not succeeded after four retries and a margin,
  about a minute, is taken as that, and the ball sleeps.

Going to sleep is four steps, in order: stop the motor, whatever it was doing, and hold its
pin low, because in deep sleep nothing drives it; say `disconnected` and wait for the goodbye
to leave, the one place anything waits on telemetry; disconnect from the access point cleanly,
so it holds no stale association for the ball's next wake to wait out; then deep-sleep for
five minutes. A wake counter in RTC memory, which a deep sleep keeps, is published as
`ball/wakes`, so a reset that was not a wake shows as the count starting over.

No universe after a wake means the rig is off, and a ball that stayed awake for it would drain
its pack exactly when nobody is there. It counts only at that one decision: an awake ball that
loses the universe mid-show, to a brain restart say, darkens and waits as it does today. Once
awake, only the channel puts the ball to sleep.

The brain asks from the end of one show until ten minutes before the next, which is one
stretch of sleep, a wake, and a margin, so the ball is up when the show starts; never in test
mode. The Sleep channel's awake band is `NoFunction`, so the e-stop's mask wakes the balls with
no rule of its own.

### 3.6 The eyeball

One small reading of the scene out to the show, from the frames of a camera (§3.6.1), and an
annotated feed for humans. It had two lives. First a Python process beside the brain, reading
a person's arms for the mage rig; that code is on the
`last-version-with-eyeball-for-pose-estimation` tag (`eyeball/` and `cortex/src/eyeball.rs`).
Then a Rust crate running as threads inside the brain, handing the Halloween show how far
into the yard the furthest person had come; it was removed after that show, and is on the
`last-version-with-eyeball-for-group-detection` tag (`eyeball/`, and the brain's
`presence.rs`, `draw.rs` and `feed.rs`).

**The retina sees, the eyeball looks.** The retina is the camera; the eyeball is what is in
the frame: the detector and the distance. Between them is a one-slot mailbox of JPEG frames,
taken rather than read, so no frame is detected twice. The eyeball says what it needs in a
`View` — a resolution from those the camera offers, a frame rate within what it can deliver,
infrared or colour, and which way up the camera hangs — and the retina sets the camera up for
it, so a consumer with other needs takes no change in the retina. The eyeball starts its own
retina with its view, so whoever runs the eyeball only starts it. A different camera is a
different retina and the same eyeball. Each reports its own half on the broker, `retina/…`
and `eyeball/…`, so a dead camera and a slow detector never look alike.

**The rig draws, the eyeball sees.** The eyeball hands out one sighting per frame: the
distance and when it was seen, with the picture and the boxes it came from. The show reads
the distance; the annotated feed, which is the rig's and lives in the brain, draws the rest.
What the feed draws — the two columns, a dot on the leading foot — is the show's space, and an
HTTP server is the rig's choice of how an operator looks, so neither belongs in the crate that
finds people.

**Inside the brain (§1.6).** One machine and no hop: the reading crosses a latest-value seam
to the show, the same kind of seam an OSC receiver would publish on if a sensor ever moved to
another machine (§1.4). The price is the one §1.6 names: a crash in the inference runtime is
a crash of the brain. And the Pi's cores are shared with the DMX loop, so the detector runs
below the brain's real-time threads, on at most three of the four, and a slow model costs the
eyeball frames and never the rig one.

**Three outputs, three audiences.** The reading goes to the show, once per processed frame
and never repeated, with the time it was seen, so a stalled camera or a wedged detector is a
snapshot going stale, which the show's staleness window turns into the safe default. The
annotated feed goes to a browser over HTTP, because that is what a human uses to aim a
camera; it binds the WireGuard address alone, like the sACN receiver, because a wildcard
would show the space, and whoever is in it, to the 4G uplink's public address. Telemetry —
the reading itself among it — goes to MQTT through the brain's connection.

**The show reads the least it needs.** The eyeball reduces what the model sees to the one
quantity the show uses and keeps the rest on its own side, where the feed draws it. In the
first life that was arm angles, not the seventeen joints of one pose model; in the second it
is one distance, not boxes or a count. The estimator is expected to be replaced, and a schema
naming its internals would have to change with it.

**The feed costs nothing unwatched.** Drawing on a frame is cheap; compressing it to JPEG
again is not, on a Pi already running a detector. So the feed is encoded on its own thread,
from the latest sighting, and only while a browser is connected.

#### 3.6.1 The retina

A network camera, an Axis M3105-LVE, set up over VAPIX and read as MJPEG, its newest frame
handed to the eyeball: setting the camera up, reading its stream, reconnecting when it goes.
A component of its own beside the eyeball, removed with it after the Halloween show; it is on
the `last-version-with-retina` tag (`retina/`, and the camera's DHCP reservation in
`etc/claw-eth0.dnsmasq.conf`).

**The bounds of a view are types,** checked where the view is written. What belongs to the
camera rather than to whoever looks, the mains' 50 Hz and the exposure for people walking,
stays the retina's own.

**The camera is configured by the retina, not by hand.** Which stream, at what size and rate,
the image rotation, and the sensor's settings are the retina's to decide, beside the camera's
host and login. It sets the camera's parameters over VAPIX on every connect and reads them
back, because a setting made by hand in the camera's web UI is lost on a factory reset or a
replacement camera; a setting the camera refuses, or a login it rejects, will not go away by
retrying, and stops the brain (§1.7). The login is the camera's factory default, kept as it
is: a camera reset or replaced takes it with nothing done by hand, and the camera boots in its
own time, before the brain or after it, with the same login waiting whenever it answers. The
camera is on the Pi's private wired segment, so a default password costs nothing. Infrared beats visible light by a wide margin, so it runs the IR-cut
filter off under the camera's own illuminator, with exposure capped at 10 ms so a moving limb
is a shape and not a streak. MJPEG over HTTP, because it has no inter-frame state or jitter
buffer and needs only a JPEG decoder, where H.264 would need a video decoder in the build.

**The newest frame, always.** The detector sets the pace, not the camera. A reader that kept
every frame would queue them behind a slow model, and the reading would lag the world by the
queue; so the reader keeps only the newest and drops what has piled up, and a slow model
shows as a lower frame rate on the broker rather than as a show that reacts late.

**The camera's address is a reservation, not a setting.** On the Pi's wired segment, in the
private dnsmasq NetworkManager runs for every shared connection, keyed on the camera's MAC, so
a camera that is factory reset or replaced comes up where the retina looks for it with
nothing set on it by hand.

#### In the brain: one distance

A person detector, not a pose model: boxes are enough, and a person is the bottom corner of
their box on the near side of the space — the leading foot, which a box merged over a bunch
still puts on the one in front. A YOLOX-Nano model (Apache-2.0) runs on the CPU through `tract`,
pure Rust; the model is compiled into the binary (§2.3), with its licence and notice beside it.

**The model's input is the camera's shape, not the square it was published at.** Its work
scales with the pixels it looks at, so Megvii's weights are exported at the camera's aspect
ratio and well under the published 416×416: several times the frame rate on the Pi, and no
work spent on padding. The cost is the smallest people, under about 25 pixels tall in the
model's picture. The export is a script beside the model, so another shape is one run of it.

**The models not taken.** YOLOX-Tiny is the step up, too slow on a Pi 3B+'s cores beside the
frame loop and the same code on a Pi 4. YOLOv8 and YOLO11 are AGPL. A detector trained on
crowds would separate a bunch of children better, but has no clean first-party export, and the
show reads only the front of a group. ONNX Runtime runs the model four times faster than
`tract` on a Mac, but its crate downloads a prebuilt library; only Debian's package would do.

**No calibration, only aiming.** The space has two benchmarks, and they are two columns
fixed in the camera's picture: the camera, square on to the space, is aimed so the benchmarks
in the space land on the columns, watching the feed, which draws them. A distance is where a
person's feet fall between the two columns, linear in the picture, so in the space only as
linear as the camera is square — which the feed shows at a glance, and which nothing in a
show that compares distances needs to be exact. The alternative, a ground-plane homography
from four measured marks, is right at any angle and is four points to measure and a matrix to
get wrong.

**The unit is the space, not the metre.** The distance is 1 at one benchmark and 0 at the
other, so nothing is ever measured with a tape, and an empty space is `+∞`: infinitely far,
so every comparison a show makes falls the safe way with no magic number. Debouncing and
hysteresis are the show's policy, not the eyeball's: it reports what it saw in each frame,
and tracks nobody.

#### Beside the brain: arm angles

It ran as its own Python process on the Pi, on OpenCV and numpy, with a MoveNet pose model on
TFLite or ONNX, and sent the brain its own datagrams over UDP on the loopback. The model got
three of the Pi's cores, leaving one for the DMX loop. The preview was shown to the audience,
so the drawn skeleton was a deliverable, not a debug view.

**A person is several landmarks.** The model now and then puts a confident landmark on a
bush, so a person was reported only once five landmarks cleared the confidence threshold.

### 3.7 The ponytail

A cheap fibre-optic light fixture — plastic fibres lit by an LED array, with a patterned
plate turned by a stepper in front of them — retrofitted with a Seeed Studio XIAO ESP32-S3
that joined the Pi's WiFi and followed sACN. It was removed from the code; the last version is
on the `last-version-with-ponytails` tag.

Two personalities shared one sACN front end. **PWM** drove the RGBW LED array directly.
**BLE bridge** kept the fixture's original Telink controller and translated DMX into the BLE
writes its app sends — the only path that reaches the gobo motor. That fixture is modal: RGB
and white cannot light together, so White above zero overrode RGB with a hard cut, the dimmer
was applied in software so it worked the same in both modes, and with no readback the bridge
re-asserted the whole state on every change, on a heartbeat and on every reconnect.

Its sACN decoder did source arbitration in the fixture itself, so a console could take it over
even with `brain` hung.

### 3.8 The mouth

The rig's sound: recordings played through a USB audio interface into two powered speakers, a
channel each. Its own crates, `mouth/` and `mouth-build/`, which a rig crate depended on when
its show had sound; `cortex` knew nothing of it (§2.3). It voiced the Halloween show and was
removed after it; the code is on the `last-version-with-mouth` tag (`mouth/`, `mouth-build/`,
and the brain's `sounds/`, `sounds.rs` and `mouth_show.rs`). It was the one part that linked
ALSA, and so the reason the brain could be built only on Linux.

**A unit, so a show without sound leaves it out.** On the rig crate's side the mouth is its
recordings in `sounds/`, the sound table that names them and their gains, and one show file of
its own, through which the show cues its sounds and reads their envelopes; the rest of the rig
crate touches it in the few lines that spawn it and include its generated `sounds` module, and
in the show's calls into that file. Taking it out is deleting those and the two crates.

**The recordings are compiled from the files as they were found** (§2.3). The build decodes
each, resamples it to the interface's native rate so nothing resamples at run time, keeps it
in stereo, trims what is 60 dB or more below its peak off both ends so its first sample is
the sound, brings its peak to just under full scale, and then takes it down by its gain in
the table, so the samples are stored at the level they play at and that gain is the only
thing that sets it. A gain above 0 dB would clip, and fails the build. Overall level is the speakers' own knobs; there is no master
gain in software to disagree with them.

**Every recording carries its envelope**: its loudness, one byte per DMX frame, at the frame
rate the rig runs at, which `cortex` handed the rig crate's build script through cargo's
`links` metadata, so a change to the rate rebuilt every envelope. The samples are padded with silence to a whole number of frames, so the
envelope's length is the recording's length in frames, and a look that follows a recording
plays the envelope by frame index from the frame that cued it. Sound and light then share one
clock, the frame count, with the sound a buffer behind the frame that asked for it.

**A thread in the brain, not a process.** A show that cuts light and sound together needs
them together: a sound a process hop late is a cut the audience hears arrive after it sees
it. So the mouth is a playback thread inside the brain (§1.6), on blocking ALSA writes, and a
dead or missing interface costs the sound and never a frame: it reopens with backoff and
reports why it is mute. An interface that opens but will not play the recordings as they were
built, at their rate and format, stops the brain instead (§1.7).

**Small buffers, because the cut is the point.** A period of 256 frames and two of them in the
buffer put the next sound about 11 ms behind the frame that asked for it. A mute zeroes the
next buffer and kills every voice, so silence lands within a period of the frame.

**The show hands it a cue every frame, over a latest-value seam**: a trigger counter per
sound, and mute as a level. The mouth reads the latest cue every period, several times a
frame, and a counter that has moved is the only kind of trigger that reads exactly once (§1.4).
It mixes its voices into each period by adding them up, straight out of the binary, and clips
what sums past full scale. Voices overlap: a sound cued while it or another is still playing starts a
voice of its own, from a fixed pool so playing never allocates, and when the pool is full the
voice furthest into its recording gives way.

**The e-stop silences it; a takeover does not.** The e-stop is a show-stop, so it stops the
sound with the light: the rig crate mutes the cue while the face reports the e-stop, at boot
as well. A console takes over the universe, and sound is not part of the universe, so the
show's sound runs on under it.

**It reports through the brain's connection**, living and dying with the brain (§3.1): what
the stream was actually opened at, which is not always what was asked for, xruns, why it is
mute, and the interface as the USB bus sees it — identity, bus address, speed and port path,
republished on every reopen, so a re-enumeration under a running brain shows as the flaky hub
port it usually is.

The interface itself is incidental — an Alesis io|2, the one the ear used — and swapping it is
its ALSA card name and USB identity in the mouth's config.

### 3.9 Memory

The rig's long memory: everything published on the broker, kept in a time-series database on
an external SSD, so a show can be read back afterwards — what the sensors saw, what the show
made of it, how hot a Pi ran, when the uplink dropped — and the broker itself. It runs on a Pi
of its own, `memory-pi`, configured entirely from `etc-memory-pi/` and deployed by
`deploy-memory-pi.sh`.

**A Pi of its own, because `claw-pi` has no room for it.** `claw-pi` is a 3B+ with 905 MB,
swap on its SD card, and the frame loop; a database that flushes
and compacts in bursts has no place there. `memory-pi` is a Pi 3B, enough because Greptime
publishes 64-bit ARM binaries and runs in a few hundred MB.

**Never part of the show.** The show reads no topic (§1.3), so Memory's death, its SSD's or
its Pi's costs the telemetry and the history and never a frame. Everything on it reconnects
with backoff for as long as it takes, because both Pis power up together and `memory-pi` is
up long before the access point.

**On the access point, at an address it sets itself.** `memory-pi`'s only network is
`claw-pi`'s access point, at a fixed `10.0.10.2` from its own NetworkManager profile, built
from the same `claw-wifi.env` as the access point so the two cannot disagree. `claw-pi`'s
shared mode routes it out over 4G for its clock, apt and downloads. The Mac reaches it only by
jumping through `claw-pi`: a tunnel of its own would buy nothing, because a `claw-pi` that has
died leaves little to do remotely anyway.

**Five processes:**

- **Mosquitto**, Stem's broker (§3.1). Every client on the rig connects to `10.0.10.2`.
- **The health daemon**, `memory.py`: Stem's counterpart for `memory-pi`, publishing
  `health/memory`, `sys/memory` and `memory/…`. It shares Stem's board readers through
  `stem/rig_board.py`, so a fix to how a reading is taken lands on both Pis, and adds the
  SSD's free space and its own end of the WiFi link.
- **GreptimeDB**, standalone, from a pinned release with its SHA-256 in the repo, since no
  apt source carries it. Its data is on the SSD; it listens on loopback alone.
- **The recorder**, `recorder.py`, subscribing to `#` on loopback and writing every live
  message into Greptime's `rig` database over InfluxDB line protocol, in batches every five
  seconds. Greptime has no MQTT listener of its own, and its documented route is EMQX's rule
  engine, which would mean replacing Mosquitto; a recorder is a page of Python.
- **Grafana**, from Grafana's apt repository, reading Greptime as a MySQL server through its
  built-in data source, on loopback, its settings a drop-in for its unit and its data source
  provisioned from the repo. It is there so a dashboard can be made when one is wanted; none
  is provisioned, and one made in the UI lives in Grafana's own database.

**What is stored.** One table per topic's first level — `stem`, `memory`, `brain`, `health`,
`sys` — with the rest of the topic as the `topic` tag, and the payload in `value` where it is
a number, `true` and `false` as 1 and 0 so a flag draws as a line, and in `text` otherwise.
The time is when the recorder received it. Kept forever: a few MB a day against 500 GB.

**Not stored:** retained replays, which the broker sends on every connect and would write
each state again each time; empty payloads, which delete a retained topic rather than say
anything; and anything before the clock has synchronised once, which would be filed under the
stale time the Pi booted with.

**The broker comes first.** Greptime and Grafana run at a lower priority, Greptime's writes
waiting on everyone else's disk, and are the first the kernel kills when memory runs out, so
a compaction or a heavy dashboard can never take the broker down with it on a 1 GB Pi. They
are not capped in memory: the Pi's kernel ships with the memory cgroup controller off, and
turning it on is a boot parameter on a file the repo cannot own. Grafana, a Go program that
otherwise holds on to what it freed, is kept small with a Go memory limit instead.

**The SSD carries the recordings only.** `memory-pi` boots from its SD card and mounts the SSD
on `/tank` with `nofail` and a short device timeout, so a missing SSD costs the recordings and
leaves the Pi up and reporting why. Greptime requires the mount, so it never writes into the
empty mount point on the SD card.

**Power is cut, never shut down.** The cabinet has no off switch. Greptime writes its log to
the SSD every ten seconds, so a power cut costs at most those seconds and what the recorder
was holding: the recordings are read in days, not seconds.

### 3.10 Wrist

A crank on a stepper, driving a line to the sculpture; a wireless fixture on the universe; and,
for now, the Bluetooth central that relays Dendrite (§3.11).

**A fixture, not a motor on the Pi.** The universe is the one way anything on the rig is told
to move, which is what lets the e-stop, a takeover and signal loss cover the crank with no
rule of their own, and what lets an artist session turn it from a fader with no code. The
brain writes parameters; Wrist renders the motion at its own rate — the ball's rule, for the
ball's reason: forty frames a second over WiFi, arriving bunched, cannot carry step pulses.

**Limp on stop and on loss, not held.** Every other fixture holds its last values when the
signal goes, and the rig's e-stop holds the last frame for the same reason: a head that stays
put is safe. A crank is not a head. Its output is a force on a structure through a line, and
a crank held under power against a swinging sculpture is a tug-of-war with whatever is
pressing the stop. So Wrist's no-function band is **Free**: the driver is disabled and the
crank turns as the line pulls it, as if the motor had lost power. The same on losing the
universe. Nothing turns under power while the stop is pressed or the signal is gone. This is
the one fixture whose signal-loss behaviour is not "hold", and this paragraph is why.

**Free means unhomed.** The motor's encoder is inside its driver and never reaches the
firmware; while limp, the crank can be turned by the line, by a hand, by the sculpture
settling, and Wrist cannot see it. So leaving Free, for any reason — boot, stop released,
universe back, an alarm cleared — leaves Wrist **unhomed**, and unhomed it accepts only
Hold, Home and Park. A Speed or Lock in the frame is refused and reported, never resumed:
the rig's rule that a show resumes mid-stride on release is right for light and wrong for a
crank, and this is where the difference is enforced, in the fixture, where the show cannot
override it by not knowing about the stop.

**Home and neutral.** A microswitch tripped once per revolution by a tab on the crank disc.
Home is its rising edge, reached by turning slowly in the positive direction; a revolution
and a half without it is an alarm. Neutral is home plus a configured offset, the one measured
fact about a rig's build Wrist carries: the angle at which the crank arm stands perpendicular
to the line with the sculpture at rest, measured at commissioning with the line attached and
written into `wrist`'s config with the date, per §1.1's rule about measured facts. Park is
"home if unhomed, then go to neutral". In Speed and Lock the switch doubles as the index:
two revolutions without it, or an edge where it should not be, and Wrist goes Free and says
why — lost steps on a closed-loop motor mean something mechanical.

**Kick is a counter.** A one-frame band over multicast WiFi can be lost or arrive twice; a
counter that moved is read exactly once (§1.4, the mouth's trigger rule). Wrist kicks on a
change it sees while parked and homed, re-baselines the counter whenever it leaves Free, and
abandons a kick the moment the frame changes mode.

**Lock renders a phase it is sent.** In Lock, Wrist runs its own oscillator at the commanded
frequency, advances a local copy of the sway phase between frames, nudges that copy toward
each received value with a time constant of seconds, extrapolates it forward by a configured
latency estimate for the round trip, and holds crank angle = phase + offset. The frame
corrects; it does not command. That is what lets 25 ms frames with WiFi jitter carry a
sub-degree phase at sway frequencies.

**The relay, for now.** Wrist holds the Bluetooth connection to Dendrite and forwards every
sample to the brain as OSC over UDP — §1.4's path for remote sensing — with the crank angle
and Wrist's own clock stamped on it, and publishes the raw axes on MQTT for Memory. It never
acts on a sample. The central is here because `claw-pi`'s radio shares the UART the DMX HAT
needs and is disabled with it; the day the brain has a radio of its own, by a USB adapter or
by moving to an ESP, the central moves there and nothing in the brain changes: it decodes OSC
either way. Stamping the crank angle on each sample is why the brain can measure the lag
between crank and sway, and the phase error of Lock, with no shared clock and no reading of
telemetry: both numbers arrive in one message from the one place that knows both.

**Hardware.** A Waveshare ESP32-S3-RS485-CAN in its DIN case: the motor's pulse and direction
on its two outside GPIO, enable, alarm and the switch on its internal header, 3.3 V to 5 V
through a 74HCT14. Its isolated RS-485 port is why it was chosen over a bare board: it makes
Wrist a wired fixture the day the access point is the thing we do not want to depend on. WiFi
and Bluetooth share its one radio by time-slicing; the step timer runs on the other core from
both.

**The alarm is the motor's only word.** An integrated closed-loop stepper reports nothing but
an alarm line: no encoder, no in-position signal, no serial port. Its alarm latches, and
clears only when the pulses stop or the power is cycled; cleared while pulses are still
coming, the motor restarts at once, at whatever rate they arrive. So Wrist goes Free on an
alarm and stops pulsing before it clears one, and comes out of it unhomed like any other
Free.

**Telemetry**, on its own connection with a will like the ball: `wrist/mode`, `wrist/homed`,
`wrist/refused` (what the frame asked for that it would not do, and why), `wrist/alarm`,
retained on change; `wrist/angle_deg` and `wrist/freq_mhz` at 10 Hz while not Free;
`health/wrist`. And on Dendrite's behalf, §3.11.

### 3.11 Dendrite

The sway sensor: an accelerometer on the sculpture, streaming raw axes over Bluetooth LE to
whichever device is its central, for the brain to make a `SwayState` of.

**Raw axes, not a result.** The eyeball reduced landmarks to arm angles on the device so the
brain would not depend on one model's joints (§3.6). Dendrite goes the other way, and for the
same reason read forward: the reduction — axis choice, phase extraction, amplitude scaling —
is what the PoC is iterating, and the brain is where an iteration is a build and a deploy
rather than a firmware flash on a thing Velcroed to a sculpture. The brain picks the sway
axis itself, from the principal axis of the oscillation, so the sensor's orientation on the
structure does not matter and the perpendicular axis comes free as the whirl detector. When
the reduction is boring it may move down; `SwayState` is the contract either way.

**The wire.** One GATT service of ours, defined once in `dendrite-wire`: a sample
characteristic notifying packed samples — a 16-bit wrapping counter and three 16-bit axes in
milli-g, little-endian, as many per notification as the negotiated MTU allows and two at the
default — and a health characteristic once a second: sample rate, battery millivolts, name.
The counter is what makes a bunched or dropped notification harmless: the estimator
interpolates by count, not by arrival. The samples come at the accelerometer's own 50 Hz data
rate, so the rate is the sensor's, not a timer's. The relay turns each sample into one OSC
message, `/dendrite/sample`, with the counter, the axes, the relay's crank angle in
centidegrees (−1 when unhomed) and the relay's uptime in milliseconds; and `/dendrite/health`
with the rate, battery and link state. The brain binds the access point's address on UDP
9001; a sample older than half a second is stale.

**Today's hardware.** A BBC micro:bit V2 on a battery pack: an nRF52833 with the accelerometer
on board, which is the whole reason to use it. Its firmware is Rust on Embassy, built on the
Mac, flashed with `probe-rs` over the board's own debug interface. It has no gyro; at the top
of a swaying structure its accelerometer reads tilt and lateral acceleration mixed, both in
phase with displacement, so phase and frequency come out clean and amplitude in degrees is
off by the ω²A term. The rig's Dendrite is an ESP32-C3 with an IMU that has one, on the
access point, sending the same OSC itself with no relay in between.

**On MQTT.** Dendrite has no network identity of its own, so it has no `health/` topic; its
relay publishes its subtree for it: `dendrite/x`, `dendrite/y`, `dendrite/z` one message per
sample while Wrist is not Free (a few hundred messages a second, which the broker does not
notice and Memory stores at half a gigabyte per session day), `dendrite/rate_hz`,
`dendrite/battery_mv`, `dendrite/link` retained. The relay's will covers them.

### 3.12 The inner ear

The sway estimator, in the `innerear` crate: `no_std`, pure arithmetic, the same code on the Pi today
and on an ESP tomorrow.

**A PLL per frame.** From the incoming samples it keeps the principal axis of the recent
oscillation, locks a second-order loop to the signal on that axis with a bandwidth of a tenth
of the frequency it tracks, and hands the frame loop a `SwayState` through the latest-value
seam: phase, frequency, amplitude on the axis, amplitude on the perpendicular, and a lock
quality — the fraction of the signal's power the locked component explains, 0 to 1, with
locked meaning above 0.7 for a second. The receiver thread runs it per sample at normal
priority, below the frame loop, like every producer (§1.6).

**Free decay.** Given a window of samples after a transient, it fits f₁ from the zero
crossings and Q from the log-decrement of successive peaks, and reports how many cycles
cleared the noise floor; fewer than three and it says the Q is not to be trusted rather than
printing one. The window opens on a transient above a threshold — a pluck or a kick — not on
a button press, so the hand that plucks is not also the hand that presses.

**Lag and phase error** come from the stamped samples, not from telemetry: the crank angle
that arrived on each sample against the phase the loop assigns to it gives the lag between
crank and sway; in Lock, the phase the brain sent for that frame against the crank angle that
came back gives the phase error. Both are published as the brain's own derived state,
`brain/sway/lag_deg` and `brain/sway/phase_err_deg`, decimated with the rest of
`brain/sway/*` to 5 Hz, which is what a dashboard can show and what the queue can carry.
