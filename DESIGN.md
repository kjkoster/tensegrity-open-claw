# Tensegrity Open Claw — Design Document

## 1. System Overview

The system is split into two halves that communicate via sACN (E1.31) over a closed WiFi network:

- **Pi controller** — a Raspberry Pi running a Rust program on Linux. It generates 8 channels of independent Perlin noise, maps them to IRGBW DMX slots for two fixtures, and multicasts an sACN frame at 44 Hz. The Pi also acts as a WiFi access point for the fixtures and is reachable from a development laptop via its Ethernet port.

- **Ponytail fixture** — a fibre-optic light fixture retrofitted with an ESP32-S3 (Seeed Studio XIAO). The MCU joins the Pi's WiFi as a station, subscribes to the configured sACN universe, and drives the LED array via LEDC PWM and Bluetooth Low Energy (BLE). The DMX start address, universe, sACN port, and WiFi credentials are compiled in, selected at boot from a table keyed by the board's WiFi station MAC (see `ponytail/src/config.rs`).

```
    ┌── Laptop ───────────────────────────────────────┐
    │  (SSH / cross-compile Ponytail for xTensa)      │
    └────────────────── Ethernet ─────────────────────┘
                                  │
    ┌── Raspberry Pi ──────────────┴──────────────────┐
    |  (compile for Brain/local audio)                |
    │  Perlin noise engine                            │
    │  sACN sender (44 Hz) ────────── WiFi AP         │
    └─────────────────────────────────────────────────┘
                                  │ WiFi (sACN multicast)
                          ┌───────┴────────┐
                     ┌────┴─────┐    ┌─────┴────┐
                     │ Ponytail │    │ Ponytail │
                     │ ESP32-S3 │    │ ESP32-S3 │
                     └──────────┘    └──────────┘
```

### 1.1 DMX layout

Each Ponytail fixture is IRGBW + Gobo rotation: Intensity on base address +0, then R, G, B, W, Gobo.
Two Ponytail fixtures occupy one universe, 12 slots, as shown in the table below.
B to 7.

| Slot | Fixture | Channel       | Driven by                 |
|-----:|---------|---------------|---------------------------|
| 1    | A       | Intensity     | audio loudness            |
| 2    | A       | Red           | Perlin (seed 0)           |
| 3    | A       | Green         | Perlin (seed 1)           |
| 4    | A       | Blue          | Perlin (seed 2)           |
| 5    | A       | White         | Perlin (seed 3)           |
| 6    | A       | Gobo rotation | 0 (BLE personality only)  |
| 7    | B       | Intensity     | audio loudness            |
| 8    | B       | Red           | Perlin (seed 4)           |
| 9    | B       | Green         | Perlin (seed 5)           |
| 10   | B       | Blue          | Perlin (seed 6)           |
| 11   | B       | White         | Perlin (seed 7)           |
| 12   | B       | Gobo rotation | 0 (BLE personality only)  |

The sixth channel, Gobo rotation, is consumed only by the BLE bridge personality (Build 9);
the PWM fixtures ignore it and the brain currently holds it at 0.

White rides Perlin like the other colour channels, but because W desaturates the mix it
typically wants scaling down — a per-channel output gain handles this without changing the
noise model. The Intensity slots use a silence-breathing baseline, overridden by audio
loudness when music is present.

---

## 2. Hardware

### 2.1 Ponytail fixture (fibre-optic, Seeed Studio XIAO-ESP32-S3)

A cheap, generic Chinese fibre-optic light fixture. It consists of plastic optic fibres lit
by an LED array, with a patterned metal plate rotated by a stepper motor in front of them.
It runs from a 12V 2A supply. In its current form, we send commands to the device using BLE,
emulating the app that comes with this device.

For testing we still drive a few GPIO pins using PWM, with the same colours as the LEDs in the
fibre optic fixture.

**Identified components**

| Part | Role | Notes |
|------|------|-------|
| EP2T10 | RF SoC + MCU | Daughterboard, 36-pin QFN/QFP, 24MHz crystal, 2.4GHz antenna |
| 5B7TP18 | I/O / audio controller (probable) | 28-pin SMD, 2 microphones nearby |
| ULN2803A | 8-ch Darlington driver | Drives LED strings; inputs are the intervention point |
| 7805 / 78M05 | Linear regulator | 12V → 5V internal logic rail |
| GM13x (×2, SOT-23) | Small switching transistor | Discrete loads (motor / LED channel) |
| 28BYJ-48 | Unipolar stepper, 5V | Rotates the patterned plate; 5 wires (4 coils + common) |
| LED daughterboard | LED array | 5 leads, cooled, decent current → likely RGBW/RGBA (4 + common) |

The EP2T10, 5B7TP18, and the 8-pin GM13x are opaque Chinese OEM parts with no public
datasheets.

RF sniffing was attempted (nRF24L01+ in pseudo-promiscuous mode, sweeping all 126 channels
at 250 k/1 M/2 Mbps) and produced no usable data. The remote's RF chip (PL1167-family)
likely operates at 500 kbps, which the nRF24L01+ cannot demodulate. OTA emulation of the
remote is not possible with available hardware and is not pursued further.

**Open hardware questions (block Build 2 — direct control)**

- LED daughterboard supply voltage (5V or 12V?) — measure with the original board running.
- Does the daughterboard carry its own constant-current driver IC, or is it raw voltage-driven?
- LED array common-anode or common-cathode? IRLz44N (low-side N-channel) suits common-anode.
- Confirm the 5-lead connector is the LED array, not the motor.
- 7805 current headroom for the 5V rail (stepper + logic) — or replace with a buck converter.

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

sACN (E1.31, ANSI E1.31) is the primary transport for this rig, and the only one between the
Pi and the WiFi-attached fixtures. (Build 10 adds a second, *wired* DMX-512 universe out of
the Pi for rental base lights — a local output stage in the cabinet, not a change to how the
WiFi fixtures are driven.) It carries DMX-512 slots over UDP; the Pi sends each universe as
multicast.

Art-Net is explicitly not supported: we control all devices on the network, so compatibility
with older rigs is not a requirement.

### 3.2 Fixture control paths

Two paths exist into the fixture hardware:

- **Direct board replacement (Build 2):** replace the original board entirely and drive the
  RGBW LED array and 28BYJ-48 stepper directly from the ESP32-S3 for full per-channel
  control. Requires the hardware investigation in §2.1 first.
- **BLE bridge to the original controller (Build 9):** keep the fixture's original Telink
  BLE board and bridge sACN-DMX to its BLE write characteristic — the only path that reaches
  the gobo motor. A new ponytail personality, distinct from the PWM build.

### 3.3 Software stack

**Ponytail (ESP32-S3):** `no_std` Rust on `esp-hal`, async via Embassy. LEDC PWM at ≥20 kHz
(video-safe, flicker-free). Inline sACN E1.31 decoder in `sacn.rs`; the `sacn` and
`sacn-unofficial` crates are `std`-only and not usable on bare metal. For wired DMX receive
(Build 3), standard `esp-hal` UART traits treat the >88µs DMX Break as a framing error;
interrupt-driven break detection via `pac` (`rxd_break` flag) will be needed. BLE to control
the board is implemented too.

**Pi controller:** standard Rust on Trixie Raspbian Linux, `std`. sACN E1.31 encoder writing
to a UDP socket. Clones the sACN library from the Ponytail. `cpal` via ALSA for audio capture.

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
ground station (Build 6), not the fixture. The exact topology — optocoupler + isolated
DC-DC, or an integrated isolated RS-485 transceiver (e.g. ADM2582E) — is a Build 6
design decision.

Because the hybrid cable runs power alongside DMX data lines, a fray or short could
introduce supply voltage onto the ESP32's UART port. **TVS diodes must be added to the
DMX data lines inside the ponytail (Build 3) to protect the MCU.**

In the final multi-station chain (Build 8) there are several separately-powered nodes on a
structure of uncertain earth; per-station isolation is detailed there.

---

## 4. Build Plan

### Build 1 — Pi sACN sender + ponytail (first end-to-end system)

**The current target.** The Pi generates autonomous Perlin noise and multicasts sACN frames at
44 Hz to the ponytail. The ponytail firmware is already implemented; Build 1 is complete
when the full chain runs unattended: Pi up, ponytail connects, fixture glows with smoothly
drifting Perlin colour.

#### Open tasks — scrub home-WiFi credentials from git history

The old home-network SSID/passphrase (`radiowaves` / `IkWilInternetten!!`) are
committed across history — in the current `*/src/config.rs`, the removed
`ponytail/src/storage.rs`, and the old `bone/src/main.rs`. Rewrite history to
replace them everywhere with the new network values, then force-push.
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

#### Open tasks — remove Parquet recording

Remove the offline sound-profile recorder (the Arrow/Parquet capture from SOUND.md
§14). It samples the `AudioFeatures` snapshot at 10 Hz and writes rotating Parquet
files to disk — useful during DSP tuning, but not wanted in the deployed piece, and
its continuous SD-card writes work directly against the read-only-root goal in
Build 7.

- [ ] Delete `brain/src/recorder.rs`.
- [ ] Remove `mod recorder;` and the `spawn_recorder` call (and its comment) from
      `brain/src/main.rs`; `rx` then no longer needs the extra `.clone()`.
- [ ] Remove the `RECORDER_*` constants from `brain/src/config.rs`.
- [ ] Drop the now-unused `arrow`, `parquet`, and `chrono` dependencies from
      `brain/Cargo.toml`.
- [ ] Rebuild on the Pi and confirm the DMX loop is unaffected (the removal must not
      touch the sACN send path).

---

### Build 2 — Direct board replacement (ponytail)

Replace the original controller board entirely and drive the RGBW LED array and 28BYJ-48
stepper directly from the ESP32-S3. This gives full per-channel colour control and motor
speed/direction. **Requires completing the hardware investigation in §2.1 before ordering
any components.**

#### Hardware investigation (blocking)

- [ ] Measure LED daughterboard voltage with original board running.
- [ ] Determine if daughterboard has onboard constant-current driver IC.
- [ ] Determine LED driver input type (PWM / analogue / digital).
- [ ] Confirm LED array common-anode or common-cathode.
- [ ] Confirm 5-lead connector is LED array, not motor.
- [ ] Verify 7805 headroom for 5V rail, or plan buck converter replacement.

#### Hardware BOM (pending investigation results)

- [ ] 4× IRLz44N (pending LED driver investigation — may not be needed)
- [ ] ULN2003A (for 28BYJ-48 stepper)
- [ ] 4× 100Ω (IRLz44N gate), 4× 10kΩ (IRLz44N gate pulldown)
- [ ] Decoupling: 100nF ceramic per IC pin, 10µF electrolytic per rail
- [ ] 12V→5V regulation (reuse 7805 or buck converter)

#### Firmware

- [ ] 28BYJ-48 stepper sequencing via ULN2003A (timer-driven).
- [ ] Map DMX channels for motor speed and motor direction.

#### Integration

- [ ] Design and prototype replacement PCB.
- [ ] Verify fitment in enclosure.
- [ ] Connect and test LED array.
- [ ] Connect and test stepper.
- [ ] End-to-end via Pi sACN sender.

---

### Build 3 — Wired DMX receive (ponytail)

Add an RS-485 wired DMX-512 receiver to the ponytail as an alternative to WiFi sACN. This
allows control from a traditional DMX console via a 5-pin XLR without any network
configuration.

#### Hardware BOM

- [ ] SIT65HVD08P or MAX3485 (RS-485 transceiver)
- [ ] **Decision:** verify SIT65HVD08P RO output level is ≤3.3V; if not, substitute MAX3485
      to avoid damaging the ESP32.
- [ ] 5-pin XLR socket (fixture end)
- [ ] 120Ω termination resistor (if last device in chain)
- [ ] 4-wire cable (12V, GND, data+, data−) — bench/single-fixture case
- [ ] TVS diodes across DMX data lines to protect the ESP32 (see §3.5)
- [ ] USB-DMX interface dongle (for Mac / QLC+ bench testing)

#### Firmware

- [ ] DMX-512 receive via UART (250kbaud, 8N2).
- [ ] Interrupt-driven break detection via ESP32 `pac` (`rxd_break` flag) — see §3.3 for
      why the standard `esp-hal` UART traits are insufficient.
- [ ] Integrate `dmx-rdm` transport, or implement own `dmx512-rs485` on `esp-hal`.
- [ ] Evaluate `dmx512-rdm-protocol` `DmxUniverse` for the multi-channel universe model
      now that we handle 6 DMX channels (R, G, B, W, motor speed, motor direction).
- [ ] Review forum thread for ESP32/WiFi context: https://esp32.com/viewtopic.php?t=47612

#### Validation

- [ ] Control onboard LED over wired DMX via QLC+.
- [ ] End-to-end: QLC+ → USB-DMX dongle → XLR → fixture.

---

### Build 4 — Bone (electroluminescent strip)

An electroluminescent (EL) strip fixture driven by an ESP32-S3 over sACN, like the ponytail.
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

- [ ] sACN E1.31 listener (reuse the ponytail `sacn.rs` decoder).
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
- [ ] Driver: low-side MOSFET per channel from an ESP32-S3 (as the ponytail drives RGBW), or
      an off-the-shelf DMX-capable LED driver if the current is beyond a discrete MOSFET.

#### Firmware

- [ ] sACN E1.31 listener (reuse the ponytail firmware).
- [ ] LEDC PWM channel(s) for the spotlights at ≥20 kHz (video-safe).
- [ ] Assign hoof a DMX start address / universe, keyed by station MAC.

#### Validation

- [ ] Spotlights dim smoothly across the DMX range.
- [ ] End-to-end via the Pi sACN sender.

---

### Build 6 — Ground station

A purpose-built box that takes a 5-pin XLR input from a desk and re-drives DMX + power out
to the installation cable. Optoisolates the desk's signal ground from the installation ground.

#### Hardware BOM

- [ ] 5-pin XLR input socket (desk side, female)
- [ ] 5-pin XLR output connector (ground station side, male → installation cable)
- [ ] 5-pin XLR input connector (installation side, female ← installation cable)
- [ ] Optocouplers / isolated RS-485 transceiver

#### Design

- [ ] Decide isolation topology: optocoupler + isolated DC-DC, or integrated isolated
      transceiver (e.g. ADM2582E).
- [ ] Schematic: XLR in → isolate → DMX + power onto installation cable.
- [ ] Verify isolated ground preserves DMX signal integrity.
- [ ] Install everything.

#### Validation

- [ ] Engineer can patch a standard 5-pin XLR, sees no power on the signal cable.
- [ ] Full chain: desk → ground station → installation cable → fixture.

---

### Build 7 — Installation hardware

Encloses the Pi and all support hardware for a weatherproof, unattended outdoor deployment
lasting up to 7 days.

#### Bill of materials

Prices are indicative only and must be verified at purchase.

| # | Item | Qty | Notes | Approx. unit |
|---|------|----:|-------|--------------|
| 2 | backup microSD (32 GB) | 1 | Cheap cards corrupt under days of writes | $12 |
| 4 | Pi power supply (official 5 V / 3 A or quality DC-DC) | 1 | Separate from fixture supply | $10 |
| 5 | Fixture power supply | per spec | Sized to the IRGBW fixtures | per spec |
| 6 | enclosure cable glands | 1 set | Sized for Pi + audio interface + PSUs | $40 |
| 7 | Vent membrane / desiccant pack | 1 | Condensation management in sealed outdoor box | $8 |
| 8 | Mains inlet (IEC), fuse/breaker, surge protector | 1 set | Mains entry, fusing, transient protection | $25 |
| 9 | Ferrules, terminal blocks, DIN rail, drip-loop hardware | 1 set | Tidy, serviceable wiring | $20 |

The USB audio interface and ground-loop isolator for audio capture are also housed in this
enclosure.

#### Cabinet assembly

- [ ] Order the connectors.
- [ ] Order the cable glands (wartels).
- [ ] Source DIN-rail clips/brackets to mount the power supply on the DIN rail.
- [ ] Source DIN-rail clips/brackets to mount the Alesis ADC on the DIN rail.
- [ ] Buy screws and install the DIN rails.
- [ ] Add a connector panel.
- [ ] Drill the holes (glands, connectors, mounts).
- [ ] Plug the keyhole and previous mounting holes (seal the unused keyhole opening for weatherproofing).
- [ ] Design — and if needed add — drainage and ventilation holes (condensation management;
      see the vent membrane / desiccant in the BOM).
- [ ] Mount the power supply.
- [ ] Mount the 230 V mains connector (the big blue round CEE / "camping" inlet).
- [ ] Install the Alesis ADC (io|2 USB audio interface).

#### Power

- **Separate supplies.** Pi and fixtures on separate supplies. Never power fixtures from the Pi.
- **Mains entry.** Single IEC inlet, fused/breakered, with surge/transient protector.
- **Inrush and sizing.** Size fixture PSU with margin for inrush and full-white draw.
  Budget the Pi at ~5 V/3 A peak including USB interface.
- **Brown-out.** Under-voltage causes SD corruption and audio xruns; a quality supply and
  short, adequate-gauge DC runs matter.

#### Software service

- Enable watchdog (systemd + hardware) so a hang reboots rather than freezing the piece.
- **Protect the SD card:** read-only root (overlayfs) or at minimum log to a RAM ring buffer.
  Days of writes to a writable root is a classic multi-day-install failure.
- Verify enclosure thermals in direct sun; throttling surfaces first as audio xruns.
- Condensation: sealed boxes sweat; the vent membrane and desiccant prevent internal dew.

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

### Build 8 — Robust, isolated power architecture

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

### Build 9 — Ponytail BLE bridge (Telink gobo fixture)

A completely new ponytail personality. Instead of driving the LED array via PWM (Build 1) or
replacing the board (Build 2), this keeps the fixture's **original Telink BLE controller** and
bridges sACN-DMX → BLE write commands to it. It is the only control path that reaches the
fixture's **gobo motor**. The reverse-engineered 7E/EF protocol, the 9-byte frame builders,
and the DMX→BLE translation are captured in `ble/DMX-BLE.md` and the `ble/telink*.py` /
`ble/telinkled.lua` reference (test code — reference only, may contain bugs).

The fixture is **modal**: the RGB emitters and the white LED cannot light together, so
White > 0 overrides RGB (interlocked white), and the master dimmer is applied **in software**
(the native brightness command is dead in white mode — and software dimming matches what the
PWM ponytail already does with the Intensity channel). It has **no readback**, so the bridge
always asserts the complete desired state: it re-sends on every change, on a 10 s heartbeat,
and on every reconnect.

The personality grows to **6 channels** (Dimmer, R, G, B, White, Gobo rotation) — one more
than the current IRGBW fixtures. The channel re-spacing touches the brain and both ponytail
personalities; it is fully reversible via git, so we implement it and roll back if it
misbehaves.

#### Ponytail BLE personality (firmware)

- [ ] Keep the PWM personality during bring-up — it is the known-good reference for confirming
      that sACN data arrives correctly (right slots, right values) while the BLE path is tested.
      **TODO: remove the PWM personality** once the BLE bridge is validated.
- [ ] Simulate the modal white interlock in the PWM personality too, so it matches the BLE
      fixture's behaviour: when White > 0, force R = G = B = 0 (White overrides RGB). This keeps
      the reference path faithful to interlocked white rather than co-lighting RGB and W.
- [ ] Drive the gobo with **power + speed only** (no Gobo Preset `0x15`), per the doc.
      **TODO: test on hardware** that the gobo actually renders without a preset recall (add a
      one-shot preset on connect if it doesn't), and re-sweep the 1–10 speed range to confirm.

#### Review items (the doc is reference, not gospel)

- [ ] **TODO review:** `ble/DMX-BLE.md` is prescriptive in places; where a cleaner approach
      gives the same observable effect, take it and note the deviation.
- [ ] **TODO review:** the doc's author did not know this codebase — reshape the suggested
      structure to fit ponytail's existing patterns (reuse `Listener`/`Signal`, config-by-MAC,
      the existing software-dimming convention) even where it contradicts the doc.

#### Docs

- [ ] Document this BLE-bridge personality and the 6-channel map in §2.1 / §3.2.

#### Validation

- [ ] Hold WiFi + BLE together through a multi-minute run; sACN loss does not drop BLE and
      vice-versa.
- [ ] Connection survives a fixture power-cycle and range loss (auto-reconnect + resync).
- [ ] Interlock look check: White > 0 hard-cuts to white and back; Dimmer at 0 powers the LED
      (and, via the hardware coupling, the gobo) off.
- [ ] Gobo rotation tracks the channel; 0 stops the motor.
- [ ] End-to-end via the Pi sACN sender.

---

### Build 10 — Outgoing DMX universe 2 (wired, Pi DMX HAT)

The Pi gains a **second DMX universe**, emitted as **wired DMX-512** through a Pi
HAT, alongside the existing WiFi sACN universe 1. Universe 2 drives the **base
lights** — conventional rental fixtures that differ from deployment to deployment.

The split between the two universes is **only routing and cable management**:
universe 1 is the WiFi-attached ESP32 fixtures (ponytail / bone / hoof), universe 2
is whatever wired fixtures we rent for the base. The generative engine stays
**universe-agnostic** — it does not know or care where a value goes. A new **patch
layer** is the single place that maps generated signals onto real fixtures at real
addresses in real universes, so re-patching for a new rental is a config edit, not
a code change.

#### Software architecture — separate the engine from the routing

Today `noise_task` (`brain/src/orchestrator.rs`) does everything in one loop: generate
Perlin + intensity, hardcode Ponytails A and B, pack 12 slots, send one universe.
Build 10 splits that into four concerns, each replaceable on its own:

1. **Engine (universe-agnostic, mostly exists).** Produces a per-frame bundle of
   abstract source signals: the shared intensity / breathing, plus a palette of
   independent Perlin colour streams. Knows nothing about fixtures, addresses, or
   universes. This is the mapping in `orchestrator.rs` plus the Perlin streams as they stand, lifted
   out of the fixture-specific slot packing.
2. **Patch (new — the per-deployment config).** A table of fixtures, each carrying:
   target **universe**, **start address**, **profile** (channel layout), and a
   **source binding** (which engine streams feed its channels). This is the one
   module that changes when the rental list changes; treat it like `ponytail`'s
   config-by-MAC table — a hand-edited deployment constant, not runtime logic.
3. **Renderer (new — mechanical).** Walks the patch, reads the engine bundle, and
   fills one slot buffer **per universe**. No creative decisions live here.
4. **Sinks (new — an abstraction over the existing send).** One output per universe:
   universe 1 → the existing sACN-over-WiFi path; universe 2 → the DMX HAT serial
   path. A sink takes a finished slot buffer and ships it; the renderer does not
   know which transport a universe uses.

Keep it light: profiles are a small enum (IRGBW, RGBW, RGB, Dimmer, …) covering the
layouts we actually rent; a source binding is a tiny struct, not a scripting
language. The aim is that adding a rented 4-channel RGBW par is **one row** in the
patch table, and moving a fixture between universes is **one field**.

#### DMX HAT and wired output

- [ ] Choose a Pi DMX HAT with an **isolated RS-485** output — it leaves the cabinet
      on a long cable, so the same isolation argument as Builds 6/8 applies (prefer an
      ADM2582E / ADM2587E-class isolated transceiver over a bare MAX485).
- [ ] Drive it from the Pi UART at DMX-512 timing: **250 kbaud, 8N2**, break ≥ 92 µs,
      mark-after-break ≥ 12 µs, refresh at the engine's frame rate (44 Hz).
- [ ] Decide the break-generation method on the Pi UART (the hard part): `tcsendbreak`
      / baud-toggle on the Linux serial device, or bit-bang via `rppal`. Spike this
      early — clean DMX breaks from a Pi UART are the main technical risk of the build.

#### Cabinet hardware

- [ ] Pi DMX HAT mounted to the Pi inside the IP65 enclosure (Build 7).
- [ ] Panel-mount **3-pin and 5-pin XLR** DMX outputs, wired in parallel (pins
      1 = gnd, 2 = data−, 3 = data+; 5-pin leaves 4/5 unused) so either connector
      standard can be patched.
- [ ] Internal routing from the HAT to the connector panel; keep the DMX pair twisted
      and away from mains and the fixture PSU.
- [ ] **DMX cable gland in the cabinet floor** (separate from the existing glands) so
      the universe-2 cable exits downward without compromising the IP65 seal.
- [ ] Add the HAT, the 3/5-pin XLR connectors, and the extra gland to the Build 7 BOM.

#### Brain (Rust) tasks

- [ ] Replace the single `UNIVERSE` constant with a list of universe outputs, each
      bound to a sink.
- [ ] Extract the engine bundle from `noise_task` so generation no longer references
      Ponytails A/B directly.
- [ ] Add `patch.rs`: the fixture table (universe, address, profile, source binding)
      and the profile enum.
- [ ] Add a renderer that produces one slot buffer per universe from engine + patch.
- [ ] Add a `dmx_hat` sink that owns the serial port and clocks universe 2 out at
      44 Hz; keep the sACN sink for universe 1.
- [ ] Move Ponytails A/B into the patch table (universe 1) so the existing rig is just
      the first patch entry — no behavioural change for Build 1.

#### Validation

- [ ] Universe 1 (WiFi sACN) is byte-for-byte unchanged vs Build 1 after the refactor.
- [ ] Universe 2 drives a known wired fixture (rented or bench par) at the right
      address; a DMX tester / QLC+ shows correct, flicker-free 44 Hz output.
- [ ] Both 3-pin and 5-pin outputs work.
- [ ] Re-patching a fixture to a new address or universe is a patch-table edit only.
- [ ] End-to-end: engine → patch → both sinks, running unattended.

---

### Build 11 — Manual override from QLC+ (sACN priority)

A way to take live manual control of the WiFi fixtures from **QLC+ on a laptop**
without stopping or coordinating with the brain. The brain and QLC+ both send the
same universe; the fixtures obey the **highest-priority live source** (E1.31's
built-in source arbitration) and fall back automatically when the higher source
goes quiet. The brain keeps running at its normal priority throughout — so the
override works **even if the brain has hung or crashed**, which is exactly when
manual control is most wanted. This is the property the brain-side alternatives
(detect-and-back-off, or relaying a second universe) cannot offer, since they route
the manual takeover through the very box you may be overriding.

**How it works.** E1.31 carries a one-byte **priority** (0–200, default 100) in the
framing layer (packet offset 108). The brain already stamps 100
(`brain/src/dmx.rs:44`, `brain/src/orchestrator.rs:186`). QLC+ sends the same universe at a
strictly higher priority (e.g. **200**). A receiver tracks the sources it hears per
universe — keyed by the 16-byte **CID** (offset 22–37) — acts on the highest-priority
one still alive, and drops a source after the E1.31 **network-data-loss timeout
(2.5 s)**, or immediately if that source sets the **stream-terminated** flag (options
byte offset 112, bit `0x40`) on a clean stop. Using distinct priorities (100 vs 200)
sidesteps any equal-priority merge ambiguity.

#### Brain (Rust) — essentially unchanged

- [ ] No functional change: the brain already emits universe 1 at priority 100
      (`brain/src/orchestrator.rs:186`).
- [ ] Optional clarity: lift the literal `100` into a named `SACN_PRIORITY` constant
      in `brain/src/config.rs`.

#### Ponytail (firmware) — source arbitration in the shared decoder

The work lands in `ponytail/src/sacn.rs` (`parse_e131_slots` + `Listener::run`),
which bone and hoof reuse — so all WiFi fixtures gain the behaviour at once.

- [ ] Parse the **priority** byte (offset 108) and the source **CID** (offset 22–37)
      alongside the existing fields; add `PRIORITY_OFFSET`, `OPTIONS_OFFSET`,
      `CID_OFFSET` next to the current offset constants.
- [ ] Honour the **options** byte (offset 112): treat the stream-terminated bit
      (`0x40`) as an immediate release of that source.
- [ ] Keep a small per-CID **source table** (`heapless::Vec`, ~4 entries):
      `{ cid, priority, last_seen: embassy_time::Instant }`.
- [ ] Per packet: refresh/insert its source; **adopt slots only if it is the
      highest-priority source still within the 2.5 s window**; expire stale sources.
- [ ] Keep the existing change-detection (`last_value`) and the 5 s `UNIVERSE_TIMEOUT`
      socket-rebind as the "nobody talking at all" safety net — orthogonal to source
      arbitration.

#### QLC+ (laptop) — configuration

- [ ] E1.31 output plugin: map the workspace universe to **universe 1** on the
      closed-claw network interface, UDP port **5568**.
- [ ] Set the output **priority to 200** (QLC+ defaults to 100 — it must be raised
      above the brain).
- [ ] Operating note: a **blackout** in QLC+ still *owns* the fixtures (it keeps
      sending 200 at zero). To **release** back to the brain, stop the plugin output /
      quit QLC+ (sends the stream-terminated flag → instant revert), or drop off WiFi
      (revert after the 2.5 s timeout).

#### Optional — physical "manual" toggle (composes on top)

- [ ] If a tactile, unambiguous "I am in manual" control is wanted, add a Pi GPIO
      toggle that mutes the brain's send. It is **complementary** to priority
      arbitration, not a replacement: priority does the automatic merge and provides
      the survives-a-brain-crash property; the switch is just a human-facing kill for
      the brain's stream.

#### Docs

- [ ] Note the priority-arbitration behaviour in §3.1 (Transport) so the override
      path is discoverable from the design principles.

#### Validation

- [ ] QLC+ at 200 takes over within a frame; the brain (100) is ignored while QLC+
      is live.
- [ ] Stop QLC+ output → fixtures revert to the brain within ~2.5 s, or immediately
      if QLC+ sent the stream-terminated flag.
- [ ] Laptop drops off WiFi mid-control → fixtures revert to the brain after the
      2.5 s timeout.
- [ ] Kill the brain while QLC+ is controlling → no visible change (the override does
      not depend on the brain).
- [ ] With only the brain running (no QLC+), behaviour is identical to Build 1.
