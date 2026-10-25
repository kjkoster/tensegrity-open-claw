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

The workspace has two users: QLC+, which opens it to take the rig over and to aim presets,
and the build. **The running brain never reads it**: the build consumes it, and it never
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

**Scenes are pointing presets, and a prescribed set.** Where a head points is a measurement
taken on the rig with a console, so it lives in the workspace like the patch. The rig crate
lists the presets its show is written against, by their names in QLC+, and the workspace's
scenes must be exactly that set: a missing one is a look the show would not get, an extra one
a look nothing plays. The list is the code's requirement, not a copy of the workspace. Each
name becomes a constant the way a fixture name does, so a renamed preset fails at its use site.
The ingest resolves each preset at build time into a pose per head, in degrees, through the
head's own travel range — the inverse of what aiming does — so a preset is read as a pose and
never as bytes. A preset holds pan and tilt, coarse and fine, for every head and nothing
else: a missing channel would resolve to 0°, the end of the head's travel, and an extra one is
a value the show would never send. Both fail the build.

**The same rule runs past addressing into values.** A `.qxf` says a great deal more than
which slot a channel occupies: which value opens the shutter, which end of a speed channel is
the fast one, which capabilities of a colour wheel are single filters and which are split
positions, how far the head pans. Every one of those is a fact about the fixture, and every
one of them written into Rust is the same second copy this section exists to refuse — worse
than an address, because a wrong address is at least wrong on every fixture equally, while a
wrong speed polarity is wrong on one model and right on the next.

So the ingest keeps growing to read more of the definition, and the code asks the patch
rather than spelling a number out. Where the definition does not carry the fact yet — a gobo
channel with one undivided "fixed gobo" band, a strobe channel with no capabilities at all —
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

The engine keeps running while overridden — its noise fields, breath phase and slews stay
continuous — so handback resumes mid-stride rather than springing out of a frozen frame.

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

A part that senses the world publishes a snapshot of what it senses, and the state machines
take that snapshot as an argument each frame. Neither machine reaches into a part for
anything else, so every input each depends on is visible in its signature. Remote sensing,
when it comes, is another producer of the same kind of state — most likely over OSC.

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

Anything the show must survive losing runs outside the brain: Stem, and before it the
eyeball. A daemon that crashes, stalls or is restarted mid-tune then costs what it provides
and never a frame, and a process aiming moving heads at people does not share an address
space with anything that can take it down.

---

## 2. Structure

### 2.1 System overview

The rig is a **Raspberry Pi driving one DMX-512 universe of fixtures**. The Pi runs a Rust
program on Linux (`brain`) that reads the rig's sensors, runs a generative engine over them,
and clocks the resulting universe out of an RS-485 HAT at 40 Hz to the wired fixtures. The
same universe also goes out as E1.31 sACN over the network — wireless fixtures listen to it,
it is what a monitor or a console works against, and it is the path an external override
arrives on.

The Pi acts as a WiFi access point, and is reachable from the programming Mac over a
WireGuard tunnel — which is how both deployment and console traffic get there.

```
    ┌── Mac ──────────────────────────────────────────┐
    │  (SSH / deploy; QLC+ for override + presets)    │
    └───────────────── WireGuard ─────────────────────┘
                                  │
    ┌── Raspberry Pi ──────────────┴──────────────────┐
    │  (builds brain; reads the rig's sensors)        │
    │  Generative engine                              │
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
| `cortex/` | everything under the show: capture, features, noise, both transports, the frame loop, the show calendar |
| `cortex-build/` | the QLC+ ingest and the calendar's resolution, a build-dependency of the rig crate |
| `brain/` | the rig crate: the rig's patch, its show times, and its show |
| `rig-mqtt/` | the MQTT conventions the brain and the ball share |
| `ball-dmx/` | what the ball's DMX values mean where its `.qxf` cannot say, read by the ball firmware |

The rig crate's `build.rs` names its workspace and reads `shows.rs`, the show table; its
`main.rs` includes the generated patch and calendar and writes the show.

The ball firmware, `ball/`, is not a member of this workspace: it is built on the Mac for its
own target.

Beside the crates, a daemon's systemd unit lives in that daemon's directory
(`brain/brain.service`, `stem/stem.service`), and what gets installed onto the Pi without
belonging to a daemon lives in `etc/`: the access point's configuration, the 4G watchdog and
`remote-deploy.sh` itself.

### 2.4 Software stack

Standard Rust on Trixie Raspbian Linux, `std`.

- **Hand-rolled sACN E1.31 encoder** writing to a UDP socket. The `sacn` and
  `sacn-unofficial` crates were not a good fit; the packet is small enough that owning it
  costs less than working around them.
- **`zihatec-rs-485-dmx`** for DMX-512 framing — BREAK/MAB timing and full-512 padding. Only
  the device path is ours.
- **ALSA** for audio capture.
- **Embassy** (`executor-thread` + `embassy-time`) for the frame loop, with blocking
  producers — sensors, and the sACN receiver — on their own OS threads feeding it
  through a lock-free latest-value seam. A blocking `recv` on the executor's thread would
  stall the frame loop, so anything that blocks stays off it.

### 2.5 The rig state machine

The brain runs **two state machines, layered**, and keeps their states apart by name:

- The **rig state** — boot hold, running, e-stop, shutdown — says whether, and what, goes
  out. Its machine is in `cortex`'s frame loop, after the arbitration, and it is published on
  `brain/state/rig`.
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

The deployed unit is a **Raspberry Pi 3B** running standard Debian / Raspberry Pi OS. A Pi 4
has more headroom and runs the same software unchanged, if thermals or CPU ever justify it.

The Pi runs in **AP mode** (hostapd + dnsmasq) so fixtures and laptops join its own network
rather than depending on whatever WiFi exists at the site, exposes its **Ethernet port** to a
directly-connected laptop for SSH, and runs `brain` as a systemd service for unattended
operation.

Settings — SSID, serial console, DIP switches, `config.txt` — are in
[`README.md`](README.md).

---

## 3. Components

### 3.1 Stem

The backbone for telemetry: the MQTT broker, Mosquitto, and a health daemon in Python. All of
the rig's telemetry is read-only and goes through it (§1.3). It is separate from `brain`
(§1.6) so it stays alive and reporting when `brain` is down — which is precisely when health
matters — and, being Python and a broker rather than Rust, it has no place inside the brain
anyway.

**The broker belongs to Stem.** The transport the reports travel on is part of that job rather
than a service the brain happens to offer. It is Debian's Mosquitto, configured by an overlay
in its include directory (`stem/stem.conf`), so a package upgrade cannot quietly revert the
rig's settings. It keeps nothing on disk, and every client replays its retained topics on each
connect instead.

**The daemon is deliberately the least capable process on the rig**: it reads the hardware and
how it is holding up, publishes it, and does nothing else. Keeping duties out of it means its
death costs telemetry and never uptime.

Every client holds its own connection to the broker, announces itself on `health/<service>`,
and leaves a will that says `gone`: `brain`, the daemon, the `face` and the `ear` (each its own
client, inside the brain process) and the `ball` (over the AP, to the DHCP gateway). The Rust
clients share their packets and the health words through the `rig-mqtt` crate;
`stem/rig_mqtt.py` is the Python half.

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
claim does not prove, and on a ganged hub it would also cut the audio interface.

### 3.3 The face

The cabinet door — e-stop, two buttons, a green lamp in each — is wired to the Pi's GPIO, but
it is treated like any fixture in both directions rather than as a side channel.

**Its lamps are patched.** `fixtures/face.qxf` gives them two dimmable channels, and the
workspace patches them like the ball. The show writes them into the frame; the face drives
the pins from the frame that actually went out, after arbitration. So a console that takes the
rig over drives the lamps too, and the e-stop darkens them with everything else, with
no special case in either. They dim through the Pi's two hardware PWM channels, because the
software alternative is a busy-waiting real-time thread per pin, competing with the ear and
the DMX timing.

**Its inputs are a `FaceState`.** The face polls and debounces, and publishes the e-stop,
chord and press counts, and the right button held, once per poll. The show state machine
reads only the buttons; the e-stop goes to the rig state machine (§2.5).

### 3.4 The ear

The rig's microphone input is the ear: capture, feature extraction, and the `AudioFeatures`
it hands the show. It has no DMX channels and no place in the workspace, but it is a part of
the rig the way the face is — its own thread, its own state handed to the show, and its own
client on the broker, so a dead or missing interface shows up as `ear/…` rather than as a
show that has gone quiet for no visible reason.

The interface itself is incidental — today an Alesis io|2, because it was the cheapest used
one to be had — so the code names it by role. Swapping it is its ALSA card name and USB
identity in `cortex`'s config, nothing more.

### 3.5 The ball

An ESP32-C3 on the Pi's WiFi, following the sACN universe. The firmware is `no_std` Rust on
Embassy.

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

A vision sidecar: an Axis M3105-LVE network camera in, arm angles out, and an annotated
preview for humans. It drove the mage rig, and was removed with it; the code is on the
`last-version-with-mage` tag (`eyeball/` and `cortex/src/eyeball.rs`).

It ran as its own Python process (§1.6), on OpenCV and numpy, with a MoveNet pose model on
TFLite or ONNX. The model got three of the Pi's cores, leaving one for the DMX loop.

**Three outputs, three audiences.** Landmarks went to `brain` over UDP on the loopback,
because a late pose frame is worthless and dropping it beats delivering it in order; silence
and a wedged sender are then the same observation, which is what the brain's staleness window
was for. The annotated preview went to a browser over HTTP, because that is what a human uses
to aim a camera and calibrate a crop — and it was shown to the audience, so the drawn skeleton
was a deliverable, not a debug view. Health went to MQTT.

**The brain read arms, not joints.** The daemon reduced the landmarks to arm angles and kept
the landmarks on its own side, where the preview drew them. The estimator was expected to be
replaced — contour extremities under infrared, say — and a schema naming the seventeen joints
of one model would have had to change with it.

**The camera was configured by the daemon, not by hand.** The host, user and password were
the only camera settings in its configuration; which stream, at what size and rate, and the
sensor's settings were the daemon's to decide. It set the camera's parameters over VAPIX on
every start, read them back, and published any disagreement, because a setting made by hand
in the camera's web UI is lost on a factory reset or a replacement camera. Infrared beat
visible light by a wide margin, so it ran the IR-cut filter off under the camera's own
illuminator, with exposure capped at 10 ms so a moving wrist was a landmark and not a streak.
MJPEG has no inter-frame state or jitter buffer, which made it the first thing to try against
H.264 when the preview ran late.

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
