# Tensegrity Open Claw — Owned Devices

**What this file is for.** The physical hardware we own: the machines, the cabinet and what is
in it, the connectivity, the sensors, and our own fixtures — and which of them stand in for the
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
| Raspberry Pi (`claw-pi`) | Raspberry Pi 3B+, 1 GB RAM, on a DINner Plate Pi DIN rail holder, local user `kjkoster`. Named `claw-pi` for historical reasons. |
| DMX HAT | Zihatec RS422/485 HAT Rev D on the Pi's header: the wired DMX-512 output. |
| USB hub | 12 V powered, which turned out to be needed so as not to overload the Pi's USB ports. The modem and the audio interface plug into it. |
| PoE injector | Powers the camera; its data port goes to the Pi's `eth0`. |
| Ethernet hub | On its own power supply. For when a laptop needs the Pi's wired segment beside the camera. |

## Connectivity

| Device | Details |
|---|---|
| WiFi access point | The Pi's own WiFi: SSID `closed claw DMX`, channel 6, from `etc/hostapd.conf`. |
| 4G modem | Huawei E3372 USB stick (Vodafone-branded, USB vendor `12d1`) in MBIM mode, on the USB hub. |
| SIM | Odido 4G, 4 GB monthly data bundle. The Pi's address on Odido's network is publicly routable, with no DNS name. |
| WireGuard server | `wg` / `wg.kjkoster.org`, hosted on Google Cloud. The Pi (`10.8.0.3`) and the Mac (`10.8.0.4`) each keep a permanent tunnel to it, so the Mac reaches `claw-pi` at `10.8.0.3`. |
| USB Ethernet adapter | One; location unknown. |
| Ethernet leads | Two for the camera — injector to the Pi, injector to the camera — and a spare. |
| DMX widget (the piggy) | Flying Pig Systems / High End HOG 3PC USB DMX widget. 5-pin DMX out, with a 5-pin to 3-pin pigtail for the mostly 3-pin fixtures. Used when a Mac drives the fixtures; the Pi's HAT is preferred. |

## Sensors and sound

| Device | Details |
|---|---|
| Audio interface | Alesis io\|2, a 2-channel USB AD/DA converter, on the USB hub. USB `0x13b2:0x0008`, ALSA card `io2`. Was the ear's input; is the mouth's output, from its main outs (TRS). |
| Powered speakers | Two, fed from the audio interface's main outs. |
| Camera | Axis M3105-LVE, firmware 6.15.2, MIPS, powered from the PoE injector, with its own infrared illuminator. At `192.168.0.90`, user `root`; the password is lost and is never written here, so it takes a factory reset (the control button held while powering up). On the wired segment it belongs at `10.0.1.90`. Streams H.264 over RTSP (`/axis-media/media.amp?videocodec=h264`) or MJPEG over HTTP (`/axis-cgi/mjpg/video.cgi`), configured over VAPIX. Was mounted inverted for the mage. The eyeball's input. |

## Fixtures

| Fixture | Details |
|---|---|
| Ball | A Seeed Studio XIAO ESP32-C3 on a battery-powered angle grinder, running off the tool's 20 V 4 Ah pack. A relay module switches the grinder (D5); a white and a blue 3 W power LED dim through AL8860 driver modules (D7, D10); a status LED on D6; dividers read the pack voltage (D1) and the grinder's C terminal (D2). Listens to sACN on the access point. Definition `fixtures/ball.qxf`. |
| HQ Power VDPLPS36B2 LED Pinspot PAR36 | One, very cheap RGB, 3- or 5-channel. Mode and address are DIP switches, not a menu — set the binary pattern from the datasheet. Wired DMX. |
| UKing ZQ02015 moving heads | Four, cheap LED moving heads with a colour wheel, a gobo wheel and LED strips on the body, 10- or 12-channel. 540° pan and 180° tilt by the library definition, not yet measured. Its `bLnd` setting decides what a head does on signal loss; on `Auto` it starts its own program. Wired DMX. |
| JB Systems Space-4 laser | One. 8-channel mode. Needs its interlock plug fitted and the key turned on, or there is no output. Distrust the manual's DMX thresholds: keep channels in the 5–127 active band, out of the 0–4 dead zone. Wired DMX. |

## Stand-ins for rented gear

We have nothing compatible with the rented fixtures, which makes testing awkward. These stand
in:

| Rented | Owned stand-in |
|---|---|
| CLF Yara (washes) | HQ Power VDPLPS36B2 pinspot |
| CLF Poseidon Laser Beam XS (moving heads) | UKing ZQ02015 |

## Not yet confirmed

- The Pi's model: this file says 3B+, `DESIGN.md` says 3B. `cat /proc/device-tree/model` on
  the Pi settles it.
- The makes and models of the Ethernet hub and its supply, the USB Ethernet adapter and the
  speakers, and where the adapter is.
