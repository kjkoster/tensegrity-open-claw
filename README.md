# Tensegrity Open Claw

Code and hardware configuration for the Tensegrity Open Claw sculpture. **This is the single
setup reference** — everything you must configure on the Pi and the fixtures lives here.

The system is two parts:

- **Pi controller (`claw-pi`)** — a Raspberry Pi running the `brain` daemon. Captures audio,
  runs the generative engine, and emits one DMX universe **two ways** at 40 Hz: out the wired
  RS-485 HAT as raw DMX-512, and over the network as E1.31 sACN. The wire is what drives the
  rig; the sACN copy is there for monitoring and for QLC+ to work against.
- **Wired DMX** — a JB Systems Space-4 laser and three CLF Yara pars on the HAT.

Deep-dive companions: [`HARDWARE-DMX.md`](HARDWARE-DMX.md) (wired DMX/HAT rationale),
[`LASER.md`](LASER.md) (laser build phases), [`DESIGN.md`](DESIGN.md) (system design),
[`SPARKLE.md`](SPARKLE.md) (the light engine).

---

# The Pi controller (`claw-pi`)

`brain` is Linux-only (links ALSA) and is **built on the Pi**, never cross-compiled from
macOS.

## Access

- SSH alias **`claw-pi`** → `10.0.1.1`, user `kjkoster`, key `~/.ssh/id_rsa`
  (`IdentitiesOnly yes`).
- Two networks: `10.0.1.1` (SSH/dev) and `10.0.0.1` (the WiFi AP for the fixtures; DHCP
  `10.0.0.10+`, `/24`).

## Networking

### WiFi AP (fixtures join this)

- **AP mode via `hostapd` + `dnsmasq`** — fixed SSID/channel per deployment. Clients address
  `10.0.0.1`, leased from `10.0.0.10+`. The Pi's `hostapd.conf` is the only source of truth
  for the SSID and passphrase.
- The **Ethernet port** is exposed to a dev laptop for SSH.

### 4G uplink

> ⚠️ **Not yet captured in the repo.** The deployed Pi reaches the internet over a 4G modem,
> but no config for it lives in this tree. To document: modem device + connection method
> (USB dongle vs. tethering; ModemManager/`mmcli`, `usb0` DHCP, or `ppp`), APN/SIM, routing
> metrics (4G as default route while the AP `10.0.0.0/24` and dev Ethernet `10.0.1.0/24` stay
> local), and boot-time bring-up so a reboot reconnects.

## Attached hardware

| Device | Interface | Notes |
|---|---|---|
| Alesis io\|2 USB audio | USB (`plughw:CARD=io2,DEV=0`) | ALSA capture; confirm with `arecord -L`. VID:PID `0x13b2:0x0008`. Set in `brain/src/config.rs`. |
| Zihatec RS422/485 HAT Rev D | 40-pin header + hardware UART | Wired DMX-512 output. See below + `HARDWARE-DMX.md`. |
| JB Systems Space-4 laser | 3-pin XLR off the HAT | DMX address `025`, 8-channel mode. See below + `LASER.md`. |
| 3× CLF Yara LED par | DMX off the HAT | Addresses `100` / `107` / `113`, **4-channel mode** (R, G, B, White). The front-panel default is 11CH — set each one. |

## OS configuration (once)

### Serial / UART for wired DMX

The mini-UART (`ttyS0`) can't hold 250 kbaud, so move the PL011 onto the header and free it
from the login console (rationale in `HARDWARE-DMX.md`):

1. `/boot/firmware/config.txt`, add:
   - `dtoverlay=disable-bt` — makes `/dev/serial0 → ttyAMA0`.
   - `init_uart_clock=16000000` — lets 250000 baud divide cleanly.
   - `gpio=18=op,dh` — GPIO18 HIGH (transmit-enable) from boot, so the break code never
     touches the RS-485 direction line.
2. `raspi-config` → Interface → Serial: **login shell = No, hardware = Yes** (drops
   `console=serial0,115200` from `cmdline.txt`).
3. `sudo systemctl disable --now serial-getty@ttyAMA0.service hciuart`
4. Reboot, then confirm `ls -l /dev/serial0` → `ttyAMA0`.

`brain`'s `dmx_hat` sink preflights both of these at startup and panics with the remediation
if they're wrong.

### DMX timing hardening

The wired laser is a strict, cheap DMX receiver. Getting it to obey took three things — one
essential, two insurance:

- **Full 512-slot frame (the actual fix).** A short DMX frame makes this laser ignore the
  data and free-run its internal auto/sound show — with a *steady* "DMX detected" display, so
  it looks connected. `brain` pads the wired frame to a full 512-slot universe
  (`WIRED_FRAME_SLOTS`); do not ship a short frame to this fixture.
- **`disable_pvt=1`** in `config.txt` — removes Broadcom firmware voltage/temperature timing
  jitter. Cheap, keep it.
- **Real-time scheduling** — `brain.service` sets `CPUSchedulingPolicy=fifo` /
  `CPUSchedulingPriority=50` so the kernel can't preempt the DMX loop mid-frame. The RT
  throttle still guarantees non-RT tasks progress. Confirm with `chrt -p $(pgrep -x brain)`.
- **`force_turbo=1`** *(optional)* — pins the CPU clock to remove frequency-scaling jitter.
  Added during debugging; since the 512-frame turned out to be the real fix, this can be
  removed to cut heat/power on a deployed unit. Confirm with `vcgencmd get_config force_turbo`.

### Clock

The Pi has **no NTP/RTC** and its clock drifts. `deploy.sh` pushes the Mac's UTC time to the
Pi before each rsync so cargo's mtime freshness check stays sane.

## Zihatec HAT — DIP switches (manual DE/RE via GPIO18)

`brain` generates the DMX break itself, so use manual direction on GPIO18:

| Switch | 1 | 2 | 3 | 4 | Meaning |
|---|---|---|---|---|---|
| **S1** | OFF | ON | OFF | ON | DE/RE via **GPIO18** (S1.4); auto DE/RE (S1.3) OFF; GPIO18 HIGH = transmit |
| **S2** | OFF | OFF | ON | ON | Half-duplex: internal **Y→A, Z→B** (single pair on A/B) |
| **S3** | ON | OFF | ON | ON | Termination **ON** (HAT at bus end); 4k7 bias pull-down B / pull-up A |

### XLR wiring (K2 terminal block → 3-pin XLR)

| K2 | Signal | 3-pin XLR |
|---|---|---|
| A | data+ | pin 3 |
| B | data− | pin 2 |
| Shield | gnd/shield | pin 1 |

HAT output pigtail is **female**; the Space-4's DMX **input is 3-pin male** (JB reversed the
usual gender), output female. A standard male↔female DMX lead mates: male → HAT, female →
laser input. Put a **male 120 Ω terminator** in the laser's female DMX **OUT** (last device on
the bus).

## JB Systems Space-4 laser

- **Address `025`, 8-channel mode**, filling slots 25–32. Slots 1–24 are unused.
- Set on the front panel: FUNC until `1Ch`/`8Ch` → UP/DOWN to `8Ch` → ENTER → FUNC (address
  blinks) → UP/DOWN to `025` → ENTER.
- Fit the **interlock plug** (or spare shorting connector) and turn the **key** on, or there
  is no laser output.
- **Distrust the manual's DMX value thresholds** — sibling JB lasers misdocument them
  (Beglec's own support corrected the Lounge Laser manual by email, telling users to keep
  channels in a "5–127" active band and out of a 0–4 dead zone). `brain` accordingly drives
  CH1 to `255` (top of the DMX-mode band) and keeps CH7/CH8 positioning in `5–122`. Channel
  map and sweep live in `brain/src/config.rs` + `brain/src/laser.rs`. See `LASER.md`.

## systemd

`brain` runs as a service, installed and (re)started by `remote-deploy.sh`:

- Unit `brain.service` → `/etc/systemd/system/brain.service` (`ExecStart=/usr/local/bin/brain`,
  `Restart=always`, `RestartSec=5`, `After=network.target`, RT scheduling as above).
- `sudo systemctl enable --now brain` / `sudo systemctl restart brain`.
- **Disabled** for the DMX serial to work: `serial-getty@ttyAMA0.service`, `hciuart`.
- The 4G/WiFi bring-up units belong here too once documented.

## Build & deploy

From the Mac at the repo root:

```
./deploy.sh
```

Rsyncs `brain/` and `open-claw.qxw` to `claw-pi`, then runs `remote-deploy.sh` there, which
builds `brain` natively, installs `/usr/local/bin/brain` + the systemd unit, and restarts the
daemon.

The workspace goes with the sources because `brain/build.rs` reads it and compiles **both the
fixture patch and the scenes** in — so saving in QLC+ and deploying is all it takes to get an
edited scene onto the rig. The startup log names the scenes the running binary was built from.

The workspace is the **source of truth for fixture addressing**: there is no copy of it in the
Rust sources. Patch a fixture in QLC+ that nothing drives and the build warns it is unused;
delete or rename one the daemon drives and the build fails. Repatch one to a different mode
and the channel-count assertions at the fill sites stop the build too.

---

# Datasheets

In the repo root: `Datasheet RS485 HAT Rev D.pdf`, `Application Note DMX512 Rev D.pdf`,
`JB-Systems-Space-4-Laser.pdf`.
