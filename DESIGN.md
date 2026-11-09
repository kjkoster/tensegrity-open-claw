# Tensegrity Open Claw — Design Document

**What this file is for.** Where code goes and why: the principles, the structure, and a
section per component — including components no longer in the code, so this file is a
knowledge base of parts a show can be built from. Which of them a rig has changes from show to
show; the rest of the file assumes none. Fixtures are not components and have no section here.
It does not describe what the code currently does, nor fixture addressing, which lives in the
QLC+ workspace.

How to set the system up and operate it is [`README.md`](README.md); one show is
[`SHOW.md`](SHOW.md); the hardware we own is [`OWNED-DEVICES.md`](OWNED-DEVICES.md).

---

## 1. Principles

### 1.1 The QLC+ workspace is the patch

**None of the fixture addressing is written in Rust.** A rig's QLC+ workspace — the claw's is
`clockwork.qxw` — is the source of truth for *where* its fixtures sit, and the `.qxf` definitions in
`fixtures/` for *what their channels mean*. The build ingests both and generates a struct per
fixture, with a named field per channel, into that rig's own crate.

**The universe and the sACN port come from the same file**, off the E1.31 output patched to the
workspace's first universe. That output describes the console's own transmission, but a rig
has one universe and relays a takeover onto it, so those numbers are the rig's too: what the
brain listens for, what it sends, and what a wireless fixture listens for. The address the
output unicasts to is not the rig's: it has to be the brain's, as `clockwork.dot` has it, and
the build fails a workspace that unicasts anywhere else.

**The workspace holds no network addresses.** The rig's IP space — every device's address,
the services it runs and the ports they listen on — is in `clockwork.dot`, and a networked
fixture's address is compiled into its firmware from there.

**The running brain never reads the workspace**, not even as a filename: the build consumes it.
A rig has exactly one, so naming the rig in the log already says which file the binary was
built from.

The decision is to spend a build script to have **no second copy of the patch**. Channel
offsets written out in Rust would be a copy of something edited by hand in another program, at
load-in, under time pressure: right until the first repatch and silently wrong thereafter,
because a wrong offset looks wrong only once the rig is lit. Named fields hand every direction
of drift to the compiler instead, so there is no drift checker; what the build catches is
listed in the README.

**Scenes are QLC+'s own**, looks for a takeover; the build does not read them. While the rig
had moving heads, scenes were also its pointing presets — where a head points is a measurement
taken with a console, so it lived in the workspace like the patch, and the build resolved each
into a pose per head in degrees. That ingest and `cortex`'s moving-head code are on the
`last-version-with-mouth` tag.

**The same rule runs past addressing into values.** Which value opens the shutter, which end of
a speed channel is fast, which wheel positions are split, how far a head pans: each is a fact
about the fixture, and each written into Rust is the same second copy — worse than an address,
because a wrong speed polarity is wrong on one model and right on the next. So the ingest keeps
growing to read more of the definition, and the code asks the patch. Where the definition does
not carry a fact yet, the fix is to measure it and put it in the `.qxf`, which makes it work for
every fixture sharing the definition at once.

The one exception: a `.qxf` says what a fixture's channels *are*, not how it *behaves*. The
slowest speed at which a head still moves smoothly is a measured fact about the model with no
field in the format to hold it, so it lives in `cortex` beside the code that enforces it. The
test is whether the format could carry it.

What stays written in code is what is genuinely a choice — how fast we are willing to slew, how
long a breath lasts — with whoever chooses: the rig crate for a rig's numbers, `cortex`'s
`config.rs` for the cabinet's.

### 1.2 External takeover: strict priority, whole universe

A console can take the universe live and hand it back, so a human can drive the rig during a
soundcheck or an emergency without stopping the daemon or touching the Pi.

**The internal engine is a source at priority 100, and an external stream takes the universe
only if its priority is strictly greater.** Every sACN source ships defaulted to 100, so any
laptop joining the network with a live universe would otherwise seize the rig; taking over must
be deliberate. Equal priority is a no-op, not a tie-break.

Takeover is **whole-universe**: no HTP or LTP merge, no per-fixture claim. Merging is a wrong
feature, not a missing one — a fixture's channels are positions, wheel indices and modes, and
`max()` of two wheel indices is a position neither source asked for. Slots a short external
frame does not carry go to zero; half a console's look is worse than none of it.

Both outputs follow a **single arbitration decision** per frame, so the wire and the network
cannot disagree about who is driving. The relay goes out under the brain's **own CID and
priority** on its own sequence counter, so a takeover is invisible to anything watching the
stream's identity. The show keeps running while overridden, so handback resumes mid-stride.

**The e-stop comes after the arbitration**: it acts on the frame as it goes out, because a stop
button a laptop can override is not one. It holds the last frame under a mask compiled per
channel at build time from the channel's group, preset and bands. Not all zeros, because zero
is "off" only for light: for a head it is the end of travel, for a speed channel often the
fastest. Light goes to blackout by preset rather than by group, because hue, saturation and
CMY share the Intensity group and are not dark at zero. Shutters close where the definition
says how and open otherwise: the light is already out, and what matters is that the strobe
stops. Every Effect and Maintenance channel goes to its no-function band, settings included,
because channel names are free text and the one guessed harmless is the program or reset that
fires when held. A forced channel whose definition marks no such band fails the build: a
guessed stop value is the one that becomes a visible event.

The receiver binds the address `clockwork.dot` gives the brain's sACN port, alone, rather than
the wildcard, and the workspace's E1.31 output has to unicast there. The brain sends its own
universe by unicast too, to each fixture the dot gives a `sacn` edge from it, so nothing it
sends comes back to its receiver.

### 1.3 MQTT is telemetry, never show

Every topic on the broker reports; nothing the show does reads one. The show's inputs are its
sensors and the clock. That keeps the broker optional — one that is down or slow costs the
telemetry and never a frame — and keeps the show's behaviour explainable from its inputs alone.

### 1.4 Inputs are state, handed in

A part that senses the world publishes a snapshot on its own latest-value seam, and the state
machines take those snapshots as arguments each frame, so every input each depends on is in its
signature. There is no common input type: it would only be unpacked again, and the frame loop
would have to learn every sensor's shape.

An event that could fall between two frames is a counter, not a flag: a reader at frame rate
sees a press however short it was. A snapshot carries the time it was taken, so a quiet sensor
is a stale snapshot, and what stale means is the show's decision.

**A sensor on another machine arrives over OSC**, decoded by a receiver thread into that
sensor's snapshot on the same kind of seam, so moving a sensor changes the transport and
nothing after it. UDP, because a late reading is worthless and the newest wins; OSC, because it
is the smallest typed message there is and a laptop with `oscsend` can stand in for the sensor;
not MQTT, because a broker in the path would make it part of the show (§1.3). The sender sends
each new reading once, so silence goes stale. The receiver binds the one address the sensors'
network reaches, because a wildcard would also answer the 4G uplink's public address.

**One definition for every non-DMX stream.** As the QLC+ files are for DMX (§1.1), an
OSCQuery namespace file — the JSON show-control tools use to describe an OSC device: every
address with its type tags, ranges and units — is the one source of truth for a device's OSC,
in `fixtures/` beside its `.qxf`. The build compiles it into the encoders and decoders at both
ends, whatever language each end is in, so a key or a type that disagrees between them is a
build error, not a confusing trace.

### 1.5 Time is resolved at build time

The show table names a city and lists local start/stop pairs; the build turns that into a
zone, UTC instants with their offsets, and the civil dusk-to-dawn stretches inside each show.
The brain sees only timestamps — no time-zone database, no solar arithmetic, nothing that can
disagree between the Mac and the Pi. The hard cases become build errors: a night across the
clock change, a local time in the hour that happens twice or never.

**The brain does not wait for NTP.** The Pi has no RTC and the 4G link can take a minute. After
a reboot the restored clock is behind, never ahead, and behind is the safe direction: the
startup listing keeps too much rather than dropping a show, and the per-frame decision corrects
itself when NTP steps the clock.

### 1.6 What may fail runs in its own process

Anything the show must survive losing, and that has no reason to be inside the brain, runs
outside it: Stem, and Memory on a Pi of its own (§3.9). Its crash then costs what it provides
and never a frame, and the process driving fixtures beside people shares no address space with
it.

The exceptions have a reason to be inside. The face, and the ear and the mouth when a rig had
them, must stay in step with the frame to the millisecond; a process hop is latency the
audience sees. The eyeball's reading would otherwise have needed a socket between two
processes on one Pi. These are written so a dead device costs what it provides: they report,
retry with backoff and never block the frame loop; what cannot come back stops the brain
(§1.7). Heavy work drops out of the real-time scheduling, so it slows itself and never a frame.
They publish through the brain's broker connection (§3.1).

Each thread sets its own core and scheduling as its first act, from `cortex::sched`, rather than
the unit file setting one policy for the whole process, because the frame loop and the threads
beside it need opposite things. A thread spawned afterwards inherits both, which puts
embassy-time's timer thread beside the frame loop and an inference runtime's workers on the
background cores. A failure to set them stops the start.

| thread | cores | scheduling |
|---|---|---|
| the frame loop (the executor, with the face's tasks) and embassy-time's timer thread | 3 | FIFO 50 |
| `sacn-in`, telemetry, signals | 0–2 | `SCHED_OTHER` |
| Stem, in its own unit | 0–2 | `Nice=10` |

No core is isolated on the kernel command line: with everything else pinned off core 3, what is
left there is preempted by the frame loop anyway.

The price of inside is that a crash in a part is a crash of the brain, which systemd restarts
within seconds. That is fair for Rust, whose failures are panics the brain is built to end on;
it is real for native code a part links, such as a C++ inference runtime, which is why a
pure-Rust alternative is worth its cost in speed.

### 1.7 Recoverable faults are soft, unrecoverable faults are hard and fast

A fault is recoverable when it can go away by itself: a device unplugged and replugged, a link
that drops and returns, another machine that comes back. Those are reported, retried with
backoff, and cost only what they provide; the show runs on.

Every other fault — a missing boot overlay, a serial port that will not take a write, a device
that refuses the format the build was made for, a port another process holds, a thread that
cannot be pinned, a library the deploy should have installed — panics at the point of failure,
preferably at startup, so the journal names the cause and systemd's restart loop makes it
impossible to miss at the bench. Retrying only repeats it, and carrying on hides a broken rig
behind one that seems to run. The workspace builds with `panic = "abort"`, because unwinding
would end only the panicking thread and leave the brain looking healthy without it.

The test is not how much the thing matters to the show, but whether waiting could ever fix it.

### 1.8 Everything at its latest, at the start of every show

The first step of every new show is upgrading everything to its newest release and making it
work again: the Rust toolchain and crates, the Python packages, GreptimeDB, Grafana, QLC+,
Raspberry Pi OS on every Pi, and every device firmware still shipped. Pre-releases count where
a dependency has only those. An older version stays only for a reason written beside its pin.

At the start there is a whole build ahead to find what broke; the week of a show it is a risk;
and never upgrading is a pile of breaking changes that arrives all at once.

### 1.9 `no_std` and no allocator, wherever it can be

Anything shared between the Pi and a microcontroller is `no_std` — always, not when a firmware
happens to need it. `stem` set the pattern. Code that was `std` out of habit is what stops a
part moving onto a microcontroller. A crate reaches for `std` only for what the Pi alone has —
sockets, threads, the serial port, the filesystem — in the thinnest shell it can.

The same for the heap: portable code works in fixed-size buffers its caller owns. `stem` writes
each packet into one and returns its length; the sACN encoder and source table are arrays
sized by the protocol and the configuration. An allocation in the frame loop is a porting job
waiting to happen.

---

## 2. Structure

### 2.1 System overview

A **Raspberry Pi drives one DMX-512 universe**. It runs `brain`, which reads the rig's sensors,
runs the show, and clocks the universe out of an RS-485 HAT at 40 Hz to the wired fixtures.
The same universe goes out as E1.31 sACN for the wireless fixtures, a monitor or a console, and
an external override arrives the same way. The Pi is a WiFi access point, and reachable from
the Mac over a WireGuard tunnel. Telemetry — the broker and its recordings — lives on a second
Pi on that access point, Memory (§3.9), so the Pi that drives the show carries none of it.

```
    ┌── Mac ──────────────────────────────────────────┐
    │  (builds brain; deploys; QLC+ for override)     │
    └───────────────── network ───────────────────────┘
                                  │
    ┌── Raspberry Pi ──────────────┴──────────────────┐
    │  (runs brain; reads the rig's sensors)          │
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

The daemon emits exactly one universe, and the ingest rejects a fixture patched outside the
first rather than silently dropping it. Every rig so far fits in 512 slots; a second universe
would need a routing layer between the engine and the wire.

Wired DMX-512 drives the wired fixtures; sACN carries the same universe over the network in
parallel, at no cost. Art-Net is not supported: we control every device on the network.

**The wired frame is padded to a full 512 slots.** A short frame is a known way to lose cheap
DMX receivers: they ignore the data and fall back to their own internal show while still
reporting a healthy signal, which reads as a wiring fault and is not one. The padding sets the
~43 Hz ceiling the 40 Hz frame rate sits under.

### OSC: one file per device, three ways to send

A device's OSC is its OSCQuery file in `fixtures/` (§1.4), written in OSCQuery's own
attributes: `FULL_PATH`, `CONTENTS`, `TYPE`, `ACCESS`, and where they help `RANGE`, `UNIT` and
`DESCRIPTION`. Every level of a path is a node of its own. A container has `CONTENTS` and no
`TYPE`; a leaf is an OSC address, with `TYPE` and `ACCESS`. The build rejects a `FULL_PATH`
that is not its parent's path and its own key.

**The address space is by subject, with the device beneath it**, as MQTT's topics were:

| root | holds |
|---|---|
| `/<device>/…` | the device's own loop variables and raw inputs |
| `/dmx/<device>/…` | where it sits on the universe, as the patch says, and how fresh its frames are |
| `/sys/<device>/…` | what it runs on |

**One value per message.** A value's name comes from its leaf's name and path, so nothing
names arguments, and a message that carries several values has no names for them. Values that belong together are
sent together by sharing a rate, not by sharing a message.

**Each leaf is sent one way**, and a leaf with none or with two is a build error:

| attribute | sent | for |
|---|---|---|
| `"CLOCKWORK_CHAOS_ON_EVENT": "connect"` | each time the device joins the WiFi | what does not change while it runs: the patch, the MAC, the reset reason |
| `"CLOCKWORK_CHAOS_ON_EVENT": "data"` | once, when a new reading arrives | a sensor with a rate of its own, so each sample is sent once and none is repeated |
| `"CLOCKWORK_CHAOS_AT_RATE_HZ": <n>` | `n` times a second | a state, so a listener that starts late or loses a packet is one period behind |

On connect, not at boot: a device boots and joins in seconds, before the Pi that listens has
started, and a message sent to nobody is gone. OSC has no retention, so a listener that starts
while the device stays connected hears the connect messages only at its next connect.

Leaves at the same rate go out in one bundle, so a bundle is never declared: it follows from
the rate.

### 2.3 The crates

| | |
|---|---|
| `cortex/` | what every rig shares under its show: both transports, the frame loop, the show calendar, the latest-value seam |
| `cortex-build/` | the QLC+ ingest and the calendar's resolution, a build-dependency of the rig crate |
| `brain/` | the rig crate: the rig's patch, its show times, and its show |
| `stem/` | Stem as the Rust programs see it (§3.1): the health contract and the MQTT packets, purely `no_std` and allocation-free. It shares its directory with Stem's daemon and broker configuration |
| `wrist/` | Wrist's firmware (§3.10), C++ on the Arduino core for the ESP8266, not a crate |

**Beside the crates**: `stem/` also holds Stem's daemon, its broker configuration and the
Python modules every health daemon shares; `pi/chaos-pi/` is what `claw-pi`'s deploy installs
that belongs to no crate; `pi/memory-pi/` is everything `memory-pi` runs. A daemon's systemd
unit lives in its own directory. Each Pi has its own deploy script, sharing `pi/deploy-lib.sh`,
so either deploys while the other is down. `clockwork/` holds what reads `clockwork.dot` at
build time: the dot's reader, and the renderer that fills the Pis' configuration from it.

The rig crate's `build.rs` names its workspace and reads `shows.rs`; its `main.rs` includes the
generated patch and calendar, wires in the components it uses, and writes the show.

**`cortex` holds only what every rig shares.** A component a rig may or may not have is its own
crate with its own configuration, which the rig crate depends on and wires in, so `cortex::run`
and its `Rig` never grow a field for one rig's part.

**Everything is built on the Mac.** The brain is cross-built for 64-bit ARM Linux, statically
linked against musl: the Mac builds in seconds what the Pi took minutes over, and the Pi needs
no toolchain and no sources. Firmware for a wireless fixture is built on the Mac for its own
target and reads its universe, port and address out of the workspace through `cortex-build`;
Rust firmware shares `stem` by path.

**Assets are compiled in.** A recording or a model a component needs goes into the binary at
build time, so the binary is the whole show: no file to be missing at startup, no decoder at run
time, and an unreadable asset is a build error naming it. The mouth's recordings and the
eyeball's model were built this way (§3.6, §3.8).

### 2.4 Software stack

Standard Rust, `std`, on Raspberry Pi OS. Rust is preferred everywhere; a microcontroller whose
Rust support falls short may force C++, and that is then a design choice stated in the
component's section.

- **A hand-rolled sACN encoder** on a UDP socket. The `sacn` crates were not a good fit; the
  packet is small enough that owning it costs less than working around them.
- **`zihatec-rs-485-dmx`** for DMX-512 framing — break/MAB timing and full-512 padding.
- **Embassy** (`executor-thread` + `embassy-time`) for the frame loop, with blocking producers
  on their own OS threads feeding it through a lock-free latest-value seam. A blocking `recv` on
  the executor's thread would stall the frame loop, so anything that blocks stays off it, and
  anything that computes for long runs below it in scheduling (§1.6).

### 2.5 The rig state machine

The brain runs **two state machines, layered**, and keeps their states apart by name:

- The **rig state** — boot hold, running, e-stop, shutdown — says whether, and what, goes out.
  It lives in `cortex`'s frame loop, after the arbitration, published on `brain/rig`.
- The **show state** says what the show asks for. It is the rig crate's Director, described in
  [`SHOW.md`](SHOW.md).

The show state machine never sees the rig state, so it has no e-stop, boot or shutdown cases,
and runs on unchanged underneath a stop.

The rig state runs boot hold → running ⇄ e-stop → shutdown. Boot hold leaves on the first
frame: to running if the e-stop is released, or to an e-stop holding nothing, silent until
release. Until a frame has gone out the brain does not know what the fixtures are doing, so
silence, which their signal-loss behaviour answers by holding, is the only honest stop frame.
Shutdown releases the sACN source on the frame loop's own send path, so there is only ever one
sender. Neither boot hold nor shutdown can be returned to.

### 2.6 The Pi controller

The deployed unit is a **Raspberry Pi 3B+**; a Pi 4 runs the same software with more headroom.
It runs in **AP mode** so fixtures and laptops join its own network rather than whatever WiFi
the site has, uses its Ethernet port as a wired segment for a laptop, and runs `brain` as a
systemd service.

**Everything that must be found by address has a fixed one, outside the DHCP range.** Both
are in `clockwork.dot`. Each network hands out its range by DHCP as a convenience for
laptops; a device the rig talks to by address sets its own on itself — a second Pi from a
profile rendered from the dot, a wireless fixture from its firmware's header, generated from
the dot. Both networks route their clients out over 4G.

---

## 3. Components

**The rig is a fixed core and the components a show chooses.** The core stays in the code
whatever the show. A component is in the code only while a show uses it: when a show is done
without one, it is tagged `last-version-with-<component>` and removed, with everything that
existed only to serve it, and its section here stays as what was learned. A later show restores
it from its tag. Fixture definitions in `fixtures/` are a library, kept whether or not a
workspace patches them.

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
| Memory (§3.9) | `pi/memory-pi/` |
| the 4G watchdog (§3.2), the access point, the wired segment, WireGuard | `pi/chaos-pi/` |

| Component | In the code |
|---|---|
| the ear (§3.4) | no: `last-version-with-ear` |
| the ball (§3.5) | no: `last-version-with-ball` |
| the eyeball (§3.6) | no: `last-version-with-eyeball-for-pose-estimation`, `last-version-with-eyeball-for-group-detection` |
| the retina (§3.6.1) | no: `last-version-with-retina` |
| the ponytail (§3.7) | no: `last-version-with-ponytails` |
| the mouth (§3.8) | no: `last-version-with-mouth` |
| moving heads: their aiming and the QLC+ pointing presets (§1.1) | no: `last-version-with-mouth` |
| Wrist (§3.10) | `wrist/`, `fixtures/wrist.qxf`, `fixtures/wrist.oscquery.json` |

### 3.1 Stem

The telemetry backbone: the MQTT broker, Mosquitto, and a health daemon in Python. All of the
rig's telemetry goes through it (§1.3). It is separate from `brain` (§1.6) so it stays alive and
reporting when `brain` is down — precisely when health matters — and being Python and a broker,
it has no place inside the brain anyway.

**The broker belongs to Stem and runs on `memory-pi`** (§3.9), beside the recordings that read
it, so the Pi that drives the show carries none of the telemetry. It is Debian's Mosquitto with
an overlay in its include directory (`stem/stem.conf`), so a package upgrade cannot quietly
revert the rig's settings. It keeps nothing on disk; every client replays its retained topics
on each connect. Stem's daemon stays on `claw-pi`, the hardware it reads.

**The daemon is deliberately the least capable process on the rig**: it reads the hardware and
publishes it, nothing else, so its death costs telemetry and never uptime.

Every process holds its own broker connection, announces itself on `health/<service>`, and
leaves a will that says `gone`. A part inside the brain, like the face, lives and dies with it,
so a will of its own would only repeat `health/brain`; it publishes its subtree through the
brain's connection.

**The Rust side of Stem is the `stem` crate**: the health contract and the packets, `no_std` so
firmware can use them; each program brings its own transport, the brain's being
`cortex::telemetry`. `stem/rig_mqtt.py` is the Python half, and `stem/rig_board.py` the board
readers every Python health daemon shares.

### 3.2 The 4G watchdog

A shell script on a systemd timer that keeps the 4G uplink, and the WireGuard tunnel over it,
alive.

**It finds the modem itself, every run**, because the cabinet is rewired for every show: on the
USB bus by its vendor, then its network interface and control port under that device in sysfs,
then the ModemManager modem whose physical device it is. Nothing names a port, a hub or an
interface.

**It touches the modem alone**, so the show is never disturbed: no reboot, no power cut. A
reboot breaks the show for a link the show does not need. A per-port power cut can be trusted
only on a hub that really switches its ports individually, which a hub's own claim does not
prove; on a ganged hub it would cut whatever shares the hub.

### 3.3 The face

The cabinet door — e-stop, two buttons, a green lamp in each — is wired to the Pi's GPIO, but
treated like any fixture in both directions rather than as a side channel.

**Its lamps are patched.** `fixtures/face.qxf` gives them two dimmable channels. The show writes
them into the frame; the face drives the pins from the frame that actually went out, after
arbitration, so a console's takeover drives them and the e-stop darkens them with no special
case. They dim through the Pi's two hardware PWM channels, because the software alternative is
a busy-waiting real-time thread per pin, competing with the DMX timing.

**Its inputs are a `FaceState`**, polled, debounced and published once per poll on its own
seam: the e-stop, and counts of each gesture so a show reading at frame rate misses none
(§1.4). The show reads the buttons; the e-stop goes to the rig state machine (§2.5), read once
when the pins are claimed so the first frame already knows it.

**It cannot fall silent on its own.** The face polls as a task on the frame loop's executor, so
it stops only when the frame loop does, and a panic in either ends the process. That is why
there is no rule for a face that has stopped reporting, and why it has no broker connection of
its own.

### 3.4 The ear

The rig's microphone input: capture, feature extraction, and the `AudioFeatures` it handed the
show — onsets, and how much of what it heard was music — which drove the washes' sparkle and
the heads' dance. It drove the claw at ADE and was removed after it; the code is on
`last-version-with-ear` (`cortex/src/ear/`, and `cortex/src/sparkle.rs` for the mapping).

It had no DMX channels, but it was part of the rig the way the face is — its own thread, its
own state handed to the show, its own `ear/…` topics — so a dead interface showed as such
rather than as a show gone quiet for no visible reason.

**Fast and lean.** Every feature was computed on every block, so the snapshot carried only what
a show read plus diagnostics; a feature nobody read was work for nothing on a 1.2 GHz Pi. Its
capture thread — negotiated parameters, xrun recovery in place, reopen with backoff, a USB
probe published on every reopen — is the mouth's (§3.8) turned round. The interface was named
by role, so swapping it was its ALSA card name and USB identity in the config.

### 3.5 The ball

An ESP32-C3 on the Pi's WiFi, following the sACN universe, its firmware `no_std` Rust on
Embassy. Removed after the ADE show; the last version, with the deploy's firmware build and
board flashing, is on `last-version-with-ball` (`ball/`, `ball-dmx/`, `attach.sh`). Its
definition, `fixtures/ball.qxf`, stays in the library.

**A ball is a fixture in the patch like any other**, and its firmware is built from the
workspace: every ball patched through `fixtures/ball.qxf` becomes a constant carrying its
address, the channel layout and bands are read out of the definition, and a board is
provisioned by naming the one it is.

**The ball draws its own waveforms.** A 25 ms frame is longer than the pulse's hold, and
multicast over WiFi arrives bunched, so the brain sends a pattern and its parameters and the
ball renders them once per period of its 500 Hz PWM. The strobe's flash and gap lengths are in
a unit a `.qxf` cannot express, so that unit lives in `ball-dmx`; everything the `.qxf` can
express, the firmware build reads from it (§1.1).

**The ball sleeps outside shows.** Its radio is most of its draw. The brain asks it to sleep
over DMX, and it deep-sleeps in five-minute stretches, waking each time to report and ask
again. Deep sleep is a reset, so every wake is a boot, and every boot runs the same way:

- The outputs start held off and the motor tripped.
- sACN is followed from the moment the network is up, whatever comes next.
- Telemetry goes out as always, the pack voltage read at boot among it; publishing never holds
  up sACN.
- Five seconds after the lease — not boot, since joining takes seconds of its own — the ball
  decides. That is plenty for a broker that is there, and waiting longer would not bring one
  that is not; liveness is read off `health/ball` going `connected` every five minutes and a
  fresh `ball/pack_v`.
- If the universe asks it to sleep, or none was heard in those five seconds, it sleeps.
- With the rig off there is no access point and the five seconds never start, so a join that
  has not succeeded after four retries and a margin, about a minute, also means sleep.

Going to sleep is, in order: stop the motor and hold its pin low, since nothing drives it in
deep sleep; say `disconnected` and wait for it to leave, the one place anything waits on
telemetry; disconnect from the access point cleanly, so the next wake has no stale association
to wait out; deep-sleep five minutes. A wake counter in RTC memory is published as
`ball/wakes`, so a reset that was not a wake shows as the count starting over.

No universe after a wake means the rig is off, and a ball staying awake for it would drain its
pack when nobody is there. It counts only at that one decision: an awake ball that loses the
universe mid-show darkens and waits. Once awake, only the channel puts it to sleep. The brain
asks from the end of one show until ten minutes before the next — a sleep, a wake and a margin
— never in test mode. The Sleep channel's awake band is `NoFunction`, so the e-stop's mask
wakes the balls with no rule of its own.

### 3.6 The eyeball

One small reading of the scene out to the show, from a camera's frames (§3.6.1), and an
annotated feed for humans. It had two lives. First a Python process beside the brain, reading
a person's arms for the mage rig, on `last-version-with-eyeball-for-pose-estimation`
(`eyeball/`, `cortex/src/eyeball.rs`). Then a Rust crate running as threads inside the brain,
telling the Halloween show how far into the yard the furthest person had come, on
`last-version-with-eyeball-for-group-detection` (`eyeball/`, and the brain's `presence.rs`,
`draw.rs`, `feed.rs`).

**The retina sees, the eyeball looks.** The retina is the camera; the eyeball is what is in the
frame. Between them is a one-slot mailbox of JPEG frames, taken rather than read, so no frame
is detected twice. The eyeball states what it needs in a `View` — resolution, frame rate,
infrared or colour, which way up the camera hangs — and the retina sets the camera up for it,
so a consumer with other needs takes no change in the retina, and a different camera is a
different retina and the same eyeball. Each reports its own half, `retina/…` and `eyeball/…`,
so a dead camera and a slow detector never look alike.

**The rig draws, the eyeball sees.** It hands out one sighting per frame — the distance and
when, with the picture and boxes it came from. The show reads the distance; the annotated
feed, the rig's, draws the rest. What the feed draws is the show's space, and an HTTP server
is the rig's choice, so neither belongs in the crate that finds people.

**Inside the brain (§1.6).** The reading crosses a latest-value seam to the show, as an OSC
receiver's would (§1.4). The price is that a crash in the inference runtime is a crash of the
brain; and the detector runs below the real-time threads on at most three cores, so a slow
model costs the eyeball frames and never the rig one.

**Three outputs, three audiences.** The reading goes to the show once per processed frame with
the time it was seen, so a stalled camera is a stale snapshot, which the show turns into the
safe default. The feed goes to a browser over HTTP, because that is what a human uses to aim a
camera; it binds the WireGuard address alone, because a wildcard would show the space, and
whoever is in it, to the 4G uplink's public address. Telemetry goes to MQTT through the brain.

**The show reads the least it needs**: arm angles rather than seventeen joints in the first
life, one distance rather than boxes in the second. The estimator is expected to be replaced,
and a schema naming its internals would change with it.

**The feed costs nothing unwatched.** Compressing to JPEG again is not cheap on a Pi already
running a detector, so the feed is encoded on its own thread, from the latest sighting, only
while a browser is connected.

#### 3.6.1 The retina

A network camera, an Axis M3105-LVE, set up over VAPIX and read as MJPEG, its newest frame
handed to the eyeball. Removed with the eyeball after the Halloween show, on
`last-version-with-retina` (`retina/`, and the camera's DHCP reservation in
`etc/claw-eth0.dnsmasq.conf`).

**The bounds of a view are types**, checked where the view is written. What belongs to the
camera rather than to whoever looks — the mains' 50 Hz, the exposure for people walking — stays
the retina's own.

**The camera is configured by the retina, not by hand.** Stream, size, rate, rotation and
sensor settings are set over VAPIX on every connect and read back, because a setting made in
the camera's web UI is lost on a factory reset or a replacement. A setting the camera refuses,
or a login it rejects, stops the brain (§1.7). The login is the factory default, kept as is, so
a reset or replaced camera works with nothing done by hand; the camera is on the Pi's private
wired segment, so a default password costs nothing. Infrared beats visible light by a wide
margin, so the IR-cut filter is off under the camera's own illuminator, with exposure capped at
10 ms so a moving limb is a shape and not a streak. MJPEG over HTTP, because it has no
inter-frame state and needs only a JPEG decoder.

**The newest frame, always.** The detector sets the pace: a reader that kept every frame would
queue them behind a slow model and lag the world, so it keeps only the newest, and a slow model
shows as a lower frame rate on the broker rather than a show that reacts late.

**The camera's address is a reservation, not a setting**, in the dnsmasq NetworkManager runs for
the wired segment, keyed on the camera's MAC, so a reset or replaced camera comes up where the
retina looks with nothing set on it.

#### In the brain: one distance

A person detector, not a pose model: a person is the bottom corner of their box on the near
side — the leading foot, which a box merged over a bunch still puts on the one in front.
YOLOX-Nano (Apache-2.0) on the CPU through `tract`, pure Rust, compiled into the binary (§2.3)
with its licence beside it.

**The model's input is the camera's shape**, not the published square: work scales with
pixels, so the weights are exported at the camera's aspect ratio and well under 416×416 —
several times the frame rate, no work on padding — at the cost of people under about 25 pixels
tall. The export is a script beside the model.

**The models not taken.** YOLOX-Tiny is too slow beside the frame loop on a 3B+. YOLOv8 and
YOLO11 are AGPL. A crowd-trained detector separates a bunch of children better but has no clean
first-party export, and the show reads only the front of a group. ONNX Runtime is four times
faster on a Mac, but its crate downloads a prebuilt library; only Debian's package would do.

**No calibration, only aiming.** The camera, square on to the space, is aimed so the space's two
benchmarks land on two columns the feed draws. A distance is where a person's feet fall between
the columns, linear in the picture and so as linear in the space as the camera is square,
which nothing comparing distances needs exact. A ground-plane homography is right at any angle
and is four points to measure and a matrix to get wrong.

**The unit is the space, not the metre**: 1 at one benchmark, 0 at the other, nothing measured
with a tape, and an empty space `+∞`, so every comparison falls the safe way. Debouncing and
hysteresis are the show's policy; the eyeball reports each frame and tracks nobody.

#### Beside the brain: arm angles

A Python process on the Pi, OpenCV and numpy, a MoveNet pose model on TFLite or ONNX, sending
the brain datagrams over UDP on loopback; three cores for the model, one for the DMX loop. The
preview was shown to the audience, so the drawn skeleton was a deliverable, not a debug view.
The model now and then puts a confident landmark on a bush, so a person counted only once five
landmarks cleared the threshold.

### 3.7 The ponytail

A cheap fibre-optic light — plastic fibres lit by an LED array, a patterned plate turned by a
stepper in front — retrofitted with a XIAO ESP32-S3 that followed sACN on the Pi's WiFi.
Removed; on `last-version-with-ponytails`.

Two personalities shared one sACN front end. **PWM** drove the RGBW array directly. **BLE
bridge** kept the original Telink controller and translated DMX into the BLE writes its app
sends — the only path to the gobo motor. That fixture is modal: RGB and white cannot light
together, so White above zero overrode RGB with a hard cut, the dimmer was applied in software
to work the same in both modes, and with no readback the bridge re-asserted the whole state on
every change, on a heartbeat and on every reconnect.

Its decoder did source arbitration in the fixture, so a console could take it over even with
`brain` hung.

### 3.8 The mouth

The rig's sound: recordings played through a USB audio interface into two powered speakers.
Its own crates, `mouth/` and `mouth-build/`, which a rig crate depended on when its show had
sound; `cortex` knew nothing of it (§2.3). It voiced the Halloween show and was removed after
it, on `last-version-with-mouth` (`mouth/`, `mouth-build/`, and the brain's `sounds/`,
`sounds.rs`, `mouth_show.rs`). It was the one part that linked ALSA, and so the reason the brain
could be built only on Linux.

**A unit, so a show without sound leaves it out.** On the rig crate's side it was its
recordings, the sound table naming them and their gains, and one show file through which the
show cued sounds and read their envelopes; taking it out was deleting those and the two crates.

**The recordings were compiled from the files as found** (§2.3): decoded, resampled to the
interface's native rate so nothing resampled at run time, kept stereo, trimmed of what is 60 dB
below peak at both ends so the first sample is the sound, peaked just under full scale, then
taken down by the table's gain, so that gain is the only thing setting the level. A gain above
0 dB would clip, and failed the build. Overall level is the speakers' knobs; no master gain in
software to disagree with them.

**Every recording carried its envelope**: loudness, one byte per DMX frame at the rig's frame
rate, which `cortex` handed the build script through cargo's `links` metadata so a change of
rate rebuilt every envelope. Samples were padded to whole frames, so a look following a
recording played the envelope by frame index from the frame that cued it: sound and light
shared one clock, the frame count.

**A thread in the brain, not a process**, because a sound a process hop late is a cut the
audience hears after it sees it (§1.6). Blocking ALSA writes; a dead interface cost the sound
and never a frame, reopening with backoff and reporting why it was mute; one that opened but
would not play the format the recordings were built for stopped the brain (§1.7).

**Small buffers, because the cut is the point**: two periods of 256 frames put the next sound
about 11 ms behind the frame that asked. A mute zeroed the next buffer and killed every voice.

**The show handed it a cue every frame over a latest-value seam**: a trigger counter per sound
(§1.4) and mute as a level, read every period. Voices were mixed by adding, clipped past full
scale, overlapping from a fixed pool so playing never allocated; when the pool was full the
voice furthest into its recording gave way.

**The e-stop silenced it; a takeover did not.** The e-stop is a show-stop, so the rig crate
muted the cue while the face reported it, at boot too. Sound is not part of the universe, so
under a takeover the show's sound ran on.

**It reported through the brain's connection** (§3.1): what the stream was actually opened at,
xruns, why it was mute, and the interface as the USB bus saw it, republished on every reopen,
so a re-enumeration under a running brain showed as the flaky hub port it usually is.

### 3.9 Memory

The rig's long memory: everything published on the broker, kept in a time-series database on an
external SSD so a show can be read back afterwards — and the broker itself. It runs on a Pi of
its own, `memory-pi`, configured entirely from `pi/memory-pi/`.

**A Pi of its own, because `claw-pi` has no room for it.** A 3B+ with 905 MB, swap on its SD
card, and the frame loop has no room for a database that flushes and compacts in bursts. A Pi
3B is enough, because Greptime publishes 64-bit ARM binaries and runs in a few hundred MB.

**Never part of the show.** The show reads no topic (§1.3), so losing Memory, its SSD or its Pi
costs the telemetry and never a frame. Everything on it reconnects for as long as it takes,
because both Pis power up together and `memory-pi` is up long before the access point.

**On the access point, at an address it sets itself**, from a profile rendered from the same
`clockwork.dot` as the access point's, so the two cannot disagree; routed out over 4G by
`claw-pi`. The Mac reaches it only by jumping through `claw-pi`: a tunnel of its own would buy
nothing, because a dead `claw-pi` leaves little to do remotely anyway.

**Six processes:**

- **Mosquitto**, Stem's broker (§3.1).
- **The health daemon**, `memory.py`, Stem's counterpart for `memory-pi`. It shares Stem's board
  readers through `stem/rig_board.py`, so a fix to a reading lands on both Pis, and adds the
  SSD's free space and its end of the WiFi link.
- **GreptimeDB**, standalone, from a pinned release with its SHA-256 in the repo, since no apt
  source carries it; on loopback alone.
- **The recorder**, `recorder.py`, writing every live message into Greptime over InfluxDB line
  protocol in five-second batches. Greptime has no MQTT listener, and its documented route is
  EMQX's rule engine, which would replace Mosquitto; a recorder is a page of Python.
- **The OSC bridge**, ingesting the OSC a fixture sends to `memory-pi` directly, decoded from
  that fixture's OSCQuery file (§1.4), alongside what the recorder takes off the broker, so a
  fixture that speaks no MQTT is recorded without the brain in the path.
- **Grafana**, from its apt repository, reading Greptime as a MySQL server, on loopback, its
  settings and data source provisioned from the repo. No dashboard is provisioned by default;
  one made in the UI lives in Grafana's own database.

**What is stored** is one table per first topic level, numbers and words in separate columns,
flags as 1 and 0 so they draw as a line, timed when the recorder received them, kept forever:
a few MB a day against 500 GB. The schema is in the README. **Not stored:** retained replays,
which would write each state again on every connect; empty payloads, which delete a retained
topic rather than say anything; and anything before the clock first synchronised, which would
be filed under the stale boot time.

**The broker comes first.** Greptime and Grafana run at a lower priority, Greptime's writes at
idle disk priority, and they are the first the kernel kills when memory runs out, so a
compaction or a heavy dashboard cannot take the broker down on a 1 GB Pi. They are not capped
in memory, because the Pi's kernel ships with the memory cgroup controller off and turning it
on is a boot parameter on a file the repo cannot own; Grafana, a Go program that otherwise
holds on to what it freed, gets a Go memory limit instead.

**The SSD carries the recordings only.** The Pi boots from its SD card and mounts the SSD with
`nofail` and a short device timeout, so a missing SSD costs the recordings and leaves the Pi up
and reporting why. Greptime requires the mount, so it never writes into the empty mount point.

**Power is cut, never shut down.** Greptime writes its log to the SSD every ten seconds, so a
power cut costs at most those seconds and what the recorder held: the recordings are read in
days, not seconds.

### 3.10 Wrist

A guy-wire drive that sways a structure at its own first natural frequency: an integrated
closed-loop stepper reeling the wire through a series spring, an accelerometer on the structure
and a load cell under the motor, all on one board that follows the universe and closes the sway
loop itself. Its firmware is C++ on the Arduino core for the ESP8266, which the chip forces
(§2.4); what it does, its pins and its constants are in [`CONTROL_LOOP.md`](CONTROL_LOOP.md).

**A fixture, not a motor on the Pi.** The universe is the one way anything on the rig is told to
move, which lets the e-stop, a takeover and signal loss cover the drive with no rule of their
own, and lets an artist session drive it from a fader. The brain sets only a mode and an
amplitude; Wrist renders the motion at its own rate, for the ball's reason: forty bunched
frames a second over WiFi cannot carry step pulses.

**The loop closes in the fixture.** Sensor, oscillator and motor share one board, so nothing in
the phase loop crosses the network: no WiFi latency or jitter to allow for, no shared clock, no
relay. The ESP8266 has no Bluetooth, so the accelerometer is wired to the board rather than
sending from a radio of its own. A local oscillator phase-locked to the sway times the drive;
the frame says only how much.

**A resonance pumped through a spring.** Rubber bands between the wire and the structure turn
motor position into force, so the motor drives the structure's own mode and the drive's phase
is the control; a stiff line would fight the structure instead. Every lag in the chain —
sensor, filter, motor, bands, structure — is absorbed in one phase offset that Wrist learns
while it runs, by seeking the largest sway at a fixed drive, rather than one computed or set by
hand. Frequency and Q come from the ring-down after a single tug, a pluck. What Wrist learns
persists across power cycles and is reported, so the coded-in defaults are updated by hand from
what it learned.

**Blackout is limp.** Mode's no-function band is Blackout: coils released, as if the motor's
power were cut, which is also Wrist's idle state. Every other fixture holds its last values on
loss, because a head that stays put is safe; a drive held under power against a swinging
structure is a tug-of-war with whatever else is happening. So on losing the universe Wrist
releases at once, the one fixture whose signal-loss behaviour is not "hold", and this paragraph
is why. A Blackout in the frame — the e-stop's mask included, which Wrist cannot tell apart
from a requested one — first brakes the sway under power, driving in antiphase for a bounded
time, then releases: a structure brought to rest is safer than one left swinging on a limp
wire, and while frames arrive the rig is there to brake with.

**Blackout means unhomed.** Released, the shaft can be turned by the wire, a hand or the
structure settling, and the motor's encoder is inside its driver where Wrist cannot read it. So
every exit from Blackout homes first, to a switch at the wire's pay-out end with the bands still
stretched, so the wire is never slack; then Wrist takes up whatever the frame asks.

**The alarm is the motor's only word.** An integrated closed-loop stepper reports nothing but a
latching alarm, which clears when the pulses stop or the power is cycled: no encoder output, no
in-position signal. Wrist's other evidence is its own — the load cell's tension, the home switch
closing mid-run on lost steps or a slipped wrap, a stale accelerometer, a move clamped at the
travel limits — and every protection acts in the fixture, where the show cannot override it by
not knowing.

**Step pulses from a timer NMI.** The ESP8266 has one core, shared with WiFi; pulses timed in
ordinary code would jitter with the radio's interrupts, so a timer at non-maskable priority
makes them from a velocity the control tick sets.

**Telemetry is OSC, out only.** No MQTT client: Wrist sends its loop variables and raw inputs
as OSC bundles, and its transitions, pluck results and protections as events. Nothing is
computed for telemetry; what can be derived is derived from the recording. The address space is
`fixtures/wrist.oscquery.json` (§1.4). Nothing comes in over OSC: calibration constants are
coded in, worked out from raw readings in the telemetry.
