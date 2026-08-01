# Tensegrity Open Claw — Design Document

**What this file is for.** Architectural decisions and their reasoning, plus work that is
not built yet. It deliberately does *not* describe what the code currently does — the code
says that better — and it does not restate fixture addressing, which lives in the QLC+
workspace and changes too often to mirror in prose.

- [`README.md`](README.md) — the operator manual: everything you set on the Pi or on live
  hardware.
- [`SHOW.md`](SHOW.md) — the show: what the sculpture actually does, and the generative
  pipeline that will drive it.
- [`MAGE.md`](MAGE.md) — the second rig on the same cabinet, and the crate split it forced.
- [`PET-THE-DOG.md`](PET-THE-DOG.md) — watchdog layering for unattended operation.
- [`TODO.md`](TODO.md) — the empirical tests that settle the constants these designs assume.

---

## 1. System overview

The rig is a **Raspberry Pi driving one DMX-512 universe of wired fixtures**. The Pi runs a
Rust program on Linux (`brain`) that captures audio, runs a generative engine over it, and
clocks the resulting universe out of an RS-485 HAT at 40 Hz. The same universe also goes out
as E1.31 sACN over the network — no fixture listens to it, but it is what a monitor or a
console works against, and it is the path an external override arrives on.

`brain` is a symlink, not a binary: the cabinet drives two sculptures and one of them is not
audio-driven at all. Everything this document says about the show below is the claw's;
[`MAGE.md`](MAGE.md) covers the other rig and what the two hold in common.

The Pi acts as a WiFi access point, and is reachable from the programming Mac over a
WireGuard tunnel — which is how both deployment and console traffic get there.

```
    ┌── Mac ──────────────────────────────────────────┐
    │  (SSH / deploy; QLC+ for override + scenes)     │
    └───────────────── WireGuard ─────────────────────┘
                                  │
    ┌── Raspberry Pi ──────────────┴──────────────────┐
    │  (builds brain; local audio capture)            │
    │  Generative engine                              │
    │  ├── RS-485 HAT (40 Hz) ──── wired DMX-512      │
    │  └── sACN sender ──────────── network           │
    └─────────────────────────────────────────────────┘
                                  │ wired DMX
                        one universe of fixtures
                    (see claw.qxw for the patch)
```

The fixtures themselves — one wash and one moving head per leg, plus the pinspot — are
described in [`SHOW.md`](SHOW.md), which is where the reasoning about what each one is *for*
belongs.

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
that wanted it. Each of those measurements is a [`TODO.md`](TODO.md) item, and landing one
makes the code that reads it work on every fixture sharing the definition at once.

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

The receiver binds the Pi's WireGuard address alone rather than the wildcard. That is not
just narrowing: it is also what stops the brain's own multicast being delivered back to its
own receiver.

### 2.5 Software stack

Standard Rust on Trixie Raspbian Linux, `std`.

- **Hand-rolled sACN E1.31 encoder** writing to a UDP socket. The `sacn` and
  `sacn-unofficial` crates were not a good fit; the packet is small enough that owning it
  costs less than working around them.
- **`zihatec-rs-485-dmx`** for DMX-512 framing — BREAK/MAB timing and full-512 padding. Only
  the device path is ours.
- **ALSA** for audio capture.
- **Embassy** (`executor-thread` + `embassy-time`) for the frame loop, with blocking
  producers — audio capture, and the sACN receiver — on their own OS threads feeding it
  through a lock-free latest-value seam. A blocking `recv` on the executor's thread would
  stall the frame loop, so anything that blocks stays off it.

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

Each section below is one build: a chunk of work that lands as a whole. They are numbered in
the order they were written down, which is not the order they get built in — §4.2 gives that.

### 4.1 Drive the moving heads

The claw's heads are patched and its daemon does not touch them. Everything below assumes
they move, so this comes first.

What it takes is no longer a build of its own. The mage rig needed the same machinery sooner
and it landed in `cortex`: pan and tilt as 16-bit pairs, travel and park values read from the
definitions, a rate limiter in degrees per second with the floor these heads impose. So this
is the claw's show reaching for what already exists, plus the parts that are the claw's
alone — the aim points of §4.8, and the signal-loss blackout confirmed on the fixtures
themselves.

It still sits after §4.6, because the repatch moves every address the show would name.

### 4.2 The show pipeline

The generative engine today is a single mapping from audio features to colour. The design
that replaces it — moods, behaviours, verbs, and the eight-stage pipeline from capture to
output — is [`SHOW.md`](SHOW.md). That document supersedes the fixture-agnostic
engine/renderer split this file used to describe.

The pipeline's eight stages are not eight builds. Stages 1, 2 and 8 are built and stage 7
already has its vocabulary generated, so nothing upstream needs faking; and the stage
boundaries do not line up with the boundaries of inspectable progress — stages 5, 6 and 7
show nothing until all three exist, while stage 4 is nine behaviours. The builds are cut
along what can be seen and judged instead.

Two things run through all of them. **The intent frame is the only seam.** Once it is a real
type, every build after it is a question of who writes it: a constant, then a verb, then
home, then one behaviour, then nine, then the mood estimator. There is no stand-in per stage,
only a `bringup` mode of `brain` that hand-authors an intent frame and survives afterwards as
the bench and rental-verification tool. `sparkle.rs` is what a per-stage stand-in becomes when
it is allowed to live, and it is deleted rather than extended. **Telemetry is the instrument,
not the payoff.** It goes in early, publishing what already exists, and each build after it
adds its own topic as part of that build.

That gives the order: §4.6 repatch, §4.3 telemetry (its `brain` half), §4.1 the heads, §4.7
intent and drivers, §4.8 aim points, §4.9 verbs, §4.10 home and modulation, §4.11 the state
machine, §4.12 the remaining behaviours, §4.13 mood. §4.3's Stem half comes last of all —
independent of the show by construction, which is exactly why the rest of the telemetry can
sit at the front while Stem sits at the back.

### 4.3 Telemetry: MQTT and Stem

Read-only telemetry over a local MQTT broker, published by two processes: `brain` for the
show pipeline and **Stem**, a separate health daemon that stays alive and reporting when
`brain` is down — which is precisely when health matters. Topic tree and rates are in
[`SHOW.md`](SHOW.md).

Stem also absorbs the 4G watchdog that currently lives as an unversioned script on the Pi,
so there is one health daemon rather than two overlapping ones.

The two halves split across the order in §4.2: `brain`'s topics go in early, because they are
what makes the builds after them inspectable, while Stem goes in last. The early half touches
`capture.rs` and the audio pipeline, which are also among the files still citing
`SPARKLE.md`, `SOUND.md` and `HARDWARE-DMX.md` — none of which exist. Those references come
out in the same sitting, along with the ones left in `clock.rs`, `dmx.rs`, `latest.rs`,
`sparkle.rs` and the `config.rs` section headers: a comment says why its code is there, and
pointing at a design file that may be renamed or deleted is not that.

### 4.4 Watchdog

Three layers — behaviour exception, process, SoC — each catching only what the layer above
cannot see. Designed in [`PET-THE-DOG.md`](PET-THE-DOG.md).

### 4.5 Installation cabinet

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

Commissioning is a test procedure, so it lives in [`TODO.md`](TODO.md).

### 4.6 Repatch to per-leg blocks

Not code — the open item at the top of [`TODO.md`](TODO.md): legs at 10, 60 and 100, and the
Yaras switched from 4CH to 11-channel mode. Every build that names a fixture by leg rests on
this one, so it goes first; done later it moves every address out from under finished code.
Afterwards the workspace is the only record, which is what retires the TODO item.

### 4.7 The intent frame and the drivers

Stages 5 and 7 of [`SHOW.md`](SHOW.md), built as one chunk because neither shows anything
alone: an intent frame with no driver reaches no fixture, and a driver with no intent has
nothing to translate. Together they are the seam everything above plugs into, and the first
thing to write an intent frame is the `bringup` mode by hand.

Position is carried in **degrees, unreferenced**: zero is wherever the fixture's own zero
happens to fall, which depends on how that head is clamped and how the tripod sits that
night. Nothing needs an absolute origin — the recorded point table of §4.8 is the absolute
reference, and it is re-recorded every setup — so the origin is free to be arbitrary. The
scale is not. Every motion constant above stage 7 is a delta or a rate: the per-frame ceiling
for `climb`, how fast `whip` tears across, the wander radius, the smallest move that still
reads as a `jump`. In raw DMX values those are silently model-specific, so a rental with a
wider range runs the same code at a different apparent speed while still compiling and still
running. In degrees they transfer, which is what makes "a rental is a new driver and nothing
above it changes" true rather than merely stated.

The per-model range is not a hand-written constant either. `PanMax` and `TiltMax` sit in the
committed `.qxf` beside the channel definitions, so §2.3 covers them unchanged — a fixture
whose definition declares no travel gets no position at all, the same way one with no red
channel gets no `red` field.

### 4.8 Named aim points

The twelve points per head from [`SHOW.md`](SHOW.md), and the `bringup` recorder that
produces them. Thirty-six pan/tilt pairs is too many to capture by editing constants and
rebuilding, and the table is re-recorded every setup and every rental — so the recorder is
not scaffolding for this build, it is the only way the table is ever filled in.

### 4.9 Verbs

Jump, whip, carry, climb, chop and flash, each runnable on its own from `bringup`. They are
their own build rather than part of the behaviours that use them because this is where the
bench measurements land: a verb that reads wrong is one constant in one place, while the same
fault found inside a behaviour means guessing which of several constants caused it.

### 4.10 Home and modulation

Stage 6, and with it the home state: the sculpture breathes. Home is the majority of what the
audience sees and the state every failure resolves to, so from here on a half-built show
degrades into something finished rather than into darkness.

`sparkle.rs` is deleted here rather than extended. It is a stand-in for stages 5 and 6 that
was allowed to live, and keeping it would leave one fixture permanently outside the pipeline;
its slew limiters and its stale-input-reads-as-silence crossfade move into modulation and the
drivers.

### 4.11 The show state machine

Stage 4, carrying exactly one behaviour: hub-and-spoke, jittered home dwell, weighted
selection over a pool of one, and abort to home on any exception. Climb is the gentlest first
behaviour — slow enough that a wrong constant is legible rather than merely ugly, and it uses
only two of the twelve points, so it does not wait on the whole table being good.

### 4.12 The remaining behaviours

The other eight, one at a time, relaxed pool first and climax last. Going in mood order means
a partial pool is still a coherent show at every point along the way, and leaving the climax
behaviours until the end is deliberate: the detonations need the restraint rules, the
specular safety walk in [`TODO.md`](TODO.md), and an audience to judge them against.

### 4.13 Mood

Stage 3, last. Its pools mean nothing until the behaviours exist, and it is the one stage
with no visible output at all — the mood topics of §4.3 are the only way to watch it work,
which is why the telemetry build sits at the front rather than the back.

Two halves. The continuous parameter vector is half-wired already by §4.10, which drives
modulation from energy; completing it is giving it the full vector and the long time constant
that turns estimator flaps into course corrections. The discrete mood id — tempo bands,
corroborating energy, hysteresis — is the other half, and is what stage 4 reads at home.

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
