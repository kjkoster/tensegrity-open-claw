# Tensegrity Open Claw — Design Document

## 1. System Overview

The rig is a **Raspberry Pi driving one DMX-512 universe of wired fixtures**. The Pi runs a
Rust program on Linux (`brain`) that captures audio, runs a generative engine over it, and
clocks the resulting universe out of an RS-485 HAT at 40 Hz. The same universe also goes out
as E1.31 sACN over the network — nothing consumes it today, but it is what a monitor or a
console works against, and it is the path an external override arrives on.

The Pi acts as a WiFi access point and is reachable from a development laptop over Ethernet.

```
    ┌── Laptop ───────────────────────────────────────┐
    │  (SSH / deploy; QLC+ for override + scenes)     │
    └────────────────── Ethernet / WiFi ──────────────┘
                                  │
    ┌── Raspberry Pi ──────────────┴──────────────────┐
    │  (builds brain; local audio capture)            │
    │  Generative engine                              │
    │  ├── RS-485 HAT (40 Hz) ──── wired DMX-512      │
    │  └── sACN sender ──────────── network           │
    └─────────────────────────────────────────────────┘
                                  │ wired DMX
                          ┌───────┴────────┐
                     ┌────┴─────┐    ┌─────┴──────┐
                     │  Laser   │    │ 3× Yara    │
                     │  @25     │    │ @100/107/113│
                     └──────────┘    └────────────┘
```

### 1.1 DMX layout

One universe, all of it wired.

| Slots | Fixture | Mode | Channels |
|------:|---------|------|----------|
| 1–5 | HQ Power VDPLPS36B2 pinspot | 5-channel | Effect, Red, Green, Blue, Speed |
| 25–32 | JB Systems Space-4 laser | 8-channel | Mode, Pattern, Zoom, Y/X/Z roll, X/Y move |
| 100–103 | Yara 1 | 4-channel | Red, Green, Blue, White |
| 107–110 | Yara 2 | 4-channel | Red, Green, Blue, White |
| 113–116 | Yara 3 | 4-channel | Red, Green, Blue, White |

The frame spans slots 1–116 (`DMX_SLOTS`); every slot outside a fixture's block stays zero.
The wired frame is padded to a full 512-slot universe — a short frame makes the laser ignore
the data and free-run its own show (see §3.5 and README).

**None of this addressing is written in Rust.** The QLC+ workspace (`open-claw.qxw`) is the
source of truth for *where* fixtures sit, and the `.qxf` definitions committed in
`fixtures/` are the source of truth for *what their channels mean*. `brain/build.rs` reads
both, resolves each patched fixture against its definition's mode, and generates a struct
per fixture into `patch.rs` — a named field per channel, `DMX_SLOTS` included.

Because channels become *named fields* rather than offsets, drift is caught by the compiler
in every direction. Patching a fixture nothing drives leaves an unused constant (a dead-code
warning). Deleting or renaming one that code drives removes the constant its use sites need.
Repatching to a mode that lacks a channel the code writes removes the field, or stops the
fixture implementing the capability trait the code requires — so the sparkle engine can only
be wired to a fixture that genuinely has red, green and blue. And a workspace hand-edited
into disagreeing with its definition is caught by a channel-count check at generation time.
There is no drift checker because none is needed.

Every definition a patched fixture names must be committed: the Pi builds the daemon and has
no QLC+ library to fall back on, so a missing `.qxf` fails the build outright.

### 1.2 Yara pars (CLF-Lighting, wired, RGBW)

Three CLF-Lighting Yara LED pars at addresses **100, 107, and 113** in **4-channel mode**
(`Red, Green, Blue, White` — see `reference/Manual-CLF-Yara-1.0.pdf`). There is no dimmer
channel in this mode, so a full colour channel is full output. The fixtures' front-panel
default is 11CH; each par must be set to 4CH by hand, and nothing in software can detect it
being wrong.

For bring-up they are pinned to hard primaries (red, green, blue) so the three are trivially
distinguishable and their addressing is verifiable.

**Pending:** the generative engine in `brain/src/sparkle.rs` — silence breathing, music
glints, colour drift — currently has no consumer, because the fixtures it drove have been
removed. It is retained to be repointed at the Yaras, one mapping instance per par with its
own seed group. The Yara is RGBW, so the colour side maps across unchanged; the engine's gobo
output has no counterpart and is dropped.

---

## 2. Hardware

### 2.2 Raspberry Pi controller

A Raspberry Pi 4 (2 GB is ample; Pi 3B+ is a cooler-running alternative) running standard
Debian / Raspberry Pi OS. The Pi:

- Runs in **AP mode** (hostapd + dnsmasq) — fixtures join its WiFi network; fixed SSID and
  channel per deployment.
- Exposes its **Ethernet port** to a development laptop for SSH and cross-compilation.
- Runs the **Rust sACN sender** as a systemd service for unattended operation.

---

## 3. Design Principles

### 3.1 Transport: sACN over WiFi

**Wired DMX-512 out of the RS-485 HAT is what drives the rig.** sACN (E1.31) carries the same
universe over the network in parallel. It has no fixtures listening to it today, but it is
kept because it costs nothing, it lets a monitor or a console see what the daemon is doing,
and it is the path an external override arrives on.

Art-Net is explicitly not supported: we control all devices on the network, so compatibility
with older rigs is not a requirement.

### 3.3 Software stack

**Pi controller:** standard Rust on Trixie Raspbian Linux, `std`. Hand-rolled sACN E1.31
encoder writing to a UDP socket — the `sacn` and `sacn-unofficial` crates were not a good
fit. DMX-512 framing (BREAK/MAB timing, full-512 padding) comes from the
`zihatec-rs-485-dmx` crate. ALSA for audio capture. Embassy (`executor-thread` +
`embassy-time`) for the frame loop, with blocking producers — audio capture, and the sACN
receiver — on their own OS threads feeding it through a lock-free latest-value seam.

### 3.4 Power and signal separation

DMX cables are signal-only by convention; no competent engineer will patch an unknown cable
carrying power into their desk. Therefore:

- **At the hiuse console interface:** standard 5-pin XLR (signal only) + a separate power feed
  (mains into the ground station), probaby 3-pin for flexibility.
- **From ground station to the installation:** a custom hybrid cable carrying power alongside
  the DMX pair — acceptable because it is our cable between our boxes, never presented to
  the lighting console.

### 3.5 Isolation and protection

Optoisolation between the desk's signal ground and the installation ground belongs in the
ground station, not the fixture. The exact topology — optocoupler + isolated
DC-DC, or an integrated isolated RS-485 transceiver (e.g. ADM2582E) — is a ground-station
design decision.

Because the hybrid cable runs power alongside DMX data lines, a fray or short could
introduce supply voltage onto a fixture's data port. **TVS diodes must be added across the
DMX data lines at each fixture entry.**

In the final multi-station chain (Build 7) there are several separately-powered nodes on a
structure of uncertain earth; per-station isolation is detailed there.

---

## 4. Build Plan

### Build 1 — Pi driving the wired universe (first end-to-end system)

**The current target.** The Pi captures audio, runs the generative engine, and clocks one
DMX-512 universe out of the RS-485 HAT at 40 Hz. Build 1 is complete when the chain runs
unattended: Pi up, wire live, fixtures showing smoothly drifting generative colour.

#### Open tasks — scrub home-WiFi credentials from git history

The old home-network SSID/passphrase (`radiowaves` / `IkWilInternetten!!`) are
committed across history — in files that have since been removed from `HEAD`
(the ESP32 crates' `src/config.rs`, `ponytail/src/storage.rs`, the old
`bone/src/main.rs`). Deleting them does not remove them from history, so this
task stands: rewrite history to replace them everywhere with the new network
values, then force-push.
**Destructive — coordinate; every clone must be re-cloned afterwards.**

- [ ] Back up first: `git clone --mirror <repo> backup.git`.
- [ ] Commit the new credentials (above) so `HEAD` no longer contains the old strings.
- [ ] `sudo apt install git-filter-repo` (packaged on Trixie).
- [ ] Replace the old credentials across all blobs with `replacements.txt` (below).
- [ ] Re-add the remote (filter-repo drops it) and
      `git push --force --all && git push --force --tags`.
- [ ] Re-clone on every machine; delete stale clones and the mirror backup once verified.

`replacements.txt`

    radiowaves==>closed claw DMX
    IkWilInternetten!!==>close-that-claw

    git filter-repo --replace-text replacements.txt --force

---

### Build 4 — Bone (electroluminescent strip)

An electroluminescent (EL) strip fixture driven by an ESP32-S3 over sACN.
EL is not a DC load: it needs high-voltage AC (≈100V at a few hundred Hz), so the work splits
into making a *controllable* EL driver and then wiring that driver into DMX. The firmware
scaffold exists — `bone/` already joins the Pi's WiFi, self-identifies by station MAC, and
emits a fixed 2 kHz square wave on GPIO13 — but there is no sACN receive yet and no real EL
inverter; the GPIO13 drive is a logic-level placeholder.

#### Driver circuit

- [ ] Source an off-the-shelf EL inverter module, or build one (boost stage + H-bridge) that
      takes the 12V rail to ≈100V AC at a few hundred Hz.
- [ ] Make brightness controllable — gate the inverter from a logic PWM line, or vary its
      drive — so a DMX level maps to perceived brightness.
- [ ] Measure EL strip current draw and confirm inverter headroom.
- [ ] Confirm logic-side isolation/level shifting so the HV stage cannot reach the ESP32.

#### Firmware

- [ ] sACN E1.31 listener.
- [ ] Map one DMX channel to EL brightness via the inverter gate line (replace the fixed
      2 kHz placeholder on GPIO13).
- [ ] Add bone's DMX start address / universe to `bone/src/config.rs`, keyed by station MAC.

#### Validation

- [ ] EL strip dims smoothly across the DMX range with no audible inverter whine at low levels.
- [ ] End-to-end via the Pi sACN sender.

---

### Build 5 — Hoof (base LED spotlights)

LED spotlights at the base of the sculpture. These are conventional DC LED loads; the work is
to wire them into DMX so they are driven from the same sACN stream as the other fixtures.

Alternatively, these are rental DMX lights, so we need wiring (5-pin and 3-pin) for an extra
universe for these. Opto-isolation and proper standard wired DMX, no custom or non-standard
work on this universe.

#### Hardware

- [ ] Determine spotlight electrical type (voltage, constant-current vs constant-voltage,
      single-colour vs RGBW) and per-channel current.
- [ ] Driver: low-side MOSFET per channel from an ESP32-S3, or an off-the-shelf DMX-capable
      LED driver if the current is beyond a discrete MOSFET.

#### Firmware

- [ ] sACN E1.31 listener.
- [ ] LEDC PWM channel(s) for the spotlights at ≥20 kHz (video-safe).
- [ ] Assign hoof a DMX start address / universe, keyed by station MAC.

#### Validation

- [ ] Spotlights dim smoothly across the DMX range.
- [ ] End-to-end via the Pi sACN sender.

---

### Build 6 — Installation hardware

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
| 5 | 3-way connector| 1 | for the various 220V connectoers internally |

#### Cabinet assembly

- [ ] Plug the keyhole and previous mounting holes (seal the unused keyhole opening for weatherproofing).
- [ ] Design — and if needed add — drainage and ventilation holes (condensation management;
      see the vent membrane / desiccant in the BOM).
- [ ] Mount the 230 V mains connector (Neutrik PowerCON).
- [ ] Wire the 220 V mains side internally (low-power and signal wiring are done).
- [ ] Design the 48 V circuit.
- [ ] Tidy up the internal cable bundles.
- [ ] Design and implement mounting to the Tensegrity sculpture.

#### Software service

- Enable watchdog (systemd + hardware) so a hang reboots rather than freezing the piece.
- **Protect the SD card:** read-only root (overlayfs) or at minimum log to a RAM ring buffer.
  Days of writes to a writable root is a classic multi-day-install failure.
- Verify enclosure thermals in direct sun; throttling surfaces first as audio xruns.
- Condensation: sealed boxes sweat; the vent membrane and desiccant prevent internal dew.
- Temperature checking.

#### Commissioning checklist

- [ ] Assign and record static IPs for Pi and all fixtures; configure Pi AP.
- [ ] Set fixture DMX start addresses and confirm universe matches.
- [ ] Bench-run Build 1: confirm smooth drift, breathing, steady 44 Hz, fixture-loss tolerance.
- [ ] If audio capture is installed: connect a known feed, confirm intensity-on-loudness and
      beat surges, no strobe; pull the feed, confirm crossfade within a few seconds; restore.
- [ ] If audio capture is installed: unplug/replug USB interface mid-run, confirm survival.
- [ ] Reboot-on-boot, watchdog recovery, and read-only-root all verified.
- [ ] 24-hour soak before deployment; check thermals and for any log growth.

#### Acceptance

- Powers up into the running piece unattended after a cold boot.
- Runs the full deployment window (≤7 days) without intervention, SD corruption, or thermal
  shutdown.

---

### Build 7 — Robust, isolated power architecture

Target topology: **ground station → high station 1 (ESP + fixtures) → high station 2
→ high station 3 → DMX terminate.** Lead lengths up to 25 m from base to station 3.
Mounted on a metal structure of uncertain earthing.

#### Discussion — power and leads

**12V over 25 m is doable but voltage-drop-limited.** Loss is I²R; the current is set by the
LED array (≈2A/station worst case). Round-trip conductor length to station 3 is 50 m.

| Conductor | R (50 m round trip) | Drop @ 2A | Drop % | Cable loss |
|-----------|---------------------|-----------|--------|-----------|
| 1.0 mm² | 0.86 Ω | 1.72 V | 14% | 3.4 W |
| 1.5 mm² | 0.58 Ω | 1.15 V | 9.6% | 2.3 W |
| 2.5 mm² | 0.35 Ω | 0.69 V | 5.8% | 1.4 W |
| 4.0 mm² | 0.22 Ω | 0.43 V | 3.6% | 0.86 W |

If power is **daisy-chained**, the base→station-1 trunk carries the sum (up to 6A for three
stations). The critical constraint is **brightness matching**: if the LED array has a
constant-current driver, brightness holds flat down to the driver's dropout and sag stops
mattering; if it is voltage-driven, station 3's sag shows as uneven brightness (ties to the
§2.1 LED-driver investigation).

**The better architecture — distribute high, buck locally.** Distribute at 24V or 48V and
step down to a clean local 12V at each station with a buck converter. This gives ¼ (24V) or
1/16 (48V) of the I²R loss, and every station gets an identical regulated voltage regardless
of chain position. 48V stays within extra-low-voltage / touch-safe territory.

**Capacitors buffer transients, not steady sag.** A local bulk capacitor at each station
(a few thousand µF of low-ESR electrolytic + ceramics; size from C = ΔI·Δt/ΔV, e.g. ≈4000
µF for a 2A step over 1 ms held to 0.5 V) handles fast load steps that 25 m of cable
inductance cannot deliver from the base. A capacitor cannot fix steady IR-drop sag. Large
caps on power-on create inrush → add NTC / soft-start.

**Per-station isolation.** Three separately-powered nodes on a structure of uncertain earth
invites ground loops: the DMX signal common would otherwise tie all local grounds together.
Isolate each station's DMX interface (integrated isolated RS-485 transceiver with isolated
DC-DC, e.g. ADM2587E). Two sub-topologies:

- *Isolated tap:* one continuous bus through all stations, terminated once at the far end.
  Simpler.
- *Isolated repeater:* each station regenerates DMX onto the next segment; kills long-bus
  common-mode accumulation. **Fail-through risk:** a regenerating node that depends on its
  MCU breaks everything downstream if that MCU crashes — use a buffered/relay-bypass thru
  or an MCU-independent repeater.

**Grounding and cable.** Star-ground at each station. Run a **dedicated DMX signal common**
separate from the power return, with **data+/data− as a twisted pair**. The structure's
earthing and bonding is a safety matter for a qualified electrician, independent of signal
logic.

> **Deferred decision:** final distribution voltage (12 / 24 / 48 V) and distribution method
> (daisy-chain vs home-run) — see TODO below.

#### Hardware BOM

- [ ] Multi-conductor hybrid installation cable — baseline **5-conductor** (V+, return, DMX
      common, data+, data−) with data+/data− a **twisted pair**; trunk fatter than spurs if
      daisy-chained; cross-section per voltage decision
- [ ] 3× isolated RS-485 transceiver with integrated isolated DC-DC (e.g. ADM2587E)
- [ ] 3× local buck converter (12V output; input range per chosen distribution voltage)
- [ ] 3× bulk reservoir capacitor (low-ESR electrolytic, ~2200–4700 µF) + ceramics per station
- [ ] Inrush limiting (NTC thermistor or active soft-start) per station feed
- [ ] TVS diodes at each station's power entry and on DMX data lines
- [ ] 120Ω terminator — single far-end (tap topology) or per-segment (repeater topology)
- [ ] PSU sized for aggregate worst case at the chosen distribution voltage
- [ ] Optional: fuse / PTC per station feed

#### Design decisions

- [ ] **Decide distribution voltage (12 / 24 / 48 V)** and **distribution method
      (daisy-chain vs home-run)** — deferred
- [ ] Resolve LED-driver question (constant-current vs voltage-driven) from §2.1 —
      determines whether voltage sag affects brightness
- [ ] Once voltage chosen: finalise conductor cross-section and trunk/spur sizing
- [ ] Choose isolation sub-topology: isolated tap vs isolated repeater
- [ ] If repeater: design fail-through path (buffered/relay-bypass thru)
- [ ] Size bulk capacitance per station from worst-case load step (C = ΔI·Δt/ΔV)
- [ ] Specify inrush limiting for aggregate power-on surge
- [ ] Star-grounding scheme at each station PCB
- [ ] Hand structure earthing / bonding to a qualified electrician

#### Validation

- [ ] Measure voltage at each station under full simultaneous load; confirm within each
      buck's input range.
- [ ] Confirm brightness uniformity across all three stations at full load.
- [ ] Confirm DMX integrity with all motors + LEDs switching (worst-case EMI).
- [ ] Confirm power-on inrush does not trip the PSU.
- [ ] Confirm data integrity is independent of structure earth.

---

### Build 9 — Outgoing DMX universe 0 (wired, Pi DMX HAT)

The Pi gains a **second DMX universe**, emitted as **wired DMX-512** through a Pi
HAT, alongside the existing WiFi sACN universe 1. Universe 0 drives the **base
lights** — conventional rental fixtures that differ from deployment to deployment.

The split between the two universes is **only routing and cable management**:
universe 1 is the sculpture's own fixtures, universe 0 is whatever wired fixtures
we rent for the base. The generative engine stays
**universe-agnostic** — it does not know or care where a value goes. A new **patch
layer** is the single place that maps generated signals onto real fixtures at real
addresses in real universes, so re-patching for a new rental is a config edit, not
a code change.

#### Software architecture — separate the engine from the routing

Today `noise_task` (`brain/src/orchestrator.rs`) does everything in one loop: hardcode
each fixture and its address, pack the slots, send one universe.
Build 9 splits that into four concerns, each replaceable on its own:

1. **Engine (universe-agnostic, mostly exists).** Produces a per-frame bundle of
   abstract source signals: the shared intensity / breathing, plus a palette of
   independent Perlin colour streams. Knows nothing about fixtures, addresses, or
   universes. This is the mapping in `orchestrator.rs` plus the Perlin streams as they stand, lifted
   out of the fixture-specific slot packing.
2. **Patch (new — the per-deployment config).** A table of fixtures, each carrying:
   target **universe**, **start address**, **profile** (channel layout), and a
   **source binding** (which engine streams feed its channels). This is the one
   module that changes when the rental list changes — a hand-edited deployment
   constant, not runtime logic.
3. **Renderer (new — mechanical).** Walks the patch, reads the engine bundle, and
   fills one slot buffer **per universe**. No creative decisions live here.
4. **Sinks (new — an abstraction over the existing send).** One output per universe:
   universe 1 → the existing sACN-over-WiFi path; universe 0 → the DMX HAT serial
   path. A sink takes a finished slot buffer and ships it; the renderer does not
   know which transport a universe uses.

Keep it light: profiles are a small enum (IRGBW, RGBW, RGB, Dimmer, …) covering the
layouts we actually rent; a source binding is a tiny struct, not a scripting
language. The aim is that adding a rented 4-channel RGBW par is **one row** in the
patch table, and moving a fixture between universes is **one field**.

#### DMX HAT and wired output

The HAT is a **Zihatec RS422/RS485 HAT** (`hwhardsoft.de`), already fitted to the Pi. It is
**galvanically isolated**, which also satisfies the isolated-output requirement — no separate
isolation stage is needed on this universe. It connects to the Pi's hardware UART through an
RS-485 transceiver whose direction (DE/RE) is configurable. **Which GPIO pins it uses is set by
the board's jumper and DIP switches, not fixed** — determine them from the datasheet's *Used
Raspberry Pi Pins* table together with the physical jumper/DIP positions, don't assume (the HAT
has no ID EEPROM, so the device tree won't tell you either):

- **UART pins** — chosen by jumper K3 (older revisions: solder jumpers). Default is **UART0 =
  GPIO14 TX (header pin 8) / GPIO15 RX (pin 10)**, the only option on our Pi 3B (the alternate
  UART3/4/5 mappings exist only on Pi 4/5). Open the OS alias **`/dev/serial0`** — it resolves to
  this UART once Bluetooth is off and is immune to the `ttyACM*` probes renumbering things; never
  hardcode a `ttyAMA*`/`ttyS*`.
- **Direction (TX_EN) pin** — governed by DIP switch **S1**: `S1:3` = *automatic* DE/RE (the
  transceiver toggles itself from UART activity, no GPIO needed), or `S1:4` = *manual* via GPIO.
  The manual pin is **GPIO18 (header pin 12)** by default, or GPIO6 (pin 31) if the jumper selects
  it; **HIGH = transmit, LOW = receive**. Check the physical jumper to confirm which.

Because our sink generates the DMX break itself (not via OLA), prefer **manual control (S1:4),
holding GPIO18 HIGH for the whole frame including the break** — deterministic, where an automatic
switch can tri-state on the edge-less break and corrupt it. Automatic mode (S1:3) is the
vendor/OLA default and a fine fallback if we end up driving the HAT through OLA.

- **DIP switches** (manual-GPIO18, from the datasheet's RS485 example): SW1 = OFF·ON·OFF·ON,
  SW2 = OFF·OFF·ON·ON (Y→A, Z→B internally for half-duplex), SW3 = ON·OFF·ON·ON — SW3.1
  termination ON only if the HAT is the last device on the bus (else OFF); SW3.3/3.4 are the 4k7
  bias pull-down/pull-up that hold the idle line defined. (A prior project on the identical HAT
  used SW1 = OFF·ON·ON·OFF — that is S1:3 *automatic* mode, which also works for sending.)
- **Terminal block** (5-pin K2): `A` = data+ (K2.1, XLR pin 3), `B` = data− (K2.2, XLR pin 2),
  `Shield` = gnd (K2.5, XLR pin 1) — confirmed against the prior project's wiring.
- **Pi config** (`/boot/firmware/config.txt`; deployed unit is a Pi 3B): the mini-UART (`ttyS0`)
  is unfit for DMX — its baud tracks the core clock — so move the PL011 onto the header. The Pi
  currently has `enable_uart=1` only; still required: add `dtoverlay=disable-bt` and
  `init_uart_clock=16000000`; free the port with `raspi-config` → Serial Port (login shell *No*,
  hardware *Yes*, which drops `console=serial0,115200` from `cmdline.txt`) and
  `sudo systemctl disable serial-getty@ttyAMA0.service hciuart`. Reboot, then confirm
  `ls -l /dev/serial0` → `ttyAMA0`.

Because this universe is send-only, hold **GPIO18 in the transmit state for the whole frame,
break included** — no per-byte direction flipping.

- [ ] Drive the PL011 UART at DMX-512 timing: **250 kbaud, 8N2**, break ≥ 92 µs,
      mark-after-break ≥ 12 µs, refresh at the engine's frame rate (44 Hz); hold GPIO18 in
      transmit throughout.
- [ ] Decide the break-generation method on the PL011 (the hard part): `tcsendbreak` /
      baud-toggle on `/dev/serial0`, or bit-bang via `rppal`. Spike this early — clean DMX
      breaks from a Pi UART are the main technical risk of the build. OLA's UART native DMX
      plugin (output-only) and the raspberrypi-dmx.org baremetal Art-Net→DMX-Out both drive
      this exact HAT and are working references; the prior project's test sender used the
      `Ray-electrotechie/Serial-dmx-with-python3` library (Pi serial DMX-send) — a concrete
      break-generation precedent to port from.

#### Brain (Rust) tasks

- [ ] Replace the single `UNIVERSE` constant with a list of universe outputs, each
      bound to a sink.
- [ ] Extract the engine bundle from `noise_task` so generation no longer references
      individual fixtures directly.
- [x] Add `patch.rs`: the fixture table. **Done differently and better** — it is
      generated from the QLC+ workspace rather than hand-edited, so the patch has one
      source of truth and the compiler catches drift (see §1.1). What is *not* yet
      there is the per-fixture **profile** and **source binding**; QLC+ knows a
      fixture's mode but not which engine stream should feed which channel, so that
      half stays hand-written and still needs designing.
- [ ] Add a renderer that produces one slot buffer per universe from engine + patch.
- [ ] Add a `dmx_hat` sink that owns the serial port and clocks universe 0 out at
      44 Hz; keep the sACN sink for universe 1.
- [ ] Preflight check at sink startup — verify the *runtime effect*, not `config.txt` (the
      file is only the request; a failed overlay or wrong boot path would parse fine yet still
      be broken). Two `std`-only reads, panic with a remediation message on failure:
      `fs::read_link("/dev/serial0")` must resolve to `ttyAMA0` (else mini-UART / UART not
      enabled → "add `dtoverlay=disable-bt`, reboot"); `/proc/cmdline` must **not** contain
      `console=serial0`/`console=ttyAMA0` (a login getty would corrupt the stream → "raspi-config
      Serial Port: login shell off"). Do *not* try to assert `init_uart_clock` (not exposed on a
      stable path, and 250000 divides cleanly from the default clock anyway) or the S1 DIP / K3
      jumper (hardware, unreadable from software).
- [ ] Move the laser and the three Yaras into the patch table (universe 1) so the
      existing rig is just the first patch entries — no behavioural change for Build 1.

#### Validation

- [ ] Universe 0 drives a known wired fixture (rented or bench par) at the right
      address; a DMX tester / QLC+ shows correct, flicker-free 44 Hz output.
- [ ] Universe 1 (WiFi sACN) is byte-for-byte unchanged vs Build 1 after the refactor.
- [ ] Both 3-pin and 5-pin outputs work.
- [ ] Re-patching a fixture to a new address or universe is a patch-table edit only.
- [ ] End-to-end: engine → patch → both sinks, running unattended.
