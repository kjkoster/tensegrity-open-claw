# Tensegrity Open Claw — Design Document

## 1. System Overview

The system is split into two halves that communicate via sACN (E1.31) over a closed WiFi network:

- **Pi controller** — a Raspberry Pi running a Rust program on Linux. It generates 8 channels of independent Perlin noise, maps them to IRGBW DMX slots for two fixtures, and unicasts an sACN frame to each fixture at 44 Hz. The Pi also acts as a WiFi access point for the fixtures and is reachable from a development laptop via its Ethernet port.

- **Ponytail fixture** — a fibre-optic light fixture retrofitted with an ESP32-S3 (Seeed Studio XIAO). The MCU joins the Pi's WiFi as a station, subscribes to the configured sACN universe, and drives the LED array via LEDC PWM. A web config portal sets the DMX start address, universe, sACN port, and WiFi credentials. **The ponytail firmware is implemented.**

```
    ┌── Laptop ───────────────────────────────────────┐
    │  (SSH / cross-compile / web config browser)    │
    └────────────────── Ethernet ─────────────────────┘
                                  │
    ┌── Raspberry Pi ──────────────┴──────────────────┐
    │  Perlin noise engine                            │
    │  sACN sender (44 Hz) ────────── WiFi AP        │
    └─────────────────────────────────────────────────┘
                                  │ WiFi (sACN unicast)
                          ┌───────┴────────┐
                     ┌────┴─────┐    ┌─────┴────┐
                     │ Ponytail │    │ Fixture B│
                     │ ESP32-S3 │    │ ESP32-S3 │
                     └──────────┘    └──────────┘
```

### 1.1 DMX layout

Each fixture is IRGBW: Intensity on base address +0, then R, G, B, W. Two fixtures occupy one
universe, 10 slots. Fixture A is set to DMX start address 1, Fixture B to 6.

| Slot | Fixture | Channel   | Driven by                 |
|-----:|---------|-----------|---------------------------|
| 1    | A       | Intensity | audio loudness (Build 3)  |
| 2    | A       | Red       | Perlin (seed 0)           |
| 3    | A       | Green     | Perlin (seed 1)           |
| 4    | A       | Blue      | Perlin (seed 2)           |
| 5    | A       | White     | Perlin (seed 3)           |
| 6    | B       | Intensity | audio loudness (Build 3)  |
| 7    | B       | Red       | Perlin (seed 4)           |
| 8    | B       | Green     | Perlin (seed 5)           |
| 9    | B       | Blue      | Perlin (seed 6)           |
| 10   | B       | White     | Perlin (seed 7)           |

White rides Perlin like the other colour channels, but because W desaturates the mix it
typically wants scaling down — a per-channel output gain handles this without changing the
noise model. The Intensity slots use the silence-breathing function in Build 1 and are
overridden by audio loudness in Build 3.

---

## 2. Hardware

### 2.1 Ponytail fixture (fibre-optic, ESP32-S3)

A cheap, generic Chinese fibre-optic light fixture. It consists of plastic optic fibres lit
by an LED array, with a patterned metal plate rotated by a stepper motor in front of them.
It runs from a 12V 2A supply. The original controller board has been bypassed; an ESP32-S3
daughter board receives sACN over WiFi and drives one LED channel via PWM.

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
datasheets. Functional tracing beats datasheet hunting: a logic analyser on the ULN2803A
inputs reveals what signals control what, regardless of chip identity.

RF sniffing was attempted (nRF24L01+ in pseudo-promiscuous mode, sweeping all 126 channels
at 250 k/1 M/2 Mbps) and produced no usable data. The remote's RF chip (PL1167-family)
likely operates at 500 kbps, which the nRF24L01+ cannot demodulate. OTA emulation of the
remote is not possible with available hardware and is not pursued further.

**Open hardware questions (block Build 4 — direct control)**

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

sACN (E1.31, ANSI E1.31) is the only transport for this rig. It carries DMX-512 slots over
UDP and supports both multicast and unicast. The same frame is unicast to each address in a
configured fixture-IP list; this works regardless of any multicast group the fixture also
joins and requires no multicast routing infrastructure.

Art-Net is explicitly not supported: we control all devices on the network, so compatibility
with older rigs is not a requirement.

### 3.2 Fixture control paths

Two paths exist into the fixture hardware:

- **Wireless sACN receiver (current, Build 1):** the original board remains in the fixture;
  the ESP32-S3 receives sACN and drives a single LED channel via LEDC PWM through the
  ULN2803A. **This is implemented.**
- **Direct board replacement (Build 4):** replace the original board entirely and drive the
  RGBW LED array and 28BYJ-48 stepper directly from the ESP32-S3 for full per-channel
  control. Requires the hardware investigation in §2.1 first.

### 3.3 Software stack

**Ponytail (ESP32-S3):** `no_std` Rust on `esp-hal`, async via Embassy. LEDC PWM at ≥20 kHz
(video-safe, flicker-free). Inline sACN E1.31 decoder in `sacn.rs`; the `sacn` and
`sacn-unofficial` crates are `std`-only and not usable on bare metal. For wired DMX receive
(Build 5), standard `esp-hal` UART traits treat the >88µs DMX Break as a framing error;
interrupt-driven break detection via `pac` (`rxd_break` flag) will be needed.

**Pi controller:** standard Rust on Linux, `std`. sACN E1.31 encoder writing to a UDP socket.
Evaluate existing `sacn` / `sacn-unofficial` crates — both are `std`-only and would work
here; implement inline (see Appendix A) only if neither is suitable. `cpal` via ALSA for
audio capture in Build 3.

### 3.4 Power and signal separation

DMX cables are signal-only by convention; no competent engineer will patch an unknown cable
carrying power into their desk. Therefore:

- **At the desk interface:** standard 5-pin XLR (signal only) + a separate power feed
  (mains into the ground station).
- **From ground station to the installation:** a custom hybrid cable carrying power alongside
  the DMX pair — acceptable because it is our cable between our boxes, never presented to
  the desk.

### 3.5 Isolation and protection

Optoisolation between the desk's signal ground and the installation ground belongs in the
ground station (Build 6), not the fixture. The exact topology — optocoupler + isolated
DC-DC, or an integrated isolated RS-485 transceiver (e.g. ADM2582E) — is a Build 6
design decision.

Because the hybrid cable runs power alongside DMX data lines, a fray or short could
introduce supply voltage onto the ESP32's UART port. **TVS diodes must be added to the
DMX data lines inside the ponytail (Build 5) to protect the MCU.**

In the final multi-station chain (Build 8) there are several separately-powered nodes on a
structure of uncertain earth; per-station isolation is detailed there.

---

## 4. Build Plan

### Build 1 — Pi sACN sender + ponytail (first end-to-end system)

**The current target.** The Pi generates autonomous Perlin noise and unicasts sACN frames at
44 Hz to the ponytail. The ponytail firmware is already implemented; Build 1 is complete
when the full chain runs unattended: Pi up, ponytail connects, fixture glows with smoothly
drifting Perlin colour.

#### Network setup

- Pi runs in AP mode (hostapd + dnsmasq). Fixed SSID and channel per deployment.
- Ponytail connects to the Pi's AP as a WiFi station (configured via the web config portal).
- Laptop connects via Ethernet to the Pi for SSH and cross-compilation. No internet needed
  on the rig network.
- Static IPs for Pi and all fixtures; no DHCP surprises mid-show.

#### Noise generation

1-D gradient (Perlin) noise, one independent field per RGBW channel:

- **No permutation table.** The gradient at integer lattice cell `i` is derived by hashing
  the 64-bit cell index (splitmix64) mixed with a per-channel seed. The classic 256-entry
  table repeats every 256 cells; hashing the index pushes the repeat period past f64
  integer precision. Over a 7-day run at the default drift speed the field advances on the
  order of 10^5 cells — non-repeating with enormous margin.
- **Independence** comes from a distinct 64-bit seed per channel; all eight share one global
  drift speed but sample uncorrelated fields.
- Quintic fade and standard lerp. Optional fBm over a few octaves; default is a single
  octave.
- Output mapping: noise (~[−0.5, 0.5]) → 0..255 via a contrast gain, clamp, and perceptual
  gamma. A per-channel output gain allows trimming W.

#### Intensity breathing (silence default)

Intensity is not held static (a fixed level reads as dead) and never fully off (dark reads
as broken). The silence default is a slow "breathing" between a configurable floor and
ceiling over ≈20 s. Build 3 overrides this with audio loudness.

#### sACN sender

- Three E1.31 layers (root / framing / DMP), 0x00 DMX start code, 10 active slots.
- Stable CID generated once at startup.
- Increment-and-wrap sequence number; priority 100; configurable source name and universe.
- Deadline-based 44 Hz pacing (sleep to an absolute tick); a downed fixture logs an error
  without stalling the others.

#### Configuration (defaults)

| Parameter          | Default       | Meaning                                         |
|--------------------|---------------|-------------------------------------------------|
| `FIXTURE_IPS`      | (per install) | Unicast targets                                 |
| `SACN_PORT`        | 5568          | E1.31 UDP port                                  |
| `UNIVERSE`         | 1             | E1.31 universe                                  |
| `FRAME_RATE_HZ`    | 44            | Send rate                                       |
| `PRIORITY`         | 100           | E1.31 priority                                  |
| `NOISE_SPEED`      | 0.25 cells/s  | Baseline drift speed                            |
| `OCTAVES`          | 1             | fBm octaves                                     |
| `CONTRAST`         | 1.6           | Gain before clamp; higher = hits the rails more |
| `GAMMA`            | 2.2           | Perceptual mapping for output                   |
| `W_GAIN`           | (tune)        | Per-channel trim for White                      |
| `I_SILENCE_FLOOR`  | 0.35          | Breathing minimum (fraction of full)            |
| `I_SILENCE_CEIL`   | 0.65          | Breathing maximum                               |
| `I_SILENCE_PERIOD` | 20 s          | Breathing cycle length                          |

#### Open tasks — ponytail firmware

- [ ] **Bug:** `storage.write_dmx_config()` returns `FlashStorageError::OtherCoreRunning`
      when the write is attempted while running under the probe-rs debugger, but succeeds
      when attaching after boot. The ESP32-S3 flash driver refuses writes if Core 1 is
      active during an erase/write. Fix: retry on `OtherCoreRunning` in `flush()` since
      the condition is transient.
- [ ] Redo start-up logging: log firmware version, all config values, and assigned IP.

#### Open tasks — Pi sender

- [ ] Pi AP mode: configure hostapd + dnsmasq; assign static IPs.
- [ ] Evaluate `sacn` / `sacn-unofficial` crates for E1.31 encoding; implement inline
      (see Appendix A) only if neither is suitable.
- [ ] Implement Perlin noise engine (splitmix64 gradient hash, quintic fade, optional fBm).
- [ ] Implement sACN sender (deadline-paced 44 Hz loop, unicast to fixture IPs).
- [ ] Implement silence-breathing Intensity.
- [ ] Implement per-channel output gain (W trim).

#### Acceptance

- Fixture shows smooth, independently drifting RGBW colour and slow intensity breathing.
- No visible repeat across a multi-hour soak.
- 44 Hz holds steady; pulling the ponytail's power does not affect the Pi loop.
- Packets parse correctly on the ponytail (visible via config portal or RTT log).

---

### Build 2 — Breadboard installation

Validate the full two-fixture pipeline with simple RGB LEDs on breadboard MCUs representing
the sculpture fixtures. The Pi runs the Build 1 Perlin sender unchanged (no audio). Two
ESP32-S3 boards on breadboards each receive their assigned DMX address range and drive an
RGB LED, giving a visible, low-stakes confirmation that both sACN channels are working and
independent before dealing with the real fixture hardware.

#### Firmware changes

The current ponytail firmware drives a single DMX channel to a single LEDC output. This
build extends it to drive three LEDC channels (R, G, B) from the three consecutive DMX slots
at the configured start address:

- Slot offset +1 → Red LED (LEDC channel 1)
- Slot offset +2 → Green LED (LEDC channel 2)
- Slot offset +3 → Blue LED (LEDC channel 3)

Fixture A (start address 1) picks up R=slot 2, G=slot 3, B=slot 4.
Fixture B (start address 6) picks up R=slot 7, G=slot 8, B=slot 9.

The Intensity and White slots are not wired on the breadboard; they can be ignored until
Build 4.

#### Hardware BOM

| # | Item | Qty | Notes |
|---|------|----:|-------|
| 1 | Seeed Studio XIAO ESP32-S3 | 2 | Same MCU as the ponytail |
| 2 | Common-cathode RGB LED | 2 | One per board |
| 3 | 100Ω resistor | 6 | Current-limit per LED colour |
| 4 | Breadboard + jumper wires | 2 sets | |
| 5 | USB power supply / bench PSU | 2 | Power for each board |

#### Open tasks

- [ ] Extend ponytail firmware to drive 3 LEDC PWM channels (R, G, B) from consecutive DMX
      slots at the start address.
- [ ] Wire two breadboard setups: RGB LED + resistors + ESP32-S3.
- [ ] Configure Fixture A with start address 1, Fixture B with start address 6.
- [ ] Configure both boards to connect to the Pi's AP.
- [ ] Update Pi sender `FIXTURE_IPS` to list both board IPs.

#### Acceptance

- Both breadboard fixtures show smooth, independently drifting colour driven by their
  respective Perlin seeds.
- Fixture A and B drift independently — no visible synchronisation between them.
- Pulling power from one board does not disturb the other or the Pi loop.

---

### Build 3 — Audio integration (Pi)

Adds an audio thread to the Pi sender. Build 1 autonomous behaviour is the exact fallback
when audio is absent or silent.

#### Audio capture

- Input: USB audio interface (cheap class-compliant line-in box, e.g. UCA202-class).
- Take the feed through a passive ground-loop isolator (RCA transformer) to break hum and
  protect the Pi's USB interface from foreign grounds.
- Capture mono (sum channels) at 48 kHz via ALSA (`cpal` as the front end).
- Gain staging: USB interface input gain + software trim sits well below clipping; the AGC
  handles the rest.

#### DSP

Two concurrent activities share state and run at independent rates:

- An **audio thread** captures and analyses audio in short blocks and publishes a small set
  of control values (loudness, onset events, optional tempo). It is the only producer.
- The **render/send loop** (already running from Build 1) reads those values each tick.

Three DSP products, all cheap and real-time:

- **Loudness** — envelope follower on a bass band (≈40–160 Hz), fast attack (~20 ms),
  slower release (~150 ms), with a slow AGC (≈30 s) so the piece tracks dynamics rather
  than absolute level. Drives Intensity.
- **Onsets** — half-wave-rectified spectral flux off a short FFT (1024-point, ~256 hop),
  peak-picked against an adaptive threshold. Robust across genres. Drives the speed surge.
- **Tempo (optional)** — `aubio` tempo estimation, heavily smoothed and octave-clamped,
  used only to set a slow baseline drift speed.

#### Mapping

- **Loudness → Intensity.** Apply gamma, a minimum brightness floor (never fully dark), and
  a slew limit so brightness pulses without strobing. (Strobing is both ugly and a seizure
  concern for a public outdoor piece.)
- **Onset → speed.** Each onset injects an impulse into a decaying speed accumulator; the
  noise surges on each beat and relaxes between hits. Effective drift speed = baseline +
  accumulator, clamped to a sane range.

#### Silence fallback

Detect silence as input loudness below a noise floor for ≈5 s. **Crossfade** (≈2 s) control
values back to Build 1 defaults on silence; crossfade back on audio return. The render loop
is unaware of the source state.

#### Robustness

- USB audio device disconnect/reconnect reopens the stream and meanwhile behaves as silence.
- Audio xruns do not propagate into the render loop; it keeps sending at 44 Hz off the last
  published control values.

#### Hardware additions

| # | Item | Qty | Notes | Approx. unit |
|---|------|----:|-------|--------------|
| 1 | USB audio interface, line-in, class-compliant | 1 | UCA202-class RCA-input box | $30 |
| 2 | Passive ground-loop isolator (RCA) | 1 | Transformer isolation; breaks hum | $15 |

#### Open tasks

- [ ] Implement audio capture via `cpal` (ALSA, mono, 48 kHz).
- [ ] Implement bass-band envelope follower with slow AGC.
- [ ] Implement onset detection (spectral flux, adaptive threshold peak pick).
- [ ] Implement loudness → Intensity mapping with slew limit.
- [ ] Implement onset → speed accumulator (impulse and decay).
- [ ] Implement silence detection and crossfade.
- [ ] Verify: no strobe at any loudness level.

#### Acceptance

- With a feed: intensity tracks programme dynamics, colour drift surges on the beat, no
  strobing.
- Volume/track changes do not leave the piece stuck bright or dim (AGC works).
- Removing the feed crossfades smoothly to Build 1 behaviour within a few seconds; restoring
  it crossfades back.
- Unplugging the USB interface mid-show is survivable.

---

### Build 4 — Direct board replacement (ponytail)

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
- [ ] TVS diodes across DMX data lines to protect the ESP32

#### Firmware

- [ ] LEDC PWM for 4× LED channels at ≥20 kHz (video-safe).
- [ ] 28BYJ-48 stepper sequencing via ULN2003A (timer-driven).
- [ ] Map DMX channels: LED R, G, B, W, motor speed, motor direction.

#### Integration

- [ ] Design and prototype replacement PCB.
- [ ] Verify fitment in enclosure.
- [ ] Connect and test LED array.
- [ ] Connect and test stepper.
- [ ] End-to-end via Pi sACN sender.

---

### Build 5 — Wired DMX receive (ponytail)

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

### Build 6 — Ground station

A purpose-built box that takes a 5-pin XLR input from a desk and re-drives DMX + power out
to the installation cable. Optoisolates the desk's signal ground from the installation ground.

#### Hardware BOM

- [ ] Ground station enclosure
- [ ] 5-pin XLR input socket (desk side, female)
- [ ] 5-pin XLR output connector (ground station side, male → installation cable)
- [ ] 5-pin XLR input connector (installation side, female ← installation cable)
- [ ] RS-485 transceiver(s) for ground-station input/output re-drive
- [ ] Optocouplers / isolated RS-485 transceiver (e.g. ADM2582E)
- [ ] Locking mains power connector (e.g. Neutrik powerCON)
- [ ] Power supply (wattage = LED array + motor + logic; sizing finalised in Build 8)

#### Design

- [ ] Decide isolation topology: optocoupler + isolated DC-DC, or integrated isolated
      transceiver (e.g. ADM2582E).
- [ ] Schematic: XLR in → isolate → DMX + power onto installation cable.
- [ ] Verify isolated ground preserves DMX signal integrity.

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
| 1 | Raspberry Pi 4 (2 GB) | 1 | Pi 3B+ is a cooler-running alternative | $45 |
| 2 | High-endurance / industrial microSD (32 GB) | 1 | Cheap cards corrupt under days of writes | $12 |
| 3 | Pi heatsink / small fan | 1 | Thermal headroom inside a sealed box in sun | $8 |
| 4 | Pi power supply (official 5 V / 3 A or quality DC-DC) | 1 | Separate from fixture supply | $10 |
| 5 | Fixture power supply | per spec | Sized to the IRGBW fixtures | per spec |
| 6 | IP65 enclosure + cable glands | 1 set | Sized for Pi + audio interface + PSUs | $40 |
| 7 | Vent membrane / desiccant pack | 1 | Condensation management in sealed outdoor box | $8 |
| 8 | Mains inlet (IEC), fuse/breaker, surge protector | 1 set | Mains entry, fusing, transient protection | $25 |
| 9 | Ferrules, terminal blocks, DIN rail, drip-loop hardware | 1 set | Tidy, serviceable wiring | $20 |

The USB audio interface and ground-loop isolator from Build 3 are also housed in this
enclosure.

#### Power

- **Separate supplies.** Pi and fixtures on separate supplies. Never power fixtures from the Pi.
- **Mains entry.** Single IEC inlet, fused/breakered, with surge/transient protector.
- **Inrush and sizing.** Size fixture PSU with margin for inrush and full-white draw.
  Budget the Pi at ~5 V/3 A peak including USB interface.
- **Brown-out.** Under-voltage causes SD corruption and audio xruns; a quality supply and
  short, adequate-gauge DC runs matter.

#### Software service

- Systemd service, `Restart=always`, started on boot.
- Enable watchdog (systemd + hardware) so a hang reboots rather than freezing the piece.
- **Protect the SD card:** read-only root (overlayfs) or at minimum log to a RAM ring buffer.
  Days of writes to a writable root is a classic multi-day-install failure.
- Verify enclosure thermals in direct sun; throttling surfaces first as audio xruns.
- Condensation: sealed boxes sweat; the vent membrane and desiccant prevent internal dew.

#### Commissioning checklist

- [ ] Assign and record static IPs for Pi and all fixtures; configure Pi AP.
- [ ] Set fixture DMX start addresses and confirm universe matches.
- [ ] Bench-run Build 1: confirm smooth drift, breathing, steady 44 Hz, fixture-loss tolerance.
- [ ] If Build 3 is installed: connect a known feed, confirm intensity-on-loudness and beat
      surges, no strobe; pull the feed, confirm crossfade within a few seconds; restore.
- [ ] If Build 3 is installed: unplug/replug USB interface mid-run, confirm survival.
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

### Build 9 — Phantom power over DMX cable

Deliver 48V power to fixtures over the same 3-core cable as DMX data, eliminating a
separate power run. Visual constraints on the installation may force this choice.

#### Cable assignment

- Core 1 — ground / power return
- Core 2 — DMX data −
- Core 3 — DMX data + / 48V phantom

#### Implementation

- 48V phantom rides on the data cores.
- Blocking capacitors at each receiver decouple the DC from the RS-485 transceivers.
- Ground is shared between data return and power return.
- All devices on the rig are custom — no standard DMX equipment will be connected.

#### Constraints

- Non-standard, proprietary to this rig — must be documented at every junction.
- RS-485 transceivers must tolerate the common-mode voltage with caps in place.
- Voltage drop over long runs must be budgeted at design time.

---

### Build 10 — Remote management (OPTIONAL)

Makes the fixture behave like professional gear on a lighting console — addressable,
discoverable, with feedback. Only consider when a specific venue integration requires it.

- [ ] Implement RDM (E1.20) — bidirectional discovery, addressing, status; codec via
      `dmx512-rdm-protocol` (`rdm`), transactions via `dmx-rdm` (rides the Build 5
      wired-DMX transport).
- [ ] Implement GDTF — produce a GDTF fixture definition so consoles import the fixture with
      correct channel layout; evaluate Rust GDTF tooling, hand-author + validate if no crate
      fits.
- [ ] Consider console-specific personality files (.d4 for grandMA, .ftf for ETC Ion, etc.)
- [ ] For fixture→console feedback (motion sensors, etc.), look into OSC (`rosc`).

---

## Appendix A — sACN E1.31 data-packet layout (reference)

Single universe, 10 slots, 0x00 start code. Byte offsets (header is 126 bytes;
total = 126 + slots):

- Root: preamble `0x0010` @0; ACN ID `ASC-E1.17\0\0\0` @4..16; flags/len @16; root vector
  `0x00000004` @18..22; CID @22..38.
- Framing: flags/len @38; framing vector `0x00000002` @40..44; source name (64 B) @44..108;
  priority @108; sync addr @109..111; sequence @111; options @112; universe @113..115.
- DMP: flags/len @115; vector `0x02` @117; addr/data type `0xa1` @118; first prop addr
  `0x0000` @119..121; increment `0x0001` @121..123; property count (slots + 1) @123..125;
  start code `0x00` @125; DMX slots @126+.

Each flags/length field is `0x7000 | (bytes_to_end_of_packet & 0x0FFF)`.
