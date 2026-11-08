# Tensegrity Open Claw — Owned Devices

**What this file is for.** The physical hardware we own, and the facts about each device
itself: model, ratings, connectors, quirks, the settings that live in the device. How the
system is set up and uses them is [`README.md`](README.md); one show's use is
[`SHOW.md`](SHOW.md).

On a Tensegrity / Kultlab project we test with our own fixtures and usually rent pro fixtures
via Kultlab for the show; on a private project we are mostly limited to what is here. Soldering,
MCUs on breadboards and opening devices up to mod them are all on the table — the ball's angle
grinder is the example — and the basic electronic components are on hand.

---

## Machines

| Machine | Details |
|---|---|
| `riverrain` | MacBook Air M2, local user `kjkoster`. The development machine. |
| `eyeson` | MacBook Air 2015, dual-core Broadwell i5 at 1.6 GHz, no usable GPU, macOS Monterey (its last), no Ethernet port, local user `kjkoster`. Spare. |
| Spare Raspberry Pis | Several, at least one a Pi 4, some with more than 1 GB of RAM. |
| ESP32 DIN-rail controllers | Two Waveshare ESP32-S3-RS485-CAN-U: an ESP32-S3 with an RS-485 and a CAN interface, in a DIN-rail case. Not in use yet. |

## The cabinet

A Rittal ABS fibre box, about 60 × 60 × 20 cm, with a DIN rail.

| Device | Details |
|---|---|
| Door (the face) | An emergency stop (twist-to-unlock smash button, two contacts) and two green momentary pushbuttons, left and right, each with a green LED lamp. |
| Raspberry Pi (`claw-pi`) | Raspberry Pi 3 Model B Plus Rev 1.3, 1 GB RAM (905 MB usable), on a DINner Plate Pi DIN rail holder, local user `kjkoster`. 64-bit Debian 13 (Trixie), a 6.9 GB root partition. Named `claw-pi` for historical reasons. |
| Raspberry Pi (`memory-pi`) | Raspberry Pi 3 Model B, 1 GB RAM, on a second DINner Plate holder, local user `kjkoster`. 64-bit Raspberry Pi OS Lite (Debian 13, Trixie) on a 64 GB SD card. |
| DMX HAT | Zihatec RS422/485 HAT Rev D; DIP switches S1–S3 and a K2 terminal block, set and wired as in the README. |
| USB hub | 12 V powered, needed so as not to overload the Pi's USB ports; one charge port. |
| External SSD | Samsung T7, 500 GB (465.8 GiB), USB-C, with a DIN rail clip. |
| Multiport USB charger | Durata, 100–240 V 50–60 Hz 0.8 A in, 5 V 4.4 A out at most across four AUTO-ID ports: each offers the fastest standard charging current it detects, and a Pi simply takes 5 V. |
| PoE injector | For the camera. |
| Ethernet hub | On its own power supply. |

## Connectivity

| Device | Details |
|---|---|
| 4G modem | Huawei E3372 USB stick, Vodafone-branded, USB vendor `12d1`, in MBIM mode. |
| SIM | Odido 4G, 4 GB monthly data bundle. Its address on Odido's network is publicly routable, with no DNS name. |
| WireGuard server | `wg` / `wg.kjkoster.org`, a VM on Google Cloud. |
| USB Ethernet adapter | One; location unknown. |
| Ethernet leads | A long outdoor UTP lead, short leads, and spares. |
| DMX widget (the piggy) | Flying Pig Systems / High End HOG 3PC USB DMX widget. 5-pin DMX out, with a 5-pin to 3-pin pigtail. |

## Sensors and sound

| Device | Details |
|---|---|
| Audio interface | Alesis io\|2, a 2-channel USB AD/DA converter, USB `0x13b2:0x0008`. Balanced main outs (TRS). |
| Powered speakers | Two Monkey Banana, on balanced leads. |
| Camera | Axis M3105-LVE outdoor mini dome, discontinued, firmware 6.15.2 (never updated). 115° × 64° field of view, up to 1920×1080. An 850 nm infrared illuminator reaching about 15 m, and a removable IR-cut filter. PoE class 2; IP66. MAC `ac:cc:8e:62:c1:35`; without a DHCP server it falls back to `192.168.0.90`. Factory reset: hold the control button while powering up, 15–30 s, until the status LED flashes amber; it comes back as `root`/`pass`. |

## Fixtures

| Fixture | Details |
|---|---|
| Ball | A Seeed Studio XIAO ESP32-C3 on a battery-powered angle grinder, running off the tool's 20 V 4 Ah pack: a relay switches the grinder, and a white and a blue 3 W power LED dim through AL8860 drivers. |
| HQ Power VDPLPS36B2 LED Pinspot PAR36 | One, very cheap RGB. Sold as a pinspot, it is a wash. Mode and address are DIP switches, not a menu — set the binary pattern from the datasheet. 3-channel RGB, or 5-channel adding a strobe and macros. Wired DMX. |
| UKing ZQ02015 moving heads | Four, cheap LED moving heads with a colour wheel, a gobo wheel (a rose among them) and LED strips on the body. Wired DMX. Address (`Addr`) and mode (`CHnd`) are set from the display menu. Front-panel settings live in each head and survive no reset: `rPAN` and `rTiL` `no`, or one head lands nowhere near the others; `Soud` `oFF`; `bLnd` never `Auto`, which starts the head's own program on signal loss. Their motors are loud. The strobe band and the real pan and tilt travel are not measured. |
| JB Systems Space-4 laser | One. 8-channel mode. Needs its interlock plug fitted and the key turned on, or there is no output. Distrust the manual's DMX thresholds: keep channels in the 5–127 active band, out of the 0–4 dead zone. Wired DMX. |

## Motion

| Device | Details |
|---|---|
| Closed-loop stepper | One NEMA 23 integrated closed-loop stepper, labelled "JD57 Series Closed Stepper Motor, Voltage: 24–36 VDC input"; the seller says 24–48 V and the manual 20–50 V. Runs at 38 V: powered at 38 V the driver shows green and no over-voltage (one red flash), measured 2026-11-07. Current is set at the factory, 4 A, with no switch for it. Enable: with EN+ on 3.3 V, EN− open holds the shaft (enabled) and EN− to ground frees it (disabled), so the EN opto lit disables the driver; an input draws 11.7 mA from 3.3 V with its − pin grounded; with no fault, the alarm output rests open; at 38 V and 800 steps a revolution, with the shaft free, it follows a ramp of 200,000 steps/s² to 12,000 steps a second (15 revolutions a second) without an alarm (all measured 2026-11-07). One 12-way 2.5 mm XH-type header, left to right: AL− AL+ EN− EN+ DR− DR+ PU− PU+ V− V− V+ V+, the power pins doubled. Six DIP switches: SW1–SW4 set the pulses per revolution, per the table on the body (1600 is ON, OFF, ON, ON); SW5 the direction (off counter-clockwise, on clockwise); SW6 is labelled 内部调试用，严禁打开, "internal debugging, do not switch on", and stays off. The only manual found, "JD57 Series Integrated Close-Loop Step Motor Manual (PULSE CONTROL)", is for a sibling revision whose SW6 sets the current and whose voltage range differs. From it: inputs 5–24 V at 7–16 mA; a pulse at least 2.5 µs high and 2.5 µs low; direction steady 5 µs before the pulse edge; 200 kHz at most. The alarm is an opto-isolated open collector, 30 V and 50 mA; it latches, and resets only when the pulses stop or the power is cycled, and a reset while pulses continue restarts the motor abruptly. No encoder output, no in-position output, no serial port: the alarm line is its only feedback. |
| Motor supply | An adjustable DC supply, 38–62 V; it goes no lower than 38 V. Feeds the closed-loop stepper at its minimum, 38 V, two volts over the stepper's label, which the stepper takes (above). Kept at its minimum: the stepper's own over-voltage limit for this revision is not known. |

## Components

A partial list of a larger stock: parts are added here as they are looked for.

| Part | Details |
|---|---|
| Small-signal transistors, TO-92 | All bipolar, no MOSFETs among them. NPN: 2SC1815, 2SC945, S8050, MPSA42 (300 V), S9018 (RF), and one marked A93904, taken to be a 2N3904. PNP: 2N3906, 2SA1015, 2N5401 (150 V), S8550, S9015. Pinouts differ between types and makers; check with a multimeter's diode test before soldering. |
| Electrolytic capacitors | Mostly 16 V. A 4700 µF / 50 V (Jamicon `TLP478M050S1A5Q40L`, 22 × 40 mm) is on order for the motor's 38 V supply. |

## Stand-ins for rented gear

We have nothing compatible with the rented fixtures, which makes testing awkward. These stand
in:

| Rented | Owned stand-in |
|---|---|
| CLF Yara (washes) | HQ Power VDPLPS36B2 pinspot |
| CLF Poseidon Laser Beam XS (moving heads) | UKing ZQ02015 |
