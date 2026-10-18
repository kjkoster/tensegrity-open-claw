# Tensegrity Open Claw

Code and hardware configuration for the Tensegrity Open Claw sculpture. **This is the single
setup reference** — everything you must configure on the Pi and the fixtures lives here.

The system is three parts:

- **Pi controller (`claw-pi`)** — a Raspberry Pi running the `brain` daemon. Captures audio,
  runs the show, and emits one DMX universe **two ways** at 40 Hz: out the wired RS-485 HAT
  as raw DMX-512, and over the network as E1.31 sACN. The wire drives the wired fixtures; the
  sACN copy drives the ball, and is what a monitor or QLC+ works against.
- **Wired DMX** — the sculpture's fixtures on the HAT: a wash and a moving head per leg.
  What is patched where lives in `claw.qxw`, not in this file.
- **The ball** — an ESP32-C3 on the Pi's WiFi, following the sACN universe.

[`DESIGN.md`](DESIGN.md) holds the system's architectural decisions and what is not built
yet. It does not describe what the code does today — that is what this file and the code
itself are for.

---

# The Pi controller (`claw-pi`)

`brain` is Linux-only (links ALSA) and is **built on the Pi**, never cross-compiled from
macOS.

## Access

- SSH alias **`claw-pi`** → `10.8.0.3` over WireGuard, user `kjkoster`, key
  `~/.ssh/id_rsa` (`IdentitiesOnly yes`). The Mac is a peer at `10.8.0.4` routing a single
  `/32` and nothing else — not a general VPN. The tunnel's config is not versioned here.
- The Pi's interfaces, per `ifconfig` on `claw-pi`:

  | Interface | Address | Purpose |
  |---|---|---|
  | `eth0` | `10.0.1.1/24` | SSH / deploy from a directly-connected laptop |
  | `wlan0` | `10.0.10.1/24` | the WiFi AP |
  | `wg0` | `10.8.0.3/32` | WireGuard, for remote access over 4G |
  | `wwan0` | DHCP | 4G uplink |

  `wg0` is **POINTOPOINT with no MULTICAST flag**. A console reaching the Pi over the tunnel
  must therefore send sACN **unicast** to `10.8.0.3`; multicast only works on `eth0`/`wlan0`.

## Networking

### WiFi AP (fixtures join this)

- **AP mode via `hostapd` + `dnsmasq`** — fixed SSID/channel per deployment. Clients address
  `10.0.10.1`, leased from `10.0.10.10+`. `etc/hostapd.conf` is the only source of truth
  for the access point: every deploy installs it over the Pi's copy and restarts `hostapd`,
  which drops every WiFi client for a few seconds. The balls have the SSID and passphrase
  compiled in, read from that same file when the firmware is built, so changing either
  takes a deploy with every ball plugged in to be re-flashed.
- The **Ethernet port** is exposed to a dev laptop for SSH.

### 4G uplink

> ⚠️ **The link itself is not captured in the repo.** The deployed Pi reaches the internet over
> a 4G modem, and no config for the connection lives in this tree. To document: modem device +
> connection method (USB dongle vs. tethering; ModemManager/`mmcli`, `usb0` DHCP, or `ppp`),
> APN/SIM, routing metrics (4G as default route while the AP `10.0.10.0/24` and dev Ethernet
> `10.0.1.0/24` stay local), and boot-time bring-up so a reboot reconnects.

The **watchdog over that link is** in the repo, and `remote-deploy.sh` installs it:

| Repo | On the Pi |
|---|---|
| `etc/4g-watchdog.sh` | `/usr/local/sbin/4g-watchdog.sh` |
| `etc/4g-watchdog.service` | `/etc/systemd/system/4g-watchdog.service` |
| `etc/4g-watchdog.timer` | `/etc/systemd/system/4g-watchdog.timer` |

The timer runs the check every minute and is the only one of the two enabled — the service is
`oneshot` and exists to be fired. On a consecutive run of failed checks it escalates, at minute
3, 5 and 8: restart the connection, ask the HiLink dongle to reboot itself, cut the dongle's USB
power with `uhubctl`. **Each of those fires once**, and the gap to the next rung is what gives it
time to work — a dongle needs 30–60 s to register after a reset, so a remedy re-applied every
minute is one that never lets the dongle finish coming back.

At minute 13 it reboots the Pi, and that rung refuses itself three ways: within 15 minutes of
boot, while `IFACE` does not exist (no reboot fixes a renamed interface, and a warm reboot does
not cut USB power), and within an hour of the last watchdog reboot. That last guard is why the
timestamp lives in `/var/lib` — a reboot is precisely the event that clears `/run`, and without it
a campsite with no coverage reboots the rig every quarter of an hour, mid-show, for as long as the
outage lasts.

Separately, when the internet is up but the WireGuard tunnel is not, `wg-quick` is restarted.

```
systemctl list-timers 4g-watchdog.timer     # when it last ran and when it runs next
journalctl -t 4g-watchdog                   # what it has been doing
cat /run/4g-watchdog/consecutive-failures   # how far up the ladder it currently is
cat /var/lib/4g-watchdog/last-reboot        # epoch seconds of the last watchdog reboot
sudo /usr/local/sbin/4g-watchdog.sh         # run one check by hand
```

**The script's configuration lives inside the script**, at the top: the modem's interface, the
`uhubctl` hub and port, the tunnel address, and the thresholds. Those are the repo's copy to
change — a value edited on the Pi is overwritten by the next deploy. The hub and port in
particular have to be confirmed by eye, because a wrong pair exits 0 without cutting anything: run
`sudo uhubctl -l <loc> -p <port> -a cycle -d 5` and watch the dongle's LED go dark. The sysfs
unbind/rebind fallback follows that same port unless `USB_DEV` names another one, so the two
cannot disagree by accident — which they did, silently, because a Pi 4 gangs its USB ports and
cutting the wrong one cuts the dongle anyway.

## Attached hardware

**Addresses and modes are deliberately not listed here.** `claw.qxw` is the source of
truth and it changes; read them out of QLC+ and set each fixture to match. What this table
records is *how* you set them on each model, which does not change.

| Device | Interface | Notes |
|---|---|---|
| The ear (a USB audio interface) | USB (`plughw:CARD=io2,DEV=0`) | Any class-compliant interface; today an Alesis io\|2, only because it was the cheapest used one to be had. ALSA card name (confirm with `arecord -L`) and VID:PID (confirm with `lsusb`, today `0x13b2:0x0008`) are set in `cortex/src/config.rs`. Reports as `health/ear`, with `ear/state`, `ear/xruns`, `ear/clipping` (blocks at full scale since the brain started — turn the gain down), `ear/error` (why it is deaf, present only while it is), and `ear/usb/address`, `ear/usb/device` and `ear/usb/rate_hz`. |
| Zihatec RS422/485 HAT Rev D | 40-pin header + hardware UART | Wired DMX-512 output. |
| 3× CLF Yara LED par | DMX off the HAT | Address and mode from the front-panel LCD menu. Nothing in software can detect a wrong mode, so check each one against the patch. |
| 3× UKing ZQ02015 moving head | DMX off the HAT | Address and mode from the front-panel menu. Also set `bLnd` → `bLAc`, so signal loss blacks the head out instead of starting an auto or sound-active program. |
| ESP32-C3 MCUs (the balls) | USB serial/JTAG | Flashed by `deploy.sh` while plugged into the Pi; see [Ball fixtures](#ball-fixtures-esp32-c3). Not needed for `brain` itself. |
| Cabinet door (the face) | GPIO | E-stop on 23/24, left and right buttons on 27/25, their lamps on 13/12 through hardware PWM — see [Face lamps](#face-lamps-hardware-pwm). The lamps are patched as the `Face` fixture and follow the frame like any other. |

## OS configuration (once)

### Boot configuration

Every setting the rig needs in the boot configuration lives in
[`etc/config-claw.txt`](etc/config-claw.txt), with the reason for each beside it: the UART for
wired DMX, the HAT's transmit enable, the timing settings, onboard audio off, and the PWM
overlay for the face lamps. `config.txt` itself stays as Raspberry Pi OS ships it, apart from
one last line, `include config-claw.txt`.

`deploy.sh` copies the file to `/boot/firmware/` on every deploy and says when it changed. If
`config.txt` lacks the include line it appends it, and says so. It never reboots: a changed
boot configuration takes effect at the next `sudo reboot`, which is yours to choose.

### Serial / UART for wired DMX

The mini-UART (`ttyS0`) can't hold 250 kbaud, so the boot configuration moves the PL011 onto
the header; the login console has to let go of it too.

1. `raspi-config` → Interface → Serial: **login shell = No, hardware = Yes** (drops
   `console=serial0,115200` from `cmdline.txt`).
2. `sudo systemctl disable --now serial-getty@ttyAMA0.service hciuart`
3. Reboot, then confirm `ls -l /dev/serial0` → `ttyAMA0`.

`brain` opens `/dev/serial0` through the `zihatec-rs-485-dmx` crate, which preflights both of
these at startup and panics with the remediation if they're wrong.

### Face lamps (hardware PWM)

The two green lamps in the cabinet door's buttons dim through the Pi's two hardware PWM
channels, which the boot configuration routes to GPIO12 and GPIO13. Confirm with
`ls /sys/class/pwm/pwmchip0`. Without it `brain` still runs, logs `no PWM` for each lamp at
startup, and leaves the lamps dark.

### DMX timing hardening

Cheap DMX receivers are strict about framing. Three things went in — one essential, two
insurance:

- **Full 512-slot frame (the actual fix).** `brain` pads the wired frame to a full 512-slot
  universe; do not ship a short frame. Why a short one fails — and why it fails while still
  showing a healthy "DMX detected" — is in [`DESIGN.md`](DESIGN.md) §2.2.
- **`disable_pvt=1`** in the boot configuration — removes Broadcom firmware voltage/temperature timing
  jitter. Cheap, keep it.
- **Real-time scheduling** — `brain.service` sets `CPUSchedulingPolicy=fifo` /
  `CPUSchedulingPriority=50` so the kernel can't preempt the DMX loop mid-frame. The RT
  throttle still guarantees non-RT tasks progress. Confirm with `chrt -p $(pgrep -x brain)`.
- **`force_turbo=1`** *(optional)* — pins the CPU clock to remove frequency-scaling jitter.
  Added during debugging; since the 512-frame turned out to be the real fix, this can be
  removed to cut heat/power on a deployed unit. Confirm with `vcgencmd get_config force_turbo`.

### Clock

The Pi has **no RTC**, so it relies on `systemd-timesyncd` against the Debian NTP pool once
the network is up. Check with `timedatectl` (`System clock synchronized: yes`) or
`timedatectl timesync-status` for the server and offset.

This matters to deploys: cargo decides freshness by mtime, so a Pi clock lagging the Mac
would make every rsynced source look newer than every build artifact and rebuild the world.
`deploy.sh` used to push the Mac's time across to force agreement; that is gone now the Pi
keeps its own. Note there is still a window after each boot — before timesyncd reaches a
server — where the clock comes from `fake-hwclock` and is stale.

The brain does not wait out that window: it judges the [show times](#show-times) against
the clock as it stands, and corrects itself the frame after NTP steps it. Its log says which
clock it is on (`show: the clock is NTP-synchronised`, or `is not`), and prints the show
table again the moment that changes.

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

HAT output pigtail is **female**. Check each fixture's connector gender before assuming a
lead will mate — some makers reverse the usual convention. Put a **120 Ω terminator** in the
DMX **OUT** of the last device on the bus.

## systemd

`brain` runs as a service, installed and (re)started by `remote-deploy.sh`:

- Unit `brain/brain.service` → `/etc/systemd/system/brain.service` (`ExecStart=/usr/local/bin/brain`,
  `Restart=always`, `RestartSec=5`, `After=network.target`, RT scheduling as above).
- `sudo systemctl enable --now brain` / `sudo systemctl restart brain`.
- **Disabled** for the DMX serial to work: `serial-getty@ttyAMA0.service`, `hciuart`.
- The WiFi bring-up units belong here too once documented; the 4G watchdog's are above.

`remote-deploy.sh` also installs `stem.service`, the broker's `stem.conf` and
the 4G watchdog's unit and timer (above). Folding the 4G watchdog's job into Stem, so there is
one health daemon rather than two overlapping ones, is an open item in
[`DESIGN.md`](DESIGN.md).

## Build & deploy

From the Mac at the repo root:

```
./deploy.sh
```

Rsyncs to `claw-pi:tensegrity-open-claw/`, then runs `etc/remote-deploy.sh` there, which builds
`brain` natively, installs it as `/usr/local/bin/brain` along with the systemd unit, and
restarts the daemon.

Before any of that, `deploy.sh` builds the `ball` firmware on the Mac. Only the binary goes to
the Pi, as `tensegrity-open-claw/binaries/ball`, and the last thing `remote-deploy.sh` does is
flash it onto every board it finds under `/dev/serial/by-id`.

The cargo workspace is five crates:

| | |
|---|---|
| `cortex/` | everything under the show: capture, features, noise, both transports, the frame loop, the show calendar |
| `cortex-build/` | the QLC+ ingest and the calendar's resolution, a build-dependency of the rig crate |
| `brain/` | the claw: `claw.qxw`'s patch, its show times, and the show |
| `rig-mqtt/` | the MQTT conventions the brain and the ball share |
| `ball-dmx/` | what the ball's DMX values mean where its `.qxf` cannot say, shared by the brain and the ball |

The rig crate's `build.rs` names its workspace and reads `shows.rs`, the show table; its
`main.rs` includes the generated patch and calendar and writes the show.

Beside the crates, a daemon's systemd unit lives in that daemon's directory
(`brain/brain.service`, `stem/stem.service`), and what gets installed onto the Pi without
belonging to a daemon lives in `etc/`: the access point's configuration, the 4G watchdog and
`remote-deploy.sh` itself.

The deploy ships **only what the Pi builds from and installs** — the five crates,
`claw.qxw`, `fixtures/`, `stem/` and `etc/`. The list is written out in `deploy.sh`
rather than filtered by exclusions, so adding something is a decision; the design documents,
the reference PDFs and the git history stay on the Mac. The workspace goes because the build
compiles **its fixture patch and its scenes** in — so saving in QLC+ and deploying is all it
takes to get an edited scene onto the rig.

> The Pi built from `~/brain` before the split. That directory is dead; delete it once, by
> hand — the deploy leaves it alone rather than removing things on the Pi on its own.

### What the build does with the workspace

`cortex-build` — *Scene* — ingests a rig's `.qxw` at build time, called from that rig's
`build.rs`. Worth understanding, because it is why saving in QLC+ is the whole workflow, and
because every way it can go wrong is a failed build with a message rather than a wrong-looking
rig.

**What it reads.** Two sources of truth, neither of them Rust. The workspace says *where*
fixtures sit; the `.qxf` definitions in `fixtures/` say *what their channels mean*. Scene
reads both, resolves each patched fixture against its definition's mode, and generates a
`patch` module with a struct per fixture — a named field per channel — plus a `scenes` module,
into the rig crate that asked for them. The build reads every definition in `fixtures/` and
resolves only what the patch names.

**Why it happens at build time.** Ingesting rather than committing a generated file means
there is no stale copy to go stale. `cargo` is told to watch the workspace, so saving in QLC+
is enough to make the next build pick the change up.

**How scene values get resolved.** QLC+ stores them *fixture-relative*:

```
<FixtureVal ID="3">0,255,1,0</FixtureVal>
```

means "on the fixture whose QLC+ ID is 3, set its channel 0 to 255 and channel 1 to 0". Those
numbers are meaningless without the `<Fixture>` patch in the same file — which is exactly why
the patch is authoritative. Scene resolves them through it into absolute DMX slot indices,
the only thing the daemon speaks. Note `<Address>` in the file is **0-based**: DMX address 1
on the wire is `<Address>0</Address>`.

**Why channels are named, not numbered.** Because they become *named fields* rather than
offsets, drift is caught by the compiler in every direction:

| you do this in QLC+ | the build does this |
|---|---|
| patch a fixture nothing drives | unused constant → dead-code warning |
| delete or rename one the daemon drives | its constant is gone → every use site stops compiling |
| repatch to a mode lacking a channel the code writes | the field is gone, or the fixture stops implementing the capability trait the code requires |
| repatch so the channel count and the mode disagree | fails: "re-pick the mode in QLC+'s Fixture Manager so the two agree" |
| overlap two fixtures' slots | fails, naming both fixtures and their spans |
| name two fixtures so they yield the same Rust constant | fails — the daemon could not tell them apart |
| patch a fixture outside the first universe | fails — the daemon sends exactly one universe |
| change the E1.31 output's universe or port | the brain listens and sends there, and so does every ball once re-flashed |
| change the E1.31 output's unicast address | the brain's receiver binds that address instead |
| change the ball's channels or bands in `ball.qxf` | the brain and the ball firmware both follow, once re-flashed |
| patch two balls in different modes | fails — every board runs the same firmware, so they must share one |
| switch the E1.31 output to multicast | fails — the brain only receives a console by unicast |
| remove the E1.31 output from the first universe | fails — the universe, the port and the address come from it |
| delete or rename a ball a board is provisioned as | the ball firmware stops compiling at that board's row |
| reference a `.qxf` that is not committed | fails, naming the manufacturer and model |
| name a scene anything but `[a-z][a-z0-9_]*` | fails — scene names are the keys the daemon knows scenes by |
| give two scenes the same name | fails; QLC+ does not enforce uniqueness, the daemon needs it |

There is no drift checker because none is needed. A fixture with no red simply has no `red`
field, so the engine can only be wired to a fixture that genuinely has one.

**Every definition a patched fixture names must be committed** to `fixtures/`. The Pi builds
the daemon and has no QLC+ library to fall back on, so a missing `.qxf` fails the build
outright, with a message naming the manufacturer and model.

**Confirming it landed.** The startup log names the patch and the scenes the running binary
was built from:

```
brain: 8 fixtures
brain:   Wash red leg @ 100–103 — CLF Yara, 4 Channel
brain: 1 scenes
brain:   front_wash_warm (18 values)
```

Check the slot spans there after any repatch — that log, not the workspace, is what the
running binary actually believes.

### Show times

The rig plays only during the shows listed in `brain/shows.rs`: a `CITY`, and local
`(start, stop)` pairs in that city's time, in order. Edit it and deploy. Outside a show the
rig is black except for the ball's status LED.

During a show the ball's motor runs for one second every four minutes, the first run as the
show opens. From civil dusk to civil dawn its power LEDs pulse in silence and strobe to the
music.

The build looks the city up, finds its zone, and works out every show's instants and dark
stretches, all offline. It fails on:

| | |
|---|---|
| a city it does not know | fails, naming it |
| a city name more than one country has | fails, listing the `"City, Country"` spellings to use instead |
| a time it cannot read | fails, with the expected form, `"2026-10-21 13:00"` |
| a time in the hour the clocks skip, or the one they pass twice | fails — move it out of that hour |
| a stop that is not after its start | fails, naming the show |
| a show starting before the one above it ends | fails, naming the show |

**Confirming it landed.** The startup log lists the site and every show yet to end, with its
dark stretches, each time with its offset and season:

```
show: Amsterdam, Netherlands (52.3731°, 4.8999°), Europe/Amsterdam
show: the clock is NTP-synchronised
show: 3 of 3 shows to come
show:   2026-10-21 13:00 CEST (+02:00, summer time) → 2026-10-21 22:00 CEST (+02:00, summer time)
show:     dark 19:08 → 22:00
```

The brain publishes its state as `brain/scene` (`estop`, `blackout`, `daylight`, `pulse` or
`strobe`) and `brain/testmode`, retained, and a `show: scene …` or `show: test mode …` line in the
log on every change.

### Test mode

To try each scene without a console or waiting for dusk, use the cabinet door:

| | |
|---|---|
| hold both buttons for 2 s | test mode on, starting from the scene the rig is in; both lamps light |
| left button | the next scene: blackout → daylight → pulse → strobe → blackout |
| right button | the motor, for as long as it is held |
| hold both buttons for 2 s again | test mode off; the calendar has the rig again |

The calendar and the sun are ignored while it is on, but the audio is not: the strobe still
follows the room. The motor runs only while the right button is held, never on its beat,
and the ball's firmware still stops any run at 2 s. The e-stop still wins: the motor stops
while it is pressed. The buttons are published as
`face/left_button` and `face/right_button`. Test mode turns itself off after 15
minutes without a press, so a rig left in it cannot ignore the calendar for the rest of the
event.

---

# Ball fixtures (ESP32-C3)

An ESP32-C3 that joins the Pi's WiFi AP and listens for the sACN universe. The firmware is
the `ball/` crate: `no_std` Rust on Embassy, built on the Mac for
`riscv32imc-unknown-none-elf` with the stable toolchain. It is not a member of the cargo
workspace the Pi builds; build it from inside its own directory:

```
cd ball && cargo build --release
```

Every board gets the same binary and identifies itself at boot by its WiFi station MAC.

## Provisioning a new board

A board has to be known in two places, under two different MACs:

1. Plug it into the Pi and run `./deploy.sh`. It stops at the flash step with
   `unknown device usb-Espressif_…` — add that name to `DEVICE_MAP` in `remote-deploy.sh`,
   mapped to `ball`, and deploy again.
2. `./attach.sh` and read the log. An unprovisioned board prints its station MAC and the
   `Board` row to paste, then panics. Add the row to `BOARDS` in `ball/src/config.rs`, name
   the fixture it is in `claw.qxw` — `Ball` there is `patch::BALL` here — and deploy
   once more. The address, the universe and the port come with the fixture.

## Watching a board

```
./attach.sh            # the only board attached
./attach.sh 0          # by index in `probe-rs list`
./attach.sh AC:A7      # by any unique part of the probe's serial
```

Runs from the Mac and attaches `probe-rs` on the Pi, which is where the boards are plugged in.

---

# Front-panel settings on the moving heads

State that lives in each head rather than in this repo, and survives nothing: a head swapped,
reset or borrowed comes back with its own idea of these. Recorded here because a wrong one
does not look like a wrong setting — it looks like broken software.

| Menu | Setting | Why |
|---|---|---|
| `SLnd` | `Auto` | Follows the console. Verified on our ZQ02015s: they track DMX in this mode. |
| `rPAN` | `no` | Axis not reversed. |
| `rTiL` | `no` | Axis not reversed. |

`rPAN` and `rTiL` may be either way round, but **every head must agree**. A head with one
axis reversed tracks DMX perfectly and lands nowhere near where the others do — which reads
as broken aiming code, not as a menu.

---

# QLC+ on the Mac

QLC+ does two jobs here and nothing else: taking the rig over by hand, and programming
scenes. It is stock — no plugins, no patches.

The workspace is [`claw.qxw`](claw.qxw), and it is the **source of truth** for what the rig
has patched where. `cortex-build` reads
it at build time along with the `.qxf` definitions in `fixtures/`, so a change in QLC+ reaches
the rig by saving and deploying — there is no second copy of the patch in the Rust sources to
keep in step.

## Installing the fixture definitions (once, and after any change to `fixtures/`)

```
cp fixtures/*.qxf ~/Library/Application\ Support/QLC+/Fixtures/
```

Then **quit and relaunch QLC+** — it builds its definition cache at startup.

Check *Fixture Manager → Add fixture…* lists `CLF → Yara` and `Tensegrity → Ball`. QLC+ 5 skips definitions that fail to parse
**silently**, so absence is the error signal.

Do not put them inside `/Applications/QLC+.app`. That library is indexed by a manifest
(`FixturesMap.xml`) and ignores anything not listed in it, editing the bundle breaks its
signature, and an update wipes it.

`CLF-Yara.qxf` is ours, written from the manual, and the QLC+ library has no Yara at all —
**worth submitting upstream** to `mcallegari/qlcplus` once it has been confirmed against the
hardware. Open it in QLC+'s Fixture Editor and re-save first, so the file matches what their
tooling emits.

Every fixture the workspace patches needs its `.qxf` committed to `fixtures/` as well — the
Pi builds the daemon and has no QLC+ library to borrow from, so a missing one fails the
build with a message naming the manufacturer and model.

## Taking the rig over

The workspace's output is already configured: **unicast to `10.8.0.3:5568` at priority
200**, sent from the Mac's WireGuard address. Open the workspace and drive anything — Simple
Desk for raw channels, or a scene — and the rig is yours within a frame. The journal says so:

```
sacn: takeover by 10.8.0.4 cid=… priority=200
```

Stop, and control returns to the generative engine after the E1.31 data-loss timeout:

```
sacn: released by 10.8.0.4 … — internal engine resumes
```

Three behaviours that are correct but surprise you the first time:

- **QLC+ transmits the full universe.** Channels you have not touched are sent as 0, not
  left to the engine. Takeover means the generative look is gone until you release — knowing
  the exact state is usually the point.
- **There is a blackout between the output going live and your first value.** Raise
  something in Simple Desk, or enable a scene's live output, before enabling the plugin
  output if that matters.
- **Handback takes about 2.5 s, not one frame.** E1.31 has a stream-terminated bit that
  releases instantly, but QLC+ does not send it, so the timeout does the work.
- **A takeover beats the e-stop.** The e-stop only blacks out the generative engine;
  your universe goes out as you send it, e-stop pressed or not. A motor you run is yours:
  pull its fader down before you release the e-stop, or it carries on.

Takeover needs priority **strictly above 100**, which is what the daemon transmits at. That
is deliberate: every sACN source ships defaulted to 100, so a laptop joining the network
with a live universe cannot seize the rig by accident.

## Creating a scene

1. Take the rig over as above, so you are shaping real light rather than guessing.
2. Build the look in **Simple Desk**.
3. **Dump DMX values** into a new Scene.
4. Rename it to something `snake_case` — `front_wash_warm`. This is not cosmetic: scene
   names are the keys the daemon knows scenes by, and the build **fails** on a name that is
   not `[a-z][a-z0-9_]*`. QLC+'s default "New Scene 0" trips it, so renaming is forced
   rather than remembered.
5. **File → Save.**
6. `./deploy.sh`, then check the startup log names your scene:

   ```
   brain: 1 scenes
   brain:   front_wash_warm (18 values)
   ```

   That line is the confirmation the build ingested the save. Scenes have no runtime role
   yet — they are compiled in and logged, and nothing reads them in the frame loop.

Duplicate scene names fail the build too. QLC+ does not enforce uniqueness; the daemon needs
it, so it is enforced where it is cheap to fix.

## Editing a scene

1. **Function Manager**, open the scene in the **Scene Editor**.
2. Enable its **live-output toggle**. The rig goes to the scene's state and your edits move
   real fixtures — you are editing what you are looking at.
3. Tweak. The editor updates the Function in place.
4. **File → Save**, then deploy.

A **DMX dump always creates a new scene; it never updates an existing one.** Iterate in the
Scene Editor, and use dumps only for genuinely new looks — otherwise you end up with
`front_wash_warm` twice and a failed build telling you so.

Simple Desk fader positions are runtime-only. QLC+ does not persist them, so a look that
exists only on the desk is gone on reload. Persistent looks are Scenes.

## If nothing reaches the rig

In order of how often it is the cause:

- **The output interface.** QLC+ re-picks it when it loads a workspace and can land on
  loopback. It must be the Mac's WireGuard address; `UID` in the `.qxw` shows which one it
  chose.
- **The tunnel.** `ping 10.8.0.3`. The daemon binds that address alone, so if `wg0` is down
  at startup the bind fails and retries with backoff — the journal says
  `sacn: receive failed: Cannot assign requested address`.
- **Multicast.** It cannot work over WireGuard: `wg0` is POINTOPOINT with no MULTICAST flag.
  The output must be unicast.
- **Priority.** Anything at or below 100 is ignored by design.

---

# Datasheets

In `reference/`, each with a `.txt` conversion alongside it: `Datasheet RS485 HAT Rev D`,
`Application Note DMX512 Rev D`, `Manual-CLF-Yara-1.0`, `vdplps36b2vdplps36c2gbnlfresd`
(the HQ Power pinspot).
