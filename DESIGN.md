# Fibre-Optic Fixture → DMX: Design Document

## 1. Overview

A cheap, generic Chinese fibre-optic light fixture is to be made DMX-controllable.
The fixture consists of plastic optic fibres lit by an LED array, with a patterned
metal plate rotated by a stepper motor in front of them. It runs from a 12V 2A
supply and is currently driven by a generic 2.4GHz "LED lamp" remote (28 buttons)
via a daughterboard with a pigtail antenna.

The work is split into eight builds. Each build is independently useful and informs
the next, so the project can stop at any point with a working result. Builds 1–6 are
the transport + control path, Build 7 is remote management / console integration,
and Build 8 is the robust isolated power architecture for the final multi-station
installation.

---

## 2. Hardware Analysis

### 2.1 Identified components

| Part | Role | Notes |
|------|------|-------|
| EP2T10 | RF SoC + MCU | Daughterboard, 36-pin (4×9) QFN/QFP, 24MHz crystal, 2.4GHz antenna |
| 5B7TP18 | I/O / audio controller (probable) | 28-pin SMD, 2 microphones nearby → sound-reactive mode |
| ULN2803A | 8-ch Darlington driver | Drives LED strings / loads; inputs are the intervention point |
| 7805 / 78M05 | Linear regulator | 12V → 5V internal logic rail |
| GM13x (×2, SOT-23) | Small switching transistor | Discrete loads (motor / LED channel) |
| GM13x (8-pin SMD) | Unknown 8-pin | Same marking, different package — likely a different part |
| 28BYJ-48 | Unipolar stepper, 5V | Rotates the patterned plate; 5 wires (4 coils + common) |
| LED daughterboard | LED array | 5 leads, cooled, decent current → likely RGBW/RGBA (4 + common) |

### 2.2 Reverse-engineering note

The EP2T10, 5B7TP18, and the 8-pin GM13x are opaque Chinese OEM parts with no
public datasheets — not indexed in any reachable database. **Functional tracing
beats datasheet hunting here:** a logic analyser on the ULN2803A inputs (or on the
EP2T10's baseband line to its RF section) reveals what signals control what,
regardless of chip identity. Identification was abandoned as low-value once the
three documented parts (ULN2803A, 7805, 28BYJ-48) gave a complete-enough picture.

> **Note on RF Sniffing:** Over-the-air sniffing of the 2.4GHz protocol using an
> nRF24L01 is highly unreliable because hardware packet handlers (like ShockBurst)
> will drop packets that do not exactly match their expected MAC format. Tracing
> must be done via a logic analyzer tapped directly into the SPI bus of the original
> remote.

### 2.3 Open hardware questions (block Build 5)

- LED daughterboard supply voltage (5V or 12V?) — measure on the leads with the
  original board running.
- Does the daughterboard carry its own constant-current driver IC, or is it raw
  voltage-driven? Determines whether the IRLz44N approach applies at all.
- LED array common-anode or common-cathode? IRLz44N (low-side N-channel) suits
  common-anode; common-cathode needs high-side / P-channel switching.
- Confirm the 5-lead connector is the LED array, not the motor.
- 7805 current headroom for the 5V rail (stepper + logic) — or replace with a buck
  converter for efficiency.

---

## 3. Design Considerations

### 3.1 Two intervention paths, one control layer

There are two ways into the fixture, plus a DMX layer that sits above both:

- **Non-invasive (RF):** emulate the remote over 2.4GHz, leaving the board intact
  (Build 4). Lowest risk; constrained to whatever the remote can express.
- **Invasive (replace):** rip out the board and drive the LEDs and stepper directly
  (Build 5). Maximum control; requires the hardware investigation above.
- **DMX transport layer:** how control data reaches the fixture — wireless sACN
  (Build 1), wired DMX-512 (Build 2), or network/Ethernet (Build 6). This layer is
  shared regardless of which intervention path drives the hardware.

### 3.2 Build sequencing rationale

Builds are ordered so each yields a working artefact and de-risks the next:

1. Wireless DMX proves the protocol + config stack on the bench with zero extra hardware.
2. Wired DMX adds the RS-485 front end — the path that actually matters for venues.
3. Ground station makes the wired path venue-ready (isolation, power injection).
4. RF emulation **sniffs the remote, which reveals the fixture's controllable
   parameters** — directly informing the Build 5 channel map before any board is cut.
5. Direct control replaces the board with full per-channel command.
6. Ethernet is optional, venue-driven only.
7. Remote management (RDM / GDTF / OSC) makes the fixture behave like professional
   gear on a console — addressable, discoverable, with feedback.
8. Robust isolated power architecture turns the bench-proven system into a reliable
   multi-station installation on a real (possibly un-earthed metal) structure.

### 3.3 Transport strategy

- **sACN (E1.31) is primary.** ANSI standard, multicast, priority/failover — the
  modern pro default.
- **Art-Net supported for compatibility** with older or Art-Net-configured rigs.
  Same DMX-over-UDP shape; downstream channel handling is identical.
- **Wired DMX-512 is the realistic pro-console interface.** Venues rarely run WiFi
  for lighting. An operator plugs a 5-pin XLR into the ground station and it works
  with no network config. Wireless is for our own installs and development.

### 3.4 Power / signal separation (pro compatibility)

DMX cables are signal-only by convention; no competent engineer will patch an
unknown cable carrying power into their desk. Therefore:

- **At the desk interface:** standard 5-pin XLR (signal only) + a separate power
  feed (mains into the ground station).
- **From ground station up to the installation:** a custom hybrid cable carrying
  power alongside the DMX pair — acceptable because it is *our* cable between *our*
  boxes, never presented to the desk. The conductor count, cross-section, and
  distribution voltage of this run are a dedicated design problem; see **Build 8**.

### 3.5 Isolation & Protection

Optoisolation between the desk's signal ground and the installation ground belongs
in the ground station, not the fixture. The exact topology is a design decision:
optocoupler + isolated DC-DC on the line side, or an integrated isolated RS-485
transceiver (e.g. ADM2582E). Full galvanic isolation requires isolated power on the
isolated side. The fixture itself just sees a clean local DMX signal + local power.

Because the custom cable runs power alongside the DMX data lines, a fray or short
could introduce supply voltage onto the ESP32's UART port. **TVS (Transient Voltage
Suppression) diodes must be added to the DMX data lines inside the fixture (Build 5)
to protect the MCU.**

In the final multi-station chain (Build 8) there are several separately-powered
nodes on a structure of uncertain earth. That topology drives **per-station
isolation** beyond the single ground-station barrier described here; the multi-drop
isolation strategy is detailed in Build 8.

### 3.6 Firmware foundation

- **Rust, no_std, on `esp-hal`** for the ESP32-S3 (Seeed Studio XIAO).
- **3.3V logic throughout (outbound):** the ESP32-S3 drives the IRLz44N gates and
  ULN2003 inputs directly — no level shifters on the MCU outputs.
- **Logic-level caveat (inbound):** the ESP32-S3 is strictly 3.3V. The RS-485
  transceiver's RO output must be ≤3.3V to the ESP. Verify the SIT65HVD08P's RO
  high level, or use a strictly-3.3V transceiver (MAX3485) to avoid frying the ESP32.
- **DMX Break Detection:** standard `esp-hal` UART read traits often treat the >88µs
  DMX Break as a framing error. Firmware will likely need to bypass high-level traits
  and use the Peripheral Access Crate (`pac`) to attach an Interrupt Service Routine
  (ISR) to the `rxd_break` hardware flag.
- **Video-Safe PWM:** any LEDC PWM configuration (Build 5) must run at a frequency of
  **≥ 20kHz** to prevent banding and flicker on cameras.
- WiFi used for DMX start-address configuration (and as the Build 1 transport).

### 3.7 Codec / transport separation + shared universe spine

The architectural spine: a single DMX **universe data model** is shared across all
builds; only the **transport** differs (sACN, RS-485, RF, Ethernet). Protocol
**codecs** (sACN, Art-Net) are designed as no_std-core with an optional std/transport
feature, so the *same* codec serves embedded (Build 1) and the Pi (Build 6) without
forking. This isolates build-specific code to a thin transport layer.

### 3.8 Upstream contribution stance

Where an ecosystem crate is missing or insufficient, we extend it upstream rather
than fork silently. This bites, in priority order:

1. **`sacn` / `sacn-unofficial` → else `e131`.** Before writing a custom E1.31
   implementation from scratch, evaluate the existing `sacn` crates for `no_std`
   feature compatibility. Only if none is workable do we implement the E1.31
   data-packet codec ourselves and contribute it upstream (the `e131` crate is
   effectively empty).
2. **`dmx-rdm`** — right layer but WIP API; likely needs break-timing control
   exposed for the `esp-hal` driver. Sidesteppable by writing our own RS-485
   transport directly on `esp-hal`.
3. **`tiny-artnet`** — minimal; extend only if embedded Art-Net is pursued.

Two pieces are ours regardless of upstream: **`led-lamp-rf`** (no Rust equivalent
exists) and **`fixture-output`** (specific to this fixture).

---

## 4. Crate / Code Structure

### 4.1 Clean-room logical crates

| Crate | Responsibility | I/O? | no_std |
|-------|----------------|------|--------|
| `dmx-core` | Universe data model: 512 slots, get/set, start-address offset | none | yes |
| `dmx512-wire` | DMX-512 slot framing (start code + slots) | none | yes |
| `dmx512-rs485` | RS-485 transport: UART break/MAB, transceiver DE/RE | UART | yes |
| `rdm` | E1.20 codec + thin transaction layer | via above | yes |
| `sacn` | E1.31 packet codec, transport-agnostic | none | yes (core) |
| `artnet` | Art-Net packet codec, transport-agnostic | none | yes (core) |
| `nrf24` | embedded-hal SPI driver for nRF24L01 | SPI | yes |
| `led-lamp-rf` | PL1167-compatible remote protocol, on `nrf24` | via nrf24 | yes |
| `fixture-output` | LED PWM, 28BYJ-48 stepper, onboard LED, on `esp-hal` | GPIO/PWM | yes |
| `config-portal` | WiFi provisioning + HTTP config + persisted address | net/flash | yes |

### 4.2 Crate × Build matrix

Legend: ✅ usable as-is · ⚠️ exists, insufficient → extend upstream · 🔨 build ourselves · — not used

| Logical crate | B1 Wireless | B2 Wired | B3 GndStn | B4 RF | B5 Direct | B6 Ethernet | B7 RemoteMgmt |
|---------------|:----------:|:--------:|:---------:|:-----:|:---------:|:-----------:|:-------------:|
| `dmx-core` | ✅ | ✅ | — | ✅ | ✅ | ✅ | ✅ |
| `dmx512-wire` | — | ⚠️ | — | — | (via transport) | — | — |
| `dmx512-rs485` | — | ⚠️ | — | — | (via transport) | — | — |
| `rdm` | — | ✅/⚠️ | — | — | optional | — | ✅/⚠️ |
| `sacn` | ⚠️ | — | — | (source) | (source) | ⚠️/✅ | — |
| `artnet` | — | — | — | — | — | ✅ | — |
| `nrf24` | — | — | — | ✅ | — | — | — |
| `led-lamp-rf` | — | — | — | 🔨 | — | — | — |
| `fixture-output` | 🔨(LED) | 🔨(LED) | — | — | 🔨(full) | — | — |
| `config-portal` | 🔨 | 🔨 | — | 🔨 | 🔨 | — | — |
| runtime (`esp-hal` etc.) | ✅ | ✅ | — | ✅ | ✅ | (Pi std) | ✅ |

> B7 also pulls in tooling outside the logical-crate set: OSC (`rosc`) and GDTF
> handling (evaluate Rust GDTF tooling; GDTF is XML, so worst case hand-author +
> validate). **Build 8 is a hardware / power build with no software crates.**

### 4.3 Existing-crate mapping

| Logical crate | Existing crate | Status | Note |
|---------------|----------------|--------|------|
| `dmx-core` | `dmx512-rdm-protocol` (`default-features=false`) | ✅ | `DmxUniverse` is exactly this; the shared spine |
| `dmx512-wire` + `dmx512-rs485` | `dmx-rdm` | ⚠️ | no_std/no_alloc RS-485 transport; WIP API; supply `esp-hal` driver |
| `rdm` codec | `dmx512-rdm-protocol` (`rdm` feature) | ✅ | no_std codec |
| `rdm` transactions | `dmx-rdm` | ⚠️ | rides the same transport |
| `sacn` | `sacn` / `sacn-unofficial`, else `e131` | ⚠️ | evaluate existing for no_std first; `e131` empty → implement + contribute |
| `artnet` (std, Pi) | `artnet_protocol` | ✅ | std-only; fits Build 6 where real sockets exist |
| `artnet` (no_std, embedded) | `tiny-artnet` | ⚠️ | minimal; extend upstream if embedded Art-Net needed |
| `nrf24` | `embedded-nrf24l01` | ✅ | verify build for `xtensa-esp32s3-none-elf` |
| `led-lamp-rf` | — | 🔨 | bespoke; port packet format from MiLight C/C++ refs |
| `fixture-output` | — (opt. `stepper`) | 🔨 | onboard LED is a plain GPIO LED (LEDC); stepper table trivial |
| `config-portal` | `esp-storage` + `esp-wifi` + `picoserve` | 🔨 | assembled glue; verify `picoserve` vs HAL version |
| OSC (B7) | `rosc` | ✅ | sensor / fixture→console feedback |
| GDTF (B7) | evaluate Rust GDTF tooling | ⚠️/🔨 | XML format; hand-author + validate if no crate fits |
| runtime | `esp-hal`, `esp-wifi`, `esp-storage`, `esp-backtrace`, `esp-println` | ✅ | foundation |

---

## 5. TODO — All Builds

### Shared: DMX Rust library

- [ ] Adopt `dmx512-rdm-protocol` `DmxUniverse` (no_std) as the shared `dmx-core` spine
- [ ] Evaluate `sacn` / `sacn-unofficial` crates for `no_std` compatibility before rolling a custom `e131` codec
- [ ] Decide `sacn`/`artnet` codec design: no_std core + optional std transport feature (serves B1 and B6)

### Build 1 — Wireless DMX (sACN over WiFi)

Firmware
- [ ] **Action Point:** review forum thread for ESP32/WiFi context: https://esp32.com/viewtopic.php?t=47612
- [ ] **Bug:** `storage.write_dmx_base_address()` returns `FlashStorageError::OtherCoreRunning` when
      the write is attempted while running under `cargo run` (probe-rs debugger), but succeeds when
      attaching after boot. The ESP32-S3 flash driver refuses writes if Core 1 is active (instruction
      fetches from flash on either core during an erase/write corrupt the fetch). Under the debugger,
      Core 1 is left in an active state at start-up. Fix: retry on `OtherCoreRunning` in `flush()`
      since the condition is transient, or halt/resume Core 1 around the erase/write.
- [ ] sACN/E1.31 receive over WiFi (UDP) — implement codec or adapt `sacn`
- [ ] Replace hand-rolled HTTP server with `picoserve` — a no_std async HTTP server built for
      Embassy; would eliminate the manual header/body parsing, byte-by-byte read loop, and response
      builder in `http_server.rs`
- [ ] Audit smoltcp features in Cargo.toml — `socket-icmp`, `socket-raw`, `socket-dns`, and
      `proto-dns` are not used by the current TCP-only HTTP server + DHCP stack; removing them
      shrinks the binary
- [ ] Control onboard LED via DMX channels as test output (LEDC/GPIO)
- [ ] Stub channel handlers for later builds

Validation
- [ ] Control onboard LED brightness via QLC+ over local WiFi

### Build 2 — Wired DMX

Hardware BOM
- [ ] SIT65HVD08P or MAX3485 (RS-485 transceiver)
- [ ] **Decision:** Verify SIT65HVD08P outputs 3.3V on the RX (RO) line to avoid damaging the ESP32, or substitute with the 3.3V-specific MAX3485
- [ ] 5-pin XLR socket (fixture end)
- [ ] 120Ω termination resistor (if last device in chain)
- [ ] 4-wire cable (12V, GND, data+, data−) — bench/single-fixture case; installation cabling is Build 8
- [ ] USB-DMX interface dongle for Mac / QLC+

Firmware
- [ ] DMX-512 receive via UART (250kbaud, 8N2, break detection)
- [ ] **Experiment:** Implement interrupt-driven break detection via ESP32 `pac` (`rxd_break` flag) if `esp-hal` UART drops frames
- [ ] Integrate `dmx-rdm` transport (or write own `dmx512-rs485` on `esp-hal`)
- [ ] Reuse shared `dmx-core` from Build 1

Test
- [ ] Control onboard LED over wired DMX via QLC+

Validation
- [ ] End-to-end: QLC+ → wired DMX → fixture

### Build 3 — Ground Station

Hardware BOM
- [ ] Beefy power supply (wattage = LED array + motor + logic; sizing finalised in Build 8)
- [ ] Ground station enclosure
- [ ] 5-pin XLR input socket (desk side, female)
- [ ] 5-pin XLR output connector (ground station side, male → installation cable)
- [ ] 5-pin XLR input connector (installation side, female ← installation cable)
- [ ] RS-485 transceiver(s) for ground-station input/output re-drive
- [ ] Optocouplers / isolated RS-485 transceiver (e.g. ADM2582E)
- [ ] Locking mains power connector (e.g. Neutrik powerCON)

Design
- [ ] Decide isolation topology (optocoupler + isolated DC-DC, or integrated isolated transceiver)
- [ ] Schematic: XLR in → isolate → DMX + power onto installation cable
- [ ] Verify isolated ground preserves DMX signal integrity

Validation
- [ ] Engineer can patch a standard 5-pin XLR, sees no power on the signal cable
- [ ] Full chain: desk → ground station → installation cable → fixture

### Build 4 — RF Remote Emulator

Hardware BOM
- [ ] nRF24L01 module (for transmission / emulation)
- [ ] Wire nRF24L01 to ESP32-S3 via SPI
- [ ] Logic Analyzer (e.g. Saleae clone) for sniffing

Sniffing (Wired — see §2.2)
- [ ] **Experiment:** Open remote, identify RF chip marking (confirm PL1167 or equivalent)
- [ ] Solder logic-analyzer wires directly to the SPI bus between the remote's MCU and RF chip
- [ ] Capture packets per button (all 28). Do not attempt over-the-air sniffing — hardware filters drop mismatched frames
- [ ] Document packet structure: length, address, command bytes, timing

Firmware
- [ ] `nrf24` driver (`embedded-nrf24l01`) integration
- [ ] `led-lamp-rf` codec — bespoke, encodes captured protocol
- [ ] RF transmit to emulate remote
- [ ] Map DMX channels → captured RF commands

Validation
- [ ] Fixture responds to emulated commands
- [ ] Full channel range via QLC+

### Build 5 — Direct Control (replace board)

Investigation (before committing) — see §2.3
- [ ] Measure LED daughterboard voltage with original board running
- [ ] Determine if daughterboard has onboard current-driver IC
- [ ] Determine LED driver input signal type (PWM / analog / digital)
- [ ] Confirm LED array common-anode or common-cathode
- [ ] Confirm 5-lead connector is LED array, not motor
- [ ] Verify 7805 headroom for 5V rail (or switch to buck)

Hardware BOM
- [ ] 4× IRLz44N (pending LED driver investigation — may not be needed)
- [ ] ULN2003A (for 28BYJ-48 stepper)
- [ ] 4× 100Ω (IRLz44N gate)
- [ ] 4× 10kΩ (IRLz44N gate pulldown)
- [ ] Decoupling: 100nF ceramic (104) per IC pin, 10µF electrolytic per rail
- [ ] 12V→5V regulation (reuse 7805 or buck converter)
- [ ] **Protection:** TVS diodes across DMX data lines to protect ESP32 from supply shorts

Firmware
- [ ] LEDC PWM for 4× LED channels (`fixture-output`)
- [ ] **Decision:** Explicitly set LEDC PWM timer frequency to ≥ 20kHz for video-safe, flicker-free operation
- [ ] 28BYJ-48 stepper sequencing via ULN2003A (timer-driven)
- [ ] Map DMX channels: LED R, G, B, W, motor speed, motor direction

Integration
- [ ] Design / prototype replacement PCB
- [ ] Verify fitment in enclosure
- [ ] Connect + test LED array
- [ ] Connect + test stepper
- [ ] End-to-end via QLC+ → DMX → fixture

### Build 6 — Ethernet (OPTIONAL)

> Only consider when a specific venue requires it. Pro consoles interface cleanly
> via wired DMX (B2/B3) through house Ethernet-to-DMX nodes, so the fixture rarely
> needs to speak Ethernet itself.

Hardware BOM
- [ ] Raspberry Pi (Ethernet-to-DMX bridge — only if venue requires)
- [ ] Ethernet cabling (Cat5/6) as required

Firmware / Software (std, on the Pi)
- [ ] Receive sACN / Art-Net over wired Ethernet (`artnet_protocol` / shared `sacn` codec)
- [ ] Bridge to fixture (forward to existing DMX / RF / direct-control stage)
- [ ] Map universes / addresses to fixture channels

Validation
- [ ] Test with venue console over wired Ethernet

### Build 7 — Remote Management

- [ ] Implement RDM (E1.20) — bidirectional discovery, addressing, status; codec via
      `dmx512-rdm-protocol` (`rdm`), transactions via `dmx-rdm` (rides the Build 2 transport)
- [ ] Implement GDTF — produce a GDTF fixture definition so consoles import the fixture
      with correct channel layout; evaluate Rust GDTF tooling, hand-author + validate if none fits
- [ ] Consider console-specific personality files (.d4 for grandMA, .ftf for ETC Ion, etc.)
- [ ] For (motion) sensors and other fixture→console feedback, look into OSC (`rosc`)

### Build 8 — Robust, Isolated Power Architecture

Target topology: **ground station → high station 1 (ESP + fixtures) → high station 2
→ high station 3 → DMX terminate.** Lead lengths up to **25 m** from base to high
station 3. Mounted on a **metal structure of uncertain earthing.**

#### Discussion — power & leads

**12V over 25 m is doable but voltage-drop-limited.** Loss is I²R; the current is set
by the LED array (the original fixture was 12V 2A, so ≈2A/station worst case until
measured). Round-trip conductor length to station 3 is 50 m (current up one wire,
back the other). Copper at ≈17.2 mΩ/m per mm², 2A, home-run:

| Conductor | R (50 m round trip) | Drop @ 2A | Drop % | Cable loss |
|-----------|---------------------|-----------|--------|-----------|
| 1.0 mm² | 0.86 Ω | 1.72 V | 14% | 3.4 W |
| 1.5 mm² | 0.58 Ω | 1.15 V | 9.6% | 2.3 W |
| 2.5 mm² | 0.35 Ω | 0.69 V | 5.8% | 1.4 W |
| 4.0 mm² | 0.22 Ω | 0.43 V | 3.6% | 0.86 W |

If power is **daisy-chained** (likely, tapping at each station), the base→station-1
trunk carries the sum (up to 6A for three stations) and drops accumulate toward
station 3 (≈1.5–1.7 V down, 6–7 W total in cable at 2.5 mm²). The trunk wants to be
fatter than the spurs (4 mm²+ on the first leg). The real constraint is not "does it
work" but **headroom and brightness matching**: if LEDs are voltage-driven straight
off the rail, station 3's sag shows as uneven brightness across the sculpture; if the
array has a constant-current driver, brightness holds flat down to the driver's
dropout and the sag stops mattering (ties to the §2.3 LED-driver question).

**The better architecture — distribute high, buck locally.** Distribute at 24V or
48V and step down to a clean local 12V at each station with a buck converter. Same
power means ¼ (24V) or 1/16 (48V) of the I²R loss, *and* every station gets an
identical regulated voltage regardless of chain position — which removes the
uneven-brightness problem at its root rather than papering over it. 48V also keeps
the distribution within extra-low-voltage / touch-safe territory.

**Capacitors buffer transients, not steady sag.** A local bulk capacitor at each
station is right — and valuable, because 25 m of cable has enough inductance that it
cannot deliver a fast load step (motor switching, PWM edges, simultaneous LED steps)
from the base; the local cap supplies it. Size from C = ΔI·Δt/ΔV (e.g. a 2A step over
~1 ms held to 0.5 V → ≈4000 µF), so a few thousand µF of low-ESR electrolytic +
ceramics per station; ESR matters (2A × 50 mΩ = 0.1 V step). **A capacitor cannot fix
the steady IR-drop sag** — it charges to the sagged rail and sits there, with no
surplus to give back. Steady drop is cured only by copper, lower current, or higher
distribution voltage. Large caps across three stations also create a power-on inrush
surge → add NTC / soft-start.

**Per-station isolation.** Three separately-powered nodes on a structure of uncertain
earth invites ground loops: the DMX signal common would otherwise tie all the local
grounds together and carry their potential differences, corrupting data and stressing
transceivers. Isolate each high station's DMX interface (integrated isolated RS-485
transceiver with integrated isolated DC-DC, e.g. ADM2587E) so data integrity becomes
independent of the structure's earth state — which we cannot guarantee and which may
change. Two sub-topologies:

- *Isolated tap:* one continuous bus through all stations, signal common carried
  end-to-end, terminated once at the far end. Simpler.
- *Isolated repeater:* each station regenerates DMX onto the next segment; each hop
  is point-to-point and terminated at its own receiver; kills long-bus common-mode
  accumulation. More parts, more robust over long/noisy runs. **Fail-through risk:** a
  regenerating node that depends on its MCU will break everything downstream if that
  MCU crashes — use a buffered/relay-bypass thru or an MCU-independent repeater.

**Grounding & cable.** Star-ground at each station (heavy load returns and the
transceiver reference meet at a single point). Run a **dedicated DMX signal common**,
separate from the power return, with **data+/data− as a twisted pair** — do not fold
the DMX common into the power return. Slow the MOSFET gate edges (the 100Ω gate
resistor helps) to cut di/dt, reducing both ground-bounce and EMI. The structure's
own **earthing and bonding is a safety matter for a qualified electrician**,
independent of signal logic.

> **Deferred decision:** the final **distribution voltage (12 / 24 / 48 V)** and
> **distribution method (daisy-chain vs home-run)** are left to decide another day —
> see the TODO below.

#### Hardware BOM (Build 8)

- [ ] Multi-conductor hybrid installation cable — baseline **5-conductor**
      (distribution V+, distribution return, DMX common, data+, data−) with data+/data−
      a **twisted pair**; cross-section per voltage decision (≥2.5 mm² if 12V; thinner
      if higher-V); trunk fatter than spurs if daisy-chained
- [ ] 3× isolated RS-485 transceiver with integrated isolated DC-DC (e.g. ADM2587E), one per high station
- [ ] 3× local buck converter (output 12V; input range per chosen distribution voltage) — count/spec pending voltage decision
- [ ] 3× bulk reservoir capacitor (low-ESR electrolytic, ~2200–4700 µF) + ceramics, one set per station
- [ ] Inrush limiting (NTC thermistor or active soft-start) per station feed and/or at the PSU
- [ ] TVS diodes at each station's power entry and on DMX data lines (cross-ref §3.5 / Build 5)
- [ ] 120Ω terminator — single far-end (tap topology) or per-segment (repeater topology)
- [ ] PSU sized for aggregate worst case (3× station current + cable losses) at the chosen distribution voltage
- [ ] Connectors rated for the chosen voltage / current
- [ ] Optional: fuse / PTC per station feed

#### Design decisions / TODO (Build 8)

- [ ] **Decide distribution voltage (12 / 24 / 48 V)** and **distribution method (daisy-chain vs home-run)** — *deferred*
- [ ] Resolve the §2.3 LED-driver question (constant-current vs voltage-driven) — determines whether sag affects brightness
- [ ] Once voltage chosen: finalise conductor cross-section and trunk/spur sizing from the loss table
- [ ] Choose isolation sub-topology: isolated tap vs isolated repeater
- [ ] If repeater: design fail-through (buffered / relay-bypass thru, or MCU-independent repeater)
- [ ] Size bulk capacitance per station from worst-case load step (C = ΔI·Δt/ΔV); pick low-ESR parts
- [ ] Specify inrush limiting for aggregate power-on surge
- [ ] Star-grounding scheme at each station PCB
- [ ] Hand structure earthing / bonding to a qualified electrician (safety, separate from signal)

#### Validation (Build 8)

- [ ] Measure voltage at each station under full simultaneous load; confirm within each buck's input range
- [ ] Confirm brightness uniformity across all three stations at full load
- [ ] Confirm DMX integrity with all motors + LEDs switching (worst-case EMI)
- [ ] Confirm power-on inrush does not trip the PSU
- [ ] Confirm data integrity is independent of structure earth (test structure bonded vs floating, if safe to do)
