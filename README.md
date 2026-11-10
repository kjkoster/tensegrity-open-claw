# Tensegrity Open Claw

Code and hardware configuration for the Tensegrity Open Claw sculpture. **This file is how to set
up and operate the system**, whatever show it runs: the Pis, the cabinet's wiring, the network,
the OS, building and deploying, and QLC+. What one show does, and its fixtures, are in
[`SHOW.md`](SHOW.md); where code goes and why, in [`DESIGN.md`](DESIGN.md); the hardware we
own, in [`OWNED-DEVICES.md`](OWNED-DEVICES.md).

The system is four parts:

- **`chaos-pi`** — a Raspberry Pi in the cabinet running the `brain` daemon. It reads its
  sensors, runs the show, and sends one DMX universe at 40 Hz two ways: out of the RS-485 HAT
  as wired DMX-512, and over the network as E1.31 sACN.
- **`memory-pi`** — a second Pi in the cabinet, on the first one's WiFi, running the telemetry
  broker and recording everything published on it.
- **Wired fixtures** — on the HAT's DMX line.
- **Wireless fixtures** — on `chaos-pi`'s WiFi, following the sACN universe.

---

# The Pi controller (`chaos-pi`)

`brain` is cross-built on the Mac for `aarch64-unknown-linux-musl`; the Pi gets only the
binary. Once per Mac: `rustup target add aarch64-unknown-linux-musl`.

## Access

- SSH alias **`chaos-pi`** → `10.8.0.3` over WireGuard, user `kjkoster`, key `~/.ssh/id_rsa`
  (`IdentitiesOnly yes`). The Mac is a peer at `10.8.0.4` routing that single `/32`, not a
  general VPN. The tunnel's config is not versioned here.
- SSH alias **`memory-pi`** → `10.0.10.3`, jumping through `chaos-pi` (why: DESIGN §3.9). In
  `~/.ssh/config`:

  ```
  Host memory-pi
      HostName 10.0.10.3
      User kjkoster
      ProxyJump chaos-pi
  ```

The Pi's addresses, as `clockwork.dot` sets them, where it calls the Pi `chaos-pi`:

| Interface | Address | Purpose |
|---|---|---|
| `eth0` | `10.0.1.1/24`, DHCP `.100`–`.199` | the wired segment, for a laptop |
| `wlan0` | `10.0.10.2/24`, DHCP `.100`–`.199` | the WiFi access point |
| `wg0` | `10.8.0.3/32` | WireGuard, remote access over 4G |
| `wwan0` | DHCP | 4G uplink |

`wg0` is point-to-point with no multicast, so anything reaching the Pi over the tunnel must
unicast.

## Networking

- **NetworkManager runs every connection on both Pis** from profiles in the repo. The
  profiles are templates: each Pi's deploy fills their `@…@` placeholders with what
  `clockwork.dot` says about that Pi — the WiFi's SSID, passphrase and channel, the addresses,
  gateways and DHCP ranges — using `clockwork/pi_network.py`, so each is written once, in the dot. A
  placeholder the dot does not fill stops the deploy before anything reaches the Pi.
- **The access point**, `pi/chaos-pi/claw-ap.nmconnection`, and **the wired segment**,
  `pi/chaos-pi/claw-eth0.nmconnection`, are both in shared mode: DHCP for laptops, DNS relayed, clients
  routed out over 4G. Anything that must be found by address sets its own, outside the DHCP
  range (DESIGN §2.6). A deploy that changes a profile brings it up again, which drops that
  network's clients for a few seconds.

### 4G uplink

The modem (OWNED-DEVICES) runs in MBIM mode: ModemManager drives it as `cdc-wdm0` with
interface `wwan0`, and NetworkManager brings the link up as its `cellular` connection.
ModemManager can neither keep it off 2G nor report signal in dBm, so Stem publishes its 0–100
signal quality.

> ⚠️ **The `cellular` profile is not in the repo.** Its APN, SIM and routing metrics (4G the
> default route, `10.0.10.0/24` and `10.0.1.0/24` local) live only in
> `/etc/NetworkManager/system-connections/cellular.nmconnection` on the Pi.

**The watchdog** is `pi/chaos-pi/4g-watchdog.sh`, installed to `/usr/local/sbin/` with its `.service`
and `.timer`; only the timer is enabled, running a check every minute. Its configuration — the
modem's USB vendor, ping targets, tunnel address, thresholds — is at the top of the script;
change the repo's copy, since the next deploy overwrites the Pi's. On consecutive failed pings
through the modem it escalates, touching the modem alone (why: DESIGN §3.2): at minute 3
NetworkManager reconnects it, at minute 5 ModemManager resets it, and from minute 8 it is
re-enumerated on the USB bus, again every 5 minutes for as long as the outage lasts. When the
internet is up but the tunnel is not, it restarts `wg-quick`.

```
systemctl list-timers 4g-watchdog.timer
journalctl -t 4g-watchdog
cat /run/4g-watchdog/consecutive-failures
sudo /usr/local/sbin/4g-watchdog.sh
```

## Attached hardware

What the cabinet holds whatever the show; the show's fixtures are in [`SHOW.md`](SHOW.md).

| Device | Interface | Notes |
|---|---|---|
| Zihatec RS422/485 HAT Rev D | 40-pin header + PL011 UART | wired DMX-512 out; see [the HAT](#zihatec-hat) |
| Cabinet door (the face) | GPIO (BCM) | e-stop on 23/24, left and right buttons on 27/25, their lamps on 13/12 through hardware PWM; patched as the `Face` fixture |

The cabinet has no off switch: the Pis are never shut down, only lose power, so nothing on
the rig may depend on a clean shutdown.

## OS configuration (once)

### Boot configuration

Every boot setting the rig needs is in [`pi/chaos-pi/config-claw.txt`](pi/chaos-pi/config-claw.txt), with the
reason for each beside it. `config.txt` stays as Raspberry Pi OS ships it apart from a last
line, `include config-claw.txt`. `deploy-chaos-pi.sh` copies the file to `/boot/firmware/` on every
deploy, appends the include line if it is missing, and says when either changed. It never
reboots: a changed boot configuration waits for your `sudo reboot`.

### Serial / UART for wired DMX

- `raspi-config` → Interface → Serial: **login shell = No, hardware = Yes**.
- `sudo systemctl disable --now serial-getty@ttyAMA0.service hciuart`
- Reboot, then confirm `ls -l /dev/serial0` → `ttyAMA0`.

`brain` checks both at startup and panics with the remediation if either is wrong.

### Face lamps

The boot configuration routes the two hardware PWM channels to GPIO12 and GPIO13. Confirm with
`ls /sys/class/pwm/pwmchip0`; without them `brain` stops at startup with `no PWM`.

### DMX timing

- **Full 512-slot frame** — the actual fix for cheap receivers; `brain` pads the wired frame
  (why: DESIGN §2.2).
- **`disable_pvt=1`** — removes firmware voltage/temperature timing jitter.
- **`force_turbo=1`** *(optional)* — pins the CPU clock; can go to save heat and power. Confirm
  with `vcgencmd get_config force_turbo`.
- **Real-time scheduling** — each thread of the brain sets its own core and priority (DESIGN
  §1.6). Confirm with `ps -L -o tid,psr,cls,rtprio,comm -p $(pgrep -x brain)`.

### Clock

The Pi has no RTC: `systemd-timesyncd` sets it from the Debian NTP pool once the network is up,
and until then it comes from `fake-hwclock` and is stale. Check with `timedatectl`. The brain
judges the [show times](#show-times) against the clock as it stands and corrects itself the
frame after NTP steps it; its log says `show: the clock is NTP-synchronised` (or `is not`) and
prints the show table again when that changes.

## Zihatec HAT

`brain` generates the DMX break itself, so direction is manual, on GPIO18:

| Switch | 1 | 2 | 3 | 4 | Meaning |
|---|---|---|---|---|---|
| **S1** | OFF | ON | OFF | ON | DE/RE via **GPIO18** (S1.4); auto DE/RE (S1.3) off; GPIO18 high = transmit |
| **S2** | OFF | OFF | ON | ON | half-duplex: internal **Y→A, Z→B** |
| **S3** | ON | OFF | ON | ON | termination **on** (HAT at bus end); 4k7 bias pull-down B / pull-up A |

| K2 | Signal | 3-pin XLR |
|---|---|---|
| A | data+ | pin 3 |
| B | data− | pin 2 |
| Shield | gnd/shield | pin 1 |

The HAT's pigtail is **female**; check each fixture's connector gender, since some makers
reverse the convention. Put a **120 Ω terminator** in the DMX out of the last device.

## systemd

- `brain/brain.service` → `/etc/systemd/system/brain.service`: `Restart=always`,
  `RestartSec=5`, `After=network.target`, and no scheduling settings — each thread sets its
  own.
- `sudo systemctl restart brain`; `journalctl -u brain -f`.
- **Disabled** for the DMX serial: `serial-getty@ttyAMA0.service`, `hciuart`.

The deploy also installs `stem.service`, the watchdog's unit and timer, and the network
profiles. Stem's broker runs on [`memory-pi`](#the-memory-pi-memory-pi), at `10.0.10.30`;
Stem's unit is a template too, its broker's address filled from Stem's MQTT edge in the dot.

## Build & deploy

From the Mac at the repo root: `./deploy-chaos-pi.sh`

It renders the Pi's network profiles and Stem's unit from `clockwork.dot` into
`target/chaos-pi/`, builds `brain` on the Mac, rsyncs what the Pi runs — the binary and its
unit, Stem's daemon and Python modules, `pi/chaos-pi/`, `pi/deploy-lib.sh` and the rendered files — to
`chaos-pi:tensegrity-open-claw/`, then runs
`pi/chaos-pi/remote-deploy.sh` there. That installs each file, and restarts a service only when one of
its files changed, saying which were left alone; a restart that fails stops the deploy with
an error. The list of what ships is written out in `deploy-chaos-pi.sh`, so adding something is a
decision.

### The boards on `chaos-pi`'s USB

Wrist's firmware is built on every deploy, with `wrist/build.sh` (`arduino-cli`, the core
pinned in `wrist/sketch.yaml`), and shipped to `binaries/wrist.bin`. Last, the remote deploy
finds every board on the Pi's USB by the MAC its chip reports and flashes it with the firmware
of the device whose `mac` that is in `clockwork.dot`; that restarts every board, flashed or
not. A board is written only when its image changed. `esptool` is Debian's, installed by the
deploy.

- **A new board** stops the deploy, naming its MAC: add `mac <mac> chip <chip>` to its
  device's address in `clockwork.dot`, the chip as esptool names it, e.g.
  `wifi 10.0.10.4 mac 5c:cf:7f:01:02:03 chip esp8266`, and deploy again. The deploy flashes it
  as that chip, and esptool refuses a board whose chip is another.
- **Its serial log**: `./attach.sh` from the Mac, or `./attach.sh <device>`. It finds the
  board the same way, restarts it so the log starts at boot, and runs until Ctrl-C.

### The patch

The build reads `clockwork.qxw` and the `.qxf` definitions in `fixtures/` and compiles the patch in,
so **saving in QLC+ and deploying** is how a repatch reaches the rig (why: DESIGN §1.1). Every
definition a patched fixture names must be committed to `fixtures/`; the build never reads
QLC+'s library. `<Address>` in the `.qxw` is 0-based: DMX address 1 is `<Address>0</Address>`.

| you do this in QLC+ | the build does this |
|---|---|
| patch a fixture nothing drives | unused constant → dead-code warning |
| delete or rename one the daemon drives | every use site stops compiling |
| repatch to a mode lacking a channel the code writes | the field is gone, or a capability trait is no longer implemented |
| repatch so the channel count and the mode disagree | fails: re-pick the mode in QLC+'s Fixture Manager |
| overlap two fixtures' slots | fails, naming both and their spans |
| name two fixtures so they yield the same Rust constant | fails |
| patch a fixture outside the first universe | fails |
| change the E1.31 output's universe or port | the brain listens and sends there |
| change the E1.31 output's unicast address or port | fails unless they are where `clockwork.dot` has the brain's sACN receiver bind |
| switch the E1.31 output to multicast | fails: the brain only receives a console by unicast |
| remove the E1.31 output from the first universe | fails |
| reference a `.qxf` that is not committed | fails, naming the manufacturer and model |
| patch a fixture whose shutter, Effect or Maintenance channel marks no off band | fails, naming the band the e-stop needs marked |

The startup log names the patch the running binary was built from; check the spans there after
any repatch:

```
brain: 2 fixtures
brain:   Face @ 1–2 — Tensegrity Face, 2 Channel
brain: 4 e-stop overrides
```

### Show times

The rig plays only during the shows in `brain/shows.rs`: a `CITY`, and local `(start, stop)`
pairs in order. Edit it and deploy. The build resolves the city's zone and every show's
instants and dark stretches offline, and fails on:

| | |
|---|---|
| a city it does not know | fails, naming it |
| a city name more than one country has | fails, listing the `"City, Country"` spellings |
| a time it cannot read | fails, with the expected form, `"2026-10-21 13:00"` |
| a time in the hour the clocks skip, or the one they pass twice | fails |
| a stop not after its start | fails, naming the show |
| a show starting before the one above it ends | fails, naming the show |

The startup log lists the site and every show yet to end:

```
show: Amsterdam, Netherlands (52.3731°, 4.8999°), Europe/Amsterdam
show: the clock is NTP-synchronised
show: 3 of 3 shows to come
show:   2026-10-21 13:00 CEST (+02:00, summer time) → 2026-10-21 22:00 CEST (+02:00, summer time)
show:     dark 19:08 → 22:00
```

### Topics

The brain publishes its state retained, and logs a `rig: …` or `show: …` line on each change:

| topic | values |
|---|---|
| `brain/rig` | `boot_hold`, `running`, `estop` or `shutdown` |
| `brain/show` | the show state, as [`SHOW.md`](SHOW.md) lists it |
| `brain/testmode` | whether test mode is on |

Rig-wide facts go under a root with their owner beneath it, retained: `sys/<owner>/…` (what
each process runs on) and `dmx/<owner>/…` (where each sits on the universe). `sys/#`, `dmx/#`
and `health/#` each read the whole rig. A part inside the brain, such as the face, publishes
its own subtree (`face/…`) on the brain's connection.

### E-stop

The cabinet's e-stop is a **show-stop over DMX, not a power cut**: every fixture stays powered.
If a load ever needs a real cut, switch a relay on its feed from the button's spare contact.

While it is pressed, or its two contacts agree (a cable or contact fault), the rig holds the
last frame it sent with these overrides (why: DESIGN §1.2):

| channels | held at |
|---|---|
| light — dimmers, emitters, value / lightness, status LEDs, lamps | 0 |
| shutter / strobe | closed if the fixture has it, otherwise open, strobe off |
| Effect and Maintenance groups — programs, macros, resets, settings | their *no function* band |
| anything that spins continuously | its *rotation stop* band |
| everything else — position, speeds, wheels, hue / saturation, CMY | where it was |

- It acts on the frame as it goes out, so it stops a [takeover](#taking-the-rig-over) too.
- The held frame goes out continuously at the normal rate.
- Twisting the button out releases it; the live frame goes out at once.
- Booted with it pressed, the rig sends nothing until it is released.
- The show runs on underneath: `brain/show` keeps reporting; `brain/rig` is the rig state and
  `face/estop` the button.
- A shutter, Effect or Maintenance channel whose `.qxf` marks no band for the stop fails the
  build. Mark it in the definition; never guess a value.

Set every fixture's own **DMX-loss behaviour to hold last values**, unless its section in
DESIGN says otherwise, so a cable fault behaves like the stop.

```
rig: e-stop released — the frame goes out
rig: e-stop pressed — holding the last frame, 32 channels overridden
rig: e-stop pressed at boot — nothing goes out until it is released
rig: shutdown — releasing the sACN source (3 terminate frames)
```

---

# The memory Pi (`memory-pi`)

Stem's broker, GreptimeDB recording everything published on it onto an external SSD, and
Grafana. Why it is built this way: DESIGN §3.9. Its only network is `wlan0` at
**`10.0.10.3/24`**, as `clockwork.dot` sets it, set on the Pi itself by
`pi/memory-pi/claw.nmconnection`.

## Setting up a fresh `memory-pi`

Headless, in the cabinet; the card only has to get it onto the access point once:

- **Write the card with Raspberry Pi Imager**: 64-bit Raspberry Pi OS Lite, hostname
  `memory-pi`, user `kjkoster` with the Mac's public key, key-only SSH, the WiFi of the
  `wifi` network in `clockwork.dot`'s `networks`, and country `NL`, without which the radio
  stays blocked.
- **Power it up with `chaos-pi` running.** It takes a DHCP address; find it on `chaos-pi`:
  `sudo cat /var/lib/NetworkManager/dnsmasq-wlan0.leases`
- **Format the SSD** if it is new or wiped, which erases it: `sudo mkfs.ext4 -L memory /dev/sda1`
- **Deploy** with the alias pointed at that DHCP address: `./deploy-memory-pi.sh`, then
  `ssh memory-pi sudo reboot`, then the alias back to `10.0.10.3`.
- **Remove Imager's network profiles**, including a DHCP one for `eth0`:
  `sudo rm /etc/netplan/90-NM-*.yaml && sudo netplan generate && sudo nmcli connection reload`

## What runs

| Unit | What | Installed from |
|---|---|---|
| `mosquitto.service` | Stem's broker, port `1883` on every interface | Debian's `mosquitto`, with `stem/stem.conf` in its include directory |
| `memory.service` | the health daemon: `health/memory`, `sys/memory/…`, `memory/…` | `pi/memory-pi/memory.py`, venv in `/opt/memory/venv` |
| `greptimedb.service` | GreptimeDB, data on `/tank/greptimedb`, loopback only: HTTP `4000`, gRPC `4001`, MySQL `4002` | the release pinned in `pi/memory-pi/greptime.version`, in `/opt/greptimedb` |
| `recorder.service` | every live message into Greptime's `rig` database | `pi/memory-pi/recorder.py`, venv in `/opt/recorder/venv` |
| `grafana-server.service` | Grafana on `127.0.0.1:3000` | Grafana's apt repository, settings and data source from `pi/memory-pi/grafana/` |

To watch the broker from the Mac:

```
ssh -L 1883:localhost:1883 memory-pi
mosquitto_sub -h localhost -v -t '#'
```

## Deploy

From the Mac at the repo root: `./deploy-memory-pi.sh`

It renders its WiFi profile from `clockwork.dot` into `target/memory-pi/`, rsyncs
`pi/memory-pi/`, the rendered profile, `pi/deploy-lib.sh` and Stem's `rig_mqtt.py`,
`rig_board.py` and `stem.conf`, then runs `pi/memory-pi/remote-deploy.sh` there. It assumes a
fresh Pi and removes nothing but what an older deploy shipped, and it never touches `chaos-pi`,
as `deploy-chaos-pi.sh` never touches `memory-pi`.

- **The first deploy is long**: Greptime's release (about 170 MB) and Grafana come over 4G,
  filling it so SSH to `chaos-pi` times out meanwhile. Greptime downloads again only when its
  pin changes, Grafana only when it is missing.
- **Grafana upgrades with apt.** To choose when: `sudo apt-mark hold grafana`, `unhold` to
  upgrade.
- **A changed WiFi profile waits for a reboot**, because the deploy arrives over that link.

## The SSD on `/tank`

One ext4 partition labelled `memory`, mounted on `/tank` from `pi/memory-pi/fstab-tank`, which
the deploy puts in `/etc/fstab`; after a power cut ext4 replays its journal and is checked at
boot before it mounts. The Pi boots without it; Greptime does not start without it.
The deploy never unmounts it: a changed line is applied with a remount, a change of device
waits for a reboot.

## The recordings

One database, `rig`, kept forever. One table per first topic level — `brain`, `face`, `dmx`,
`stem`, `memory`, `health`, `sys`, `recorder` and so on — with the rest of the topic in the
`topic` tag, numbers in `value` (`true`/`false` as 1/0), words in `text`, and the time
received in `greptime_timestamp`. A table appears once its topics first change live.

```
ssh memory-pi journalctl -u recorder -n 20
ssh memory-pi "curl -s 'localhost:4000/v1/sql?db=rig' --data-urlencode 'sql=SHOW TABLES'"
```

`recorder/greptime` says `ok` or why it cannot write; `recorder/written` counts what it stored.

## Grafana

`ssh -L 3000:localhost:3000 memory-pi`, then **`http://localhost:3000`**. Viewing needs no
login; editing takes `admin` / `admin` until Grafana asks for a new password. Its data source,
**Greptime**, is Greptime as a MySQL server on `127.0.0.1:4002`, database `rig`; a panel is SQL
with Grafana's time filter:

```sql
SELECT greptime_timestamp AS time, 'chaos-pi' AS metric, value
FROM stem
WHERE topic = 'temperature_c' AND $__timeFilter(greptime_timestamp)
UNION ALL
SELECT greptime_timestamp AS time, 'memory-pi' AS metric, value
FROM memory
WHERE topic = 'temperature_c' AND $__timeFilter(greptime_timestamp)
ORDER BY time
```

Settings and the data source are provisioned from `pi/memory-pi/grafana/` on every deploy. A
dashboard made in the UI lives in Grafana's own database on the SD card; export it as JSON
(**Share → Export**) for a copy off the Pi.

---

# QLC+ on the Mac

Stock QLC+, for two jobs: holding the patch in [`clockwork.qxw`](clockwork.qxw), and taking the rig over
by hand.

## Installing the fixture definitions (once, and after any change to `fixtures/`)

```
cp fixtures/*.qxf ~/Library/Application\ Support/QLC+/Fixtures/
```

Then **quit and relaunch QLC+**, and check *Fixture Manager → Add fixture…* lists every model
in `fixtures/`: QLC+ skips a definition that fails to parse silently. Do not put them inside
`/Applications/QLC+.app`: that library is indexed by a manifest, editing the bundle breaks its
signature, and an update wipes it.

## Taking the rig over

The workspace's E1.31 output unicasts at priority 200 to the address the brain binds, port
`5568`. Open the workspace and drive anything — Simple Desk for raw channels, or a scene — and
the rig is yours within a frame:

```
sacn: takeover by <source> cid=… priority=200
sacn: released by <source> … — internal engine resumes
```

- **Takeover needs priority strictly above 100**, the brain's own; a source at 100, every
  sACN default, is ignored (why: DESIGN §1.2).
- **QLC+ sends the full universe.** Channels you have not touched go out as 0; the show's look
  is gone until you release.
- **There is a blackout between the output going live and your first value.** Raise something
  before enabling the output if that matters.
- **Handback takes about 2.5 s.** QLC+ does not send E1.31's stream-terminated bit, so the
  data-loss timeout does the work.
- **The e-stop beats a takeover.** Release it and your universe goes out again at once. A
  fixture that moves by itself may resume on release, unless its firmware refuses, as Wrist's
  does.
- **Simple Desk faders are not saved.** A persistent look is a Scene.

## If nothing reaches the rig

- **The output interface.** QLC+ re-picks it when it loads a workspace and can land on
  loopback; `UID` in the `.qxw` shows which it chose. It must be the Mac's address on the
  network that reaches the brain's.
- **The route.** `ping` the address in the output. The brain binds that address alone; if it
  is not up at startup the bind retries with backoff, and the journal says
  `sacn: receive failed: Cannot assign requested address`.
- **Multicast.** The output must be unicast; it cannot cross WireGuard at all.
- **Priority.** 100 or below is ignored by design.
