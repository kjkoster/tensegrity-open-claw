# Tensegrity Open Claw — Owned Devices

**What this file is for.** The physical hardware we own: the machines, the cabinet and what is
in it, the connectivity, the sensors, our own fixtures and motors — and which of them stand in for the
gear we rent. It is an inventory, not a manual: how to set the system up is
[`README.md`](README.md), how one show uses its fixtures is [`SHOW.md`](SHOW.md), and the
software components that drive these devices are in [`DESIGN.md`](DESIGN.md).

When we do a Tensegrity / Kultlab project we test with our own fixtures, and live we usually
rent pro fixtures via Kultlab. On a private project we are usually limited to what is owned
here, renting only where it really makes the project better.

Soldering, MCUs on breadboards, and opening devices up to hack or mod them are all on the
table — the ball's angle grinder is the example. The basic electronic components (resistors,
caps, various single-purpose chips) are on hand.

---

## Machines

| Machine | Details |
|---|---|
| `riverrain` | MacBook Air M2, local user `kjkoster`. The development machine: builds the wireless fixture firmware, runs QLC+, deploys to the Pi. |
| `eyeson` | MacBook Air 2015, dual-core Broadwell i5 at 1.6 GHz, no usable GPU, macOS Monterey (its last), no Ethernet port, local user `kjkoster`. Spare. |
| Spare Raspberry Pis | Several, at least one a Pi 4, some with more than 1 GB of RAM. |

## The cabinet

A Rittal ABS fibre box, about 60 × 60 × 20 cm, with a DIN rail to mount devices on.

| Device | Details |
|---|---|
| Door (the face) | Three buttons: an emergency stop (a twist-to-unlock smash button) and two green momentary pushbuttons, the left and right button, each with a green LED lamp. On the Pi's GPIO: e-stop on 23/24, buttons on 27/25, lamps on 13/12. |
| Raspberry Pi (`claw-pi`) | Raspberry Pi 3 Model B Plus Rev 1.3, 1 GB RAM (905 MB usable), on a DINner Plate Pi DIN rail holder, local user `kjkoster`. 64-bit Debian 13 (Trixie). A 6.9 GB root partition. Named `claw-pi` for historical reasons. |
| Raspberry Pi (`memory-pi`) | Raspberry Pi 3 Model B, 1 GB RAM, on a second DINner Plate Pi DIN rail holder, local user `kjkoster`. 64-bit Raspberry Pi OS Lite (Debian 13, Trixie) on a 64 GB SD card. On the WiFi access point at `10.0.10.2`. Powered from the USB hub's charge port. Records the rig's telemetry onto the external SSD. |
| DMX HAT | Zihatec RS422/485 HAT Rev D on the Pi's header: the wired DMX-512 output. |
| USB hub | 12 V powered, which turned out to be needed so as not to overload the Pi's USB ports. The modem and the audio interface plug into it, and its charge port powers `memory-pi`. |
| External SSD | Samsung T7, 500 GB (465.8 GiB), USB-C, with a DIN rail clip. On `memory-pi` through a USB-A to USB-C cable; one ext4 partition labelled `memory`. Holds the recorded MQTT telemetry. |
| Multiport USB charger | Durata, 100–240 V 50–60 Hz 0.8 A in, 5 V 4.4 A out at most across its four ports, each marked AUTO-ID: it detects what is plugged in and offers its fastest standard charging current, and a Pi simply takes 5 V. |
| PoE injector | Powers the camera. |
| Ethernet hub | On its own power supply. For when a laptop needs the Pi's wired segment beside the camera. |

## Connectivity

| Device | Details |
|---|---|
| WiFi access point | The Pi's own WiFi: SSID `closed claw DMX`, channel 6, from `etc/claw-wifi.env`. |
| 4G modem | Huawei E3372 USB stick (Vodafone-branded, USB vendor `12d1`) in MBIM mode, on the USB hub. |
| SIM | Odido 4G, 4 GB monthly data bundle. The Pi's address on Odido's network is publicly routable, with no DNS name. |
| WireGuard server | `wg` / `wg.kjkoster.org`, hosted on Google Cloud. The Pi (`10.8.0.3`) and the Mac (`10.8.0.4`) each keep a permanent tunnel to it, so the Mac reaches `claw-pi` at `10.8.0.3`. |
| USB Ethernet adapter | One; location unknown. |
| Ethernet leads | A long outdoor UTP lead for a camera in the yard, short leads, and spares. |
| DMX widget (the piggy) | Flying Pig Systems / High End HOG 3PC USB DMX widget. 5-pin DMX out, with a 5-pin to 3-pin pigtail for the mostly 3-pin fixtures. Used when a Mac drives the fixtures; the Pi's HAT is preferred. |

## Sensors and sound

| Device | Details |
|---|---|
| Audio interface | Alesis io\|2, a 2-channel USB AD/DA converter, on the USB hub. USB `0x13b2:0x0008`, ALSA card `io2`. Balanced main outs (TRS). |
| Powered speakers | Two Monkey Banana, on balanced leads. |
| Camera | Axis M3105-LVE outdoor mini dome, discontinued. Firmware 6.15.2, never updated: everything in the retina is verified on it. 115° × 64° field of view, up to 1920×1080. An 850 nm infrared illuminator reaching about 15 m, and a removable IR-cut filter. PoE class 2, from the PoE injector; IP66. MAC `ac:cc:8e:62:c1:35`; without a DHCP server it falls back to `192.168.0.90`. Factory reset: hold the control button while powering up, 15–30 s, until the status LED flashes amber; it comes back as `root`/`pass`, which the retina uses. |

## Fixtures

| Fixture | Details |
|---|---|
| Ball | A Seeed Studio XIAO ESP32-C3 on a battery-powered angle grinder, running off the tool's 20 V 4 Ah pack: a relay switches the grinder, and a white and a blue 3 W power LED dim through AL8860 drivers. Listens to sACN on the access point. Definition `fixtures/ball.qxf`. |
| HQ Power VDPLPS36B2 LED Pinspot PAR36 | One, very cheap RGB. Sold as a pinspot, it is a wash. Mode and address are DIP switches, not a menu — set the binary pattern from the datasheet. 3-channel RGB is all a show needs; 5-channel adds a strobe and macros the e-stop must then mask. Wired DMX. |
| UKing ZQ02015 moving heads | Four, cheap LED moving heads with a colour wheel, a gobo wheel (a rose among them) and LED strips on the body. Wired DMX. Address (`Addr`) and mode (`CHnd`) are set from the display menu. Front-panel settings live in each head and survive no reset: `rPAN` and `rTiL` `no`, or one head lands nowhere near the others; `Soud` `oFF`; `bLnd` never `Auto`, which starts the head's own program on signal loss. Their motors are loud: fine for a scare, too noisy for anything quiet up close. The definition is partly measured; the strobe band and the real pan and tilt travel are not. |
| JB Systems Space-4 laser | One. 8-channel mode. Needs its interlock plug fitted and the key turned on, or there is no output. Distrust the manual's DMX thresholds: keep channels in the 5–127 active band, out of the 0–4 dead zone. Wired DMX. |

## Motion

| Device | Details |
|---|---|
| Closed-loop stepper | One NEMA 23 integrated closed-loop stepper, labelled "JD57 Series Closed Stepper Motor, Voltage: 24–36 VDC input"; the label overrides the seller's 24–48 V and the manual's 20–50 V. Current is set at the factory, 4 A, with no switch for it. One 12-way 2.5 mm XH-type header, left to right: AL− AL+ EN− EN+ DR− DR+ PU− PU+ V− V− V+ V+, the power pins doubled. Six DIP switches: SW1–SW4 set the pulses per revolution, per the table on the body (1600 is ON, OFF, ON, ON); SW5 the direction (off counter-clockwise, on clockwise); SW6 is labelled 内部调试用，严禁打开, "internal debugging, do not switch on", and stays off. The only manual found, "JD57 Series Integrated Close-Loop Step Motor Manual (PULSE CONTROL)", is for a sibling revision whose SW6 sets the current and whose voltage range differs. From it: inputs 5–24 V at 7–16 mA; a pulse at least 2.5 µs high and 2.5 µs low; direction steady 5 µs before the pulse edge; 200 kHz at most. The alarm is an opto-isolated open collector, 30 V and 50 mA; it latches, and resets only when the pulses stop or the power is cycled, and a reset while pulses continue restarts the motor abruptly. No encoder output, no in-position output, no serial port: the alarm line is its only feedback. |

## Stand-ins for rented gear

We have nothing compatible with the rented fixtures, which makes testing awkward. These stand
in:

| Rented | Owned stand-in |
|---|---|
| CLF Yara (washes) | HQ Power VDPLPS36B2 pinspot |
| CLF Poseidon Laser Beam XS (moving heads) | UKing ZQ02015 |

