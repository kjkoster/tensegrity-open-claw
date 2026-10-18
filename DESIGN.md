# Tensegrity Open Claw — Design Document

**What this file is for.** Architectural decisions and their reasoning, plus work that is
not built yet. It deliberately does *not* describe what the code currently does — the code
says that better — and it does not restate fixture addressing, which lives in the QLC+
workspace and changes too often to mirror in prose.

[`README.md`](README.md) is the operator manual: everything you set on the Pi or on live
hardware.

---

## 1. System overview

The rig is a **Raspberry Pi driving one DMX-512 universe of wired fixtures**. The Pi runs a
Rust program on Linux (`brain`) that captures audio, runs a generative engine over it, and
clocks the resulting universe out of an RS-485 HAT at 40 Hz. The same universe also goes out
as E1.31 sACN over the network — the ball listens to it, it is what a monitor or a console
works against, and it is the path an external override arrives on.

The Pi acts as a WiFi access point, and is reachable from the programming Mac over a
WireGuard tunnel — which is how both deployment and console traffic get there.

```
    ┌── Mac ──────────────────────────────────────────┐
    │  (SSH / deploy; QLC+ for override + scenes)     │
    └───────────────── WireGuard ─────────────────────┘
                                  │
    ┌── Raspberry Pi ──────────────┴──────────────────┐
    │  (builds brain; the ear: local audio capture)   │
    │  Generative engine                              │
    │  ├── RS-485 HAT (40 Hz) ──── wired DMX-512      │
    │  └── sACN sender ──────────── network           │
    └─────────────────────────────────────────────────┘
                                  │ wired DMX
                        one universe of fixtures
                    (see claw.qxw for the patch)
```

The fixtures are one wash and one moving head per leg, the ball on the network,
and the cabinet door's two lamps — the face, a fixture inside the brain process (§2.7).

---

## 2. Decisions

### 2.1 One universe out, and only one

The daemon emits exactly one universe. Multi-universe output is **explicitly not supported**
— the ingest rejects any fixture patched outside the workspace's first universe rather than
silently dropping it.

This is a deliberate ceiling, not an oversight. A second universe buys nothing while every
fixture fits in 512 slots, and it would force a routing layer between the engine and the
wire that has no other reason to exist. If rented base lights ever need their own universe,
that is the moment to build the routing layer — not before.

### 2.2 Transport: wired DMX primary, sACN in parallel

**Wired DMX-512 out of the RS-485 HAT is what drives the rig.** sACN (E1.31) carries the
same universe over the network in parallel. It has no fixtures listening to it, but it is
kept because it costs nothing, it lets a monitor or a console see what the daemon is doing,
and it is the path an external override arrives on.

Art-Net is explicitly not supported: we control every device on the network, so
compatibility with older rigs is not a requirement.

**The wired frame is padded to a full 512-slot universe.** A short frame is a known way to
lose cheap DMX receivers: they ignore the data and fall back to their own internal show
while still reporting a healthy signal, which reads as a wiring fault and is not one. The
padding costs a fixed 512 slots per frame, which is what sets the ~43 Hz ceiling the 40 Hz
frame rate sits under.

### 2.3 The QLC+ workspace is the patch

**None of the fixture addressing is written in Rust.** A rig's QLC+ workspace — the claw's is
`claw.qxw` — is the source of truth for *where* its fixtures sit, and the `.qxf` definitions
committed in `fixtures/` are the source of truth for *what their channels mean*. The build
ingests both and generates a struct per fixture, with a named field per channel, into that
rig's own crate.

**The universe and the sACN port come from the same file**, off the E1.31 output patched to
the workspace's first universe. That output describes the console's own transmission, but a
rig has one universe and relays a takeover onto it, so those two numbers are the rig's as
well: what the brain listens for, what it sends, and what a ball listens for. The address
that output unicasts to is the one the brain's receiver binds.

**A ball is a fixture in that patch like any other**, and its firmware is built from the
workspace too. A ball is whatever the workspace patches through `fixtures/ball.qxf`: the
firmware build turns every one into a constant carrying its address, and reads the channel
layout and bands out of the definition; a board is provisioned by naming the one it is.

The workspace is strictly a **build artifact**: it is consumed at build time and never crosses
into the running program, not even as a filename. A rig has exactly one, so naming the rig in
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

### 2.4 External takeover: strict priority, whole universe

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

The cabinet's e-stop is part of the **show**, not of the arbitration: a console that takes
over replaces the frame, e-stop included, and what it sends is the operator's
responsibility. While the e-stop is pressed, or its two contacts agree (a broken cable or
a contact fault), the show is in its e-stop scene, which beats every other scene and test
mode: every slot zero, at the usual rate and priority, so nothing downstream sees a lost
stream. Being a scene, it is reported as one, on `brain/scene`. The show runs the ball's motor on a beat (§2.6), and
a run starts only at the head of its window, so releasing the e-stop partway through one
waits for the next rather than running the remainder. The face's two buttons drive only test
mode (§2.6), which runs the motor only when asked.

The receiver binds the address the workspace's console unicasts to — the Pi's WireGuard
address — alone rather than the wildcard. That is not
just narrowing: it is also what stops the brain's own multicast being delivered back to its
own receiver.

### 2.5 Software stack

Standard Rust on Trixie Raspbian Linux, `std`.

- **Hand-rolled sACN E1.31 encoder** writing to a UDP socket. The `sacn` and
  `sacn-unofficial` crates were not a good fit; the packet is small enough that owning it
  costs less than working around them.
- **`zihatec-rs-485-dmx`** for DMX-512 framing — BREAK/MAB timing and full-512 padding. Only
  the device path is ours.
- **ALSA** for the ear's capture.
- **Embassy** (`executor-thread` + `embassy-time`) for the frame loop, with blocking
  producers — the ear, and the sACN receiver — on their own OS threads feeding it
  through a lock-free latest-value seam. A blocking `recv` on the executor's thread would
  stall the frame loop, so anything that blocks stays off it.

### 2.6 Show times and the ball show

The rig runs to a fixed list of shows in local time, one scene at a time, chosen each frame
by one state machine: the e-stop scene (§2.4) first, then test mode's, then the calendar's.
Outside a show it is in blackout: black but for the ball's status LED, which keeps breathing
so a ball that hears the rig can be told apart from one that does not. Inside one, the
ball's motor runs on a beat counted from the show's start; until civil dusk that is the
daylight scene, and after it the ball's power LEDs pulse in silence and strobe with the
music. In every scene but blackout and e-stop, each wash holds its own colour — red, green or
blue — with white beside it, and one sparkle modulation drives all three, so the rig breathes
in silence and flashes on the music's onsets in step rather than three ways at once.

**All of the time work happens at build time.** The table names a city and lists local
start/stop pairs; the build turns that into coordinates, a zone, UTC instants with their
offsets, and the civil dusk-to-dawn stretches inside each show. The brain only ever sees a
complete set of timestamps — no time-zone database, no solar arithmetic, nothing that can
disagree between the Mac and the Pi at run time. It also makes the hard cases build errors
rather than show-night surprises: a show night can straddle the autumn clock change, and a
local time in the hour that happens twice, or in the hour that never happens, has no single
meaning.

**Civil dusk, not sunset,** because the light reads only once it is actually dark.

**The brain does not wait for NTP.** The Pi has no RTC and the 4G link can take a minute.
After a reboot the restored clock is behind, never ahead — by the reboot on a clean restart,
by the outage plus up to an hour on a power cut — and behind is the safe direction: the
startup listing keeps too much rather than dropping a show to come, and the per-frame
decision corrects itself when NTP steps the clock.

**The ball draws its own waveforms.** At 40 Hz a frame is 25 ms, longer than the pulse's
hold, and multicast over WiFi arrives bunched. So the brain sends a pattern and its
parameters, and the ball renders them once per period of its 500 Hz PWM. The brain still
modulates the strobe live, through the flash and gap lengths. Those lengths are a unit a
`.qxf` cannot express, so it lives in `ball-dmx`, the one crate both sides read it from.
Everything the `.qxf` can express — which channel sits where, and the bands of the motor and
the pattern — the firmware's build reads out of the definition, as §2.3 asks.

**Test mode is on the door, not on a console.** Testing a scene should not wait for dusk,
for silence, or for a laptop on the network, so the face's two buttons can take the scene
over: a two-second chord toggles test mode, and the buttons then step the scene and run the
motor for as long as one is held. A step is counted on release, which is what tells a lone
press from half of a chord. The motor runs only when asked, so stepping scenes cannot start it by
surprise, and test mode ends itself after fifteen quiet minutes, because a rig left in it
would ignore the calendar unattended.

### 2.7 The face is a fixture

The cabinet door — e-stop, two buttons, a green lamp in each — is wired to the Pi's GPIO, but
it is treated like any fixture in both directions rather than as a side channel.

**Its lamps are patched.** `fixtures/face.qxf` gives them two dimmable channels, and the
workspace patches them like the ball. The show writes them into the frame; the face drives
the pins from the frame that actually went out, after arbitration. So a console that takes the
rig over drives the lamps too, and the e-stop scene darkens them with everything else, with
no special case in either. They dim through the Pi's two hardware PWM channels, because the
software alternative is a busy-waiting real-time thread per pin, competing with the ear and
the DMX timing.

### 2.9 The ear is a part, not a fixture

The rig's microphone input is the ear: capture, feature extraction, and the `AudioFeatures`
it hands the show. It has no DMX channels and no place in the workspace, but it is a part of
the rig the way the face is — its own thread, its own state handed to the show, and its own
client on the broker, so a dead or missing interface shows up as `ear/…` rather than as a
show that has gone quiet for no visible reason.

The interface itself is incidental — today an Alesis io|2, because it was the cheapest used
one to be had — so the code names it by role. Swapping it is its ALSA card name and USB
identity in `cortex`'s config, nothing more.

**Its inputs are a state, handed in.** The face polls and debounces, and publishes a
`FaceState` — e-stop, chord and press counts, the right button held — once per poll. The
show's state machine takes that snapshot as an argument each frame and reads nothing else of
the face's, so every input it depends on is visible in its signature. Remote sensing, when it
comes, is another producer of the same kind of state — most likely over OSC.

### 2.8 MQTT is telemetry, never show

Every topic on the broker reports; nothing the show does reads one. The show's inputs are
the audio, the door and the clock, and a future remote input arrives as state handed to the
show, not as a subscription. That keeps the broker optional — a broker that is down, slow or
restarting costs the telemetry and never a frame — and keeps the show's behaviour explainable
from its inputs alone.

---

## 3. Hardware

### 3.1 Raspberry Pi controller

The deployed unit is a **Raspberry Pi 3B** running standard Debian / Raspberry Pi OS. A Pi 4
has more headroom and runs the same software unchanged, if thermals or CPU ever justify it.

The Pi runs in **AP mode** (hostapd + dnsmasq) so fixtures and laptops join its own network
rather than depending on whatever WiFi exists at the site, exposes its **Ethernet port** to a
directly-connected laptop for SSH, and runs `brain` as a systemd service for unattended
operation.

Settings — SSID, serial console, DIP switches, `config.txt` — are in
[`README.md`](README.md).

---

## 4. Not built yet

Each section below is one chunk of work that lands as a whole.

### 4.1 Telemetry: MQTT and Stem

Read-only telemetry over a local MQTT broker (§2.8). `brain` publishes what it is and what the
show is doing; **Stem**, a separate health daemon, stays alive and reporting when `brain` is
down — which is precisely when health matters.

Five clients each hold their own connection, announce themselves on `health/<service>`, and
leave a will that says `gone`: `brain`, Stem, the `face` and the `ear` (each its own client,
inside the brain process) and the `ball` (over the AP, to the DHCP gateway). The Rust clients share their
packets and the health words through the `rig-mqtt` crate; `stem/rig_mqtt.py` is the Python
half. The broker keeps nothing on disk, and every client replays its retained topics on each
connect instead.

Written but not yet run on the rig: the `face`, `ear` and `ball` clients, the brain client's
retained-topic replay, and the broker without persistence. The ball's `disconnected` is only
reachable from the deep sleep it does not have yet.

Stem also absorbs the 4G watchdog that currently lives as an unversioned script on the Pi,
so there is one health daemon rather than two overlapping ones.

### 4.2 Installation cabinet

Encloses the Pi and all support hardware for a weatherproof, unattended outdoor deployment
lasting up to 7 days.

#### Bill of materials

Prices are indicative only and must be verified at purchase.

| # | Item | Qty | Notes |
|---|------|----:|-------|
| 1 | backup microSD (32 GB) | 1 | Cheap cards corrupt under days of writes |
| 2 | Pi power supply (official 5 V / 3 A or quality DC-DC) | 1 | Separate from fixture supply |
| 3 | Vent membrane / desiccant pack | 1 | Condensation management in sealed outdoor box |
| 4 | chunky power lead | 1 | leading into the powerCon |
| 5 | 3-way connector | 1 | for the various 220 V connectors internally |

#### Assembly

- [ ] Fit the pass-through 220 V to the cabinet.
- [ ] Remove the 48 V power supply from the cabinet.
- [ ] Plug the keyhole and previous mounting holes (seal the unused keyhole opening for
      weatherproofing).
- [ ] Design — and if needed add — drainage and ventilation holes (condensation management;
      see the vent membrane / desiccant in the BOM).
- [ ] Tidy up the internal cable bundles.
- [ ] Design and implement mounting to the Tensegrity sculpture.

#### Software service

- **Protect the SD card:** read-only root (overlayfs) or at minimum log to a RAM ring
  buffer. Days of writes to a writable root is a classic multi-day-install failure.
- Verify enclosure thermals in direct sun; throttling surfaces first as audio xruns.
- Condensation: sealed boxes sweat; the vent membrane and desiccant prevent internal dew.
- Temperature checking.

#### Acceptance

- Powers up into the running piece unattended after a cold boot.
- Runs the full deployment window (≤7 days) without intervention, SD corruption, or thermal
  shutdown.

### 4.3 Show times, the ball show and the face

The design is §2.6 and §2.7. Written but not yet run on the rig: the calendar, the scenes,
test mode, the face as a fixture with its PWM lamps and its `FaceState`, the ball's pulse and
strobe, and the brain's `scene` and `testmode` topics.

1. **Flash-rate cap.** The strobe's flash and gap ranges reach 12.5 Hz, inside the 3–30 Hz
   band that can set off photosensitive seizures. Settle whether the gap needs a floor that
   keeps the strobe below 3 Hz; the ranges carry a `TODO` until then.

---

## 5. Scrub the old home-WiFi credentials from git history

The old home-network SSID and passphrase are committed across history, in files that have
since been removed from `HEAD`. Deleting the files does not remove the strings from history,
so this task stands: rewrite history to replace them everywhere, then force-push.

The literal strings are deliberately **not** written here — this document would then be the
thing that needs scrubbing. Read them out of any pre-scrub commit, or off the old network
hardware, at the time you do the work.

**Destructive — coordinate; every clone must be re-cloned afterwards.**

- [ ] Back up first: `git clone --mirror <repo> backup.git`.
- [ ] Confirm the strings are absent from `HEAD` before starting, so the rewrite only has
      history to fix: `git grep -I <old-ssid> HEAD` must come back empty.
- [ ] `sudo apt install git-filter-repo` (packaged on Trixie).
- [ ] Write `replacements.txt` mapping each old value to its replacement, one
      `old==>new` pair per line. **Do not commit this file** — it is exactly the secret you
      are removing. Keep it outside the work tree.
- [ ] `git filter-repo --replace-text /path/to/replacements.txt --force`
- [ ] Re-add the remote (filter-repo drops it) and
      `git push --force --all && git push --force --tags`.
- [ ] Shred `replacements.txt`, re-clone on every machine, and delete stale clones and the
      mirror backup once verified.
