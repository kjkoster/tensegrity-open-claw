# Tensegrity Open Claw

Code and hardware configuration for the Tensegrity Open Claw sculpture. **This is the setup
reference for the system**: the Pi, the cabinet, its wiring, the network, and building and
deploying — whatever show it runs. Nothing here depends on which fixtures a show chose; that,
and the configuration and operation that come with them, belongs in [`SHOW.md`](SHOW.md).

The system is four parts:

- **Pi controller (`claw-pi`)** — a Raspberry Pi in the cabinet running the `brain` daemon.
  Reads its sensors, runs the show, and emits one DMX universe **two ways** at 40 Hz: out the
  wired RS-485 HAT as raw DMX-512, and over the network as E1.31 sACN. The wire drives the
  wired fixtures; the sACN copy drives the wireless ones, and is what a monitor or QLC+ works
  against.
- **Memory Pi (`memory-pi`)** — a second Raspberry Pi in the cabinet, on the first one's WiFi,
  recording everything published on the telemetry broker into a database on an SSD and
  showing it in Grafana.
- **Wired fixtures** — on the HAT's DMX line.
- **Wireless fixtures** — on the Pi's WiFi, following the sACN universe.

[`SHOW.md`](SHOW.md) holds the show: what the audience sees and why, and the fixtures it uses
and how each is set up. [`DESIGN.md`](DESIGN.md) holds where code goes and why: the
principles, the structure, a section per component. Neither describes what the code does
today — that is what this file and the code itself are for. [`OWNED-DEVICES.md`](OWNED-DEVICES.md)
is the inventory of the hardware we own, this setup's machines and cabinet among it.

---

# The Pi controller (`claw-pi`)

`brain` is Linux-only (links ALSA) and is **built on the Pi**, never cross-compiled from
macOS.

## Access

- SSH alias **`claw-pi`** → `10.8.0.3` over WireGuard, user `kjkoster`, key
  `~/.ssh/id_rsa` (`IdentitiesOnly yes`). The Mac is a peer at `10.8.0.4` routing a single
  `/32` and nothing else — not a general VPN. The tunnel's config is not versioned here.
- SSH alias **`memory-pi`** → `10.0.10.2` on the access point, user `kjkoster`, jumping through
  `claw-pi`. It has no WireGuard tunnel of its own, deliberately: if `claw-pi` dies there is
  precious little to do remotely anyway, so a second way in would buy nothing. The deploy and
  every SSH forward to it go the same way. In `~/.ssh/config`:

  ```
  Host memory-pi
      HostName 10.0.10.2
      User kjkoster
      ProxyJump claw-pi
  ```
- The Pi's interfaces, per `ifconfig` on `claw-pi`:

  | Interface | Address | Purpose |
  |---|---|---|
  | `eth0` | `10.0.1.1/24`, from `etc/claw-eth0.nmconnection`; DHCP for laptops from `.100`–`.199` | the wired segment: a network camera, and SSH / deploy from a laptop |
  | `wlan0` | `10.0.10.1/24`, from `etc/claw-ap.nmconnection`; DHCP for laptops from `.100`–`.199` | the WiFi AP |
  | `wg0` | `10.8.0.3/32` | WireGuard, for remote access over 4G |
  | `wwan0` | DHCP | 4G uplink |

  `wg0` is **POINTOPOINT with no MULTICAST flag**. A console reaching the Pi over the tunnel
  must therefore send sACN **unicast** to `10.8.0.3`; multicast only works on `eth0`/`wlan0`.

## Networking

### WiFi AP (fixtures join this)

- **NetworkManager runs every connection on both Pis**, the access point included, from
  profiles in the repo. The rig's WiFi — SSID, passphrase, channel — is defined once, in
  `etc/claw-wifi.env`; each Pi's deploy fills it into its profile templates, so the access
  point and its clients cannot disagree.
- **The access point** is `etc/claw-ap.nmconnection`, in NetworkManager's shared mode: the
  Pi at `10.0.10.1`, DHCP for laptops from `.100`–`.199`, DNS relayed, and every client routed
  out over 4G. Everything on it that must be found by address sets its own, below `.100`. Every
  deploy installs the profile and brings it up, which drops every WiFi client for a few
  seconds.
- The **Ethernet port** is the wired segment: a network camera through its PoE injector, and
  a laptop for SSH, with an Ethernet hub in the cabinet when both need it. It is
  `etc/claw-eth0.nmconnection`, in shared mode like the access point: the Pi at `10.0.1.1`,
  DHCP for a laptop from `.100`–`.199`, routed out over 4G. The camera's address, `10.0.1.3`,
  is a reservation for its MAC in `etc/claw-eth0.dnsmasq.conf`, which the deploy installs for
  NetworkManager's own DHCP server; the file lists every fixed address on the segment.

### 4G uplink

The modem is a Huawei E3372 USB stick (Vodafone-branded, vendor `12d1`) in MBIM mode: ModemManager
drives it as `cdc-wdm0` with network interface `wwan0`, and NetworkManager brings the link up as
its `cellular` connection. ModemManager can neither keep it off 2G (it offers one mode
combination only) nor report signal in dBm (no extended signal support), so Stem publishes its
0–100 signal quality instead.

> ⚠️ **The connection's configuration is not captured in the repo.** The APN, SIM and routing
> metrics (4G as default route while the AP `10.0.10.0/24` and the wired segment `10.0.1.0/24`
> stay local) live only in the Pi's NetworkManager profile,
> `/etc/NetworkManager/system-connections/cellular.nmconnection` — the one profile on the Pi
> the repo does not yet carry.

The **watchdog over that link is** in the repo, and `remote-deploy.sh` installs it:

| Repo | On the Pi |
|---|---|
| `etc/4g-watchdog.sh` | `/usr/local/sbin/4g-watchdog.sh` |
| `etc/4g-watchdog.service` | `/etc/systemd/system/4g-watchdog.service` |
| `etc/4g-watchdog.timer` | `/etc/systemd/system/4g-watchdog.timer` |

The timer runs the check every minute and is the only one of the two enabled — the service is
`oneshot` and exists to be fired.

It finds the modem by its USB vendor on every run, so no port, hub or interface needs
configuring; why, and why it never reboots or cuts power, is in [`DESIGN.md`](DESIGN.md) §3.2.

On a consecutive run of failed pings through the modem's interface it escalates, touching the
modem alone so the show is never disturbed: at minute 3 NetworkManager reconnects it, at minute 5
ModemManager resets it, and from minute 8 it is dropped off the USB bus and re-enumerated — a
replug without cutting power — again every 5 minutes for as long as the outage lasts. The first
two fire once, and the gap to the next rung is what gives each time to work: a modem needs
30–60 s to register after a reset.

Separately, when the internet is up but the WireGuard tunnel is not, `wg-quick` is restarted.

```
systemctl list-timers 4g-watchdog.timer     # when it last ran and when it runs next
journalctl -t 4g-watchdog                   # what it has been doing
cat /run/4g-watchdog/consecutive-failures   # how far up the ladder it currently is
sudo /usr/local/sbin/4g-watchdog.sh         # run one check by hand
```

**The script's configuration lives inside the script**, at the top: the modem's USB vendor, the
ping targets, the tunnel address and the thresholds. Those are the repo's copy to change — a value
edited on the Pi is overwritten by the next deploy.

## Attached hardware

What the cabinet holds whatever the show. The show's own fixtures, and how to set each one up,
are in [`SHOW.md`](SHOW.md).

| Device | Interface | Notes |
|---|---|---|
| The mouth (a USB audio interface) | USB on the hub (`plughw:CARD=io2,DEV=0`) | Any class-compliant interface; today an Alesis io\|2, only because it was the cheapest used one to be had. ALSA card name (confirm with `aplay -L`) and VID:PID (confirm with `lsusb`, today `0x13b2:0x0008`) are set in `mouth/src/config.rs`. Reports under `mouth/` through the brain's connection; the topics are in [`SHOW.md`](SHOW.md). |
| Zihatec RS422/485 HAT Rev D | 40-pin header + hardware UART | Wired DMX-512 output. |
| Cabinet door (the face) | GPIO | E-stop on 23/24, left and right buttons on 27/25, their lamps on 13/12 through hardware PWM — see [Face lamps](#face-lamps-hardware-pwm). The lamps are patched as the `Face` fixture and follow the frame like any other. Reports under `face/` through the brain's connection. |
| The retina (an Axis network camera) | `eth0`, through its PoE injector | At `10.0.1.3`, reserved for it by the Pi's DHCP server. The retina owns the camera: its user (`root`), password (`pass`, the factory default) and settings are in `retina/src/config.rs`, the retina sets them over VAPIX on every connect, and it reads the picture the eyeball asks for. Reports under `retina/` through the brain's connection. The eyeball only takes the retina's frames: it runs the detector, reports under `eyeball/`, and serves the annotated feed at **`http://10.8.0.3:8080/`** from the Mac — the WireGuard address alone, never the wildcard, which would show the camera's picture to the 4G uplink's public address. The topics of both are in [`SHOW.md`](SHOW.md). |
| Ethernet hub | `eth0`, own power supply | Only when a laptop needs the wired segment beside the camera. |

The retina and the eyeball run inside the brain, at normal priority below its real-time
threads, so a slow camera or a slow model costs the eyeball frames and never a DMX one; why,
and what it costs, is in [`DESIGN.md`](DESIGN.md) §1.6.

## Power

The cabinet has no off switch. Every Pi in it is never shut down, only loses power, so nothing
on the rig may depend on a clean shutdown. On `memory-pi` that costs at most the last ten
seconds of recordings: Greptime writes its log to the SSD every ten seconds, which is enough
for recordings read back in days. The SSD's ext4 replays its journal and is checked at boot
before it mounts.

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
`ls /sys/class/pwm/pwmchip0`. Without it `brain` stops at startup with `no PWM` and the lamp
it could not open.

### DMX timing hardening

Cheap DMX receivers are strict about framing. Three things went in — one essential, two
insurance:

- **Full 512-slot frame (the actual fix).** `brain` pads the wired frame to a full 512-slot
  universe; do not ship a short frame. Why a short one fails — and why it fails while still
  showing a healthy "DMX detected" — is in [`DESIGN.md`](DESIGN.md) §2.2.
- **`disable_pvt=1`** in the boot configuration — removes Broadcom firmware voltage/temperature timing
  jitter. Cheap, keep it.
- **Real-time scheduling** — the frame loop puts itself on core 3 at `SCHED_FIFO` 50, so the
  kernel can't preempt it mid-frame, and every other thread of the brain sets its own core
  and priority (`DESIGN.md` §1.6). The RT throttle still guarantees non-RT tasks progress.
  Confirm with `ps -L -o tid,psr,cls,rtprio,comm -p $(pgrep -x brain)`.
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

`remote-deploy.sh` also installs `stem.service`, the 4G watchdog's unit and timer (above), and
the access point's and wired segment's NetworkManager profiles, which it brings up (see
[Networking](#networking)). Stem's broker is not on this Pi: it runs on
[`memory-pi`](#the-memory-pi-memory-pi), at `10.0.10.2`, where the brain and Stem's daemon
report to it across the access point.

## Build & deploy

From the Mac at the repo root:

```
./deploy.sh
```

Rsyncs to `claw-pi:tensegrity-open-claw/`, then runs `etc/remote-deploy.sh` there, which builds
`brain` natively, installs it as `/usr/local/bin/brain` along with the systemd unit, and
restarts the daemon.

The cargo workspace is six crates; what goes in each is in [`DESIGN.md`](DESIGN.md) §2.3.

The deploy ships **only what the Pi builds from and installs** — the crates, `claw.qxw`,
`fixtures/`, `stem/` and `etc/`. The list is written out in `deploy.sh` rather
than filtered by exclusions, so adding something is a decision; the design documents, the
reference PDFs and the git history stay on the Mac. The
workspace goes because the build compiles **its fixture patch and its presets** in — so
saving in QLC+ and deploying is all it takes to get a re-aimed preset onto the rig. The
build compiles the recordings in too, as it does the eyeball's model, and both travel inside
their crates: the recordings in `brain/sounds/`, as they were found, since the build decodes
them itself.

> The Pi built from `~/brain` before the split. That directory is dead; delete it once, by
> hand — the deploy leaves it alone rather than removing things on the Pi on its own.

### What the build does with the workspace

`cortex-build` ingests a rig's `.qxw` at build time, called from that rig's `build.rs`. Worth understanding, because it is why saving in QLC+ is the whole workflow, and
because every way it can go wrong is a failed build with a message rather than a wrong-looking
rig.

**What it reads.** Two sources of truth, neither of them Rust. The workspace says *where*
fixtures sit; the `.qxf` definitions in `fixtures/` say *what their channels mean*. The
ingest reads both, resolves each patched fixture against its definition's mode, and generates
a `patch` module with a struct per fixture — a named field per channel — plus a `presets`
module, into the rig crate that asked for them. The build reads every definition in `fixtures/` and
resolves only what the patch names.

**Why it happens at build time.** Ingesting rather than committing a generated file means
there is no stale copy to go stale. `cargo` is told to watch the workspace, so saving in QLC+
is enough to make the next build pick the change up.

**How presets get resolved.** A preset is a QLC+ scene holding where every head points. QLC+
stores scene values *fixture-relative*:

```
<FixtureVal ID="3">0,255,1,0</FixtureVal>
```

means "on the fixture whose QLC+ ID is 3, set its channel 0 to 255 and channel 1 to 0". Those
numbers are meaningless without the `<Fixture>` patch and the definitions — which is exactly
why the patch is authoritative. The ingest resolves them through it into a pan and a tilt in
degrees per head, through each head's travel range, so `presets::LOOK_AT_BALL.head_red_leg`
is a pose. Note `<Address>` in the file is **0-based**: DMX address 1 on the wire is
`<Address>0</Address>`.

**The presets are a prescribed set.** `brain/presets.rs` lists the names the show is written
against — this show's are in [`SHOW.md`](SHOW.md) — and the workspace's scenes must be exactly
that set. A name becomes its constant the way a fixture name does: "Look at ball" →
`LOOK_AT_BALL`.

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
| change the E1.31 output's universe or port | the brain listens and sends there |
| change the E1.31 output's unicast address | the brain's receiver binds that address instead |
| switch the E1.31 output to multicast | fails — the brain only receives a console by unicast |
| remove the E1.31 output from the first universe | fails — the universe, the port and the address come from it |
| reference a `.qxf` that is not committed | fails, naming the manufacturer and model |
| patch a fixture whose shutter, Effect or Maintenance channel marks no off band | fails, naming the band the e-stop needs marked |
| add a scene that is not in `brain/presets.rs` | fails, naming it — and the preset it is most likely a typo of |
| delete or rename a preset's scene | fails, naming the preset that has no scene |
| record a preset without Pan, Pan Fine, Tilt or Tilt Fine for some head | fails, naming the preset, the head and the channel — a missing pan would be 0°, the end of the head's travel |
| record a preset with any other channel ticked | fails, naming the preset, fixture and channel — untick it in the Scene Editor |
| give two scenes the same name | fails; QLC+ does not enforce uniqueness, the daemon needs it |
| list two names in `brain/presets.rs` that yield the same Rust constant | fails |
| list a name in `brain/presets.rs` that yields `SUMMARIES` | fails — the presets module uses that name for its log lines |
| remove a name from `brain/presets.rs` the show still uses | its constant is gone → the use site stops compiling |

There is no drift checker because none is needed. A fixture with no red simply has no `red`
field, so the engine can only be wired to a fixture that genuinely has one.

**Every definition a patched fixture names must be committed** to `fixtures/`. The Pi builds
the daemon and has no QLC+ library to fall back on, so a missing `.qxf` fails the build
outright, with a message naming the manufacturer and model.

**Confirming it landed.** The startup log names the patch and the presets the running binary
was built from:

```
brain: 8 fixtures
brain:   Wash red leg @ 11–14 — CLF Yara, 4 Channel
brain: 32 e-stop overrides
brain: 4 presets
brain:   Look at ball — Head red leg 270.0°/119.5°, Head green leg 270.0°/119.5°, Head blue leg 270.0°/119.5°
```

A preset still at its old pose after you re-aimed and deployed means the build never saw
the save.

Check the slot spans there after any repatch — that log, not the workspace, is what the
running binary actually believes.

### Show times

The rig plays only during the shows listed in `brain/shows.rs`: a `CITY`, and local
`(start, stop)` pairs in that city's time, in order. Edit it and deploy. What the rig shows
during and outside a show is in [`SHOW.md`](SHOW.md).

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

The brain publishes its state, retained, on every change:

| topic | values |
|---|---|
| `brain/rig` | `boot_hold`, `running`, `estop` or `shutdown` — see [E-stop](#e-stop) |
| `brain/show` | the show state; the show's own states are in [`SHOW.md`](SHOW.md) |
| `brain/testmode` | whether test mode is on |

Rig-wide facts go under a root with their owner beneath it, retained: what each process runs
on under `sys/<owner>/` (`sys/brain/board`, `sys/stem/ram_kb`), and where each sits on the
universe under `dmx/<owner>/` (`dmx/brain/universe`, `dmx/brain/frame_rate_hz`,
`dmx/face/address`). `sys/#`, `dmx/#` and `health/#` each read the whole rig in one
subscription. A part inside the brain, such as the face, publishes its own subtree (`face/…`)
on the brain's connection and has no `health/` topic of its own.

The log has a `rig: …` line for each change of the rig state, and a `show: …` line for each
change of the show state.

### E-stop

The e-stop on the cabinet door is a **show-stop, not a power cut**. It acts over DMX only:
it takes the light away and stops what moves by itself, but every fixture stays powered and
nothing is isolated. Do not rely on it to remove a hazard. If a load ever needs a real cut,
switch a relay on its feed from the button's spare contact, not from a DMX channel.

While it is pressed, or its two contacts agree (a broken cable or a contact fault), the rig
holds the last frame it sent, with these overrides:

| channels | held at |
|---|---|
| light — dimmers, emitters, value / lightness, status LEDs, lamps | blackout (0) |
| shutter / strobe | closed if the fixture has it, otherwise open: strobe off |
| auto programs, macros, pattern selectors, resets, settings (Effect and Maintenance groups) | their *no function* band |
| anything that spins continuously | its *rotation stop* band |
| everything else — pan / tilt, speeds, wheels, gobos, beam, hue / saturation, CMY | where it was |

Not all zeros, because zero is only "off" for light. For a head it is the end of its travel,
for a speed channel often the fastest, for a wheel a slot, so an all-zero frame would send
every head to its corner at full speed. Holding everything but the light is QLC+'s Blackout
plus strobe, programs, resets and rotation, which Blackout leaves running.

- **It beats everything.** It acts on the frame as it goes out, so a console's takeover is
  stopped too (see [Taking the rig over](#taking-the-rig-over)).
- **The held frame goes out continuously** at the normal rate, so no fixture sees a lost
  signal.
- **Release is twisting the button out.** The live frame goes out again at once.
- **The button is read before anything goes out.** Booted with it released, the rig
  starts sending from the first frame. Booted with it pressed, the rig sends nothing until
  it is released: after a restart the brain does not know what the
  fixtures were doing, and silence leaves them as they were, where a held frame would be a
  guess.
- **The sound stops too.** The mouth is muted while the e-stop is pressed, at boot as well.
  A console takeover does not mute it: sound is not part of the universe.
- **The show does not know.** It runs on underneath the stop, so `brain/show` keeps
  reporting the show state it is in. The rig state is `brain/rig`; the button itself
  is `face/estop`.
- **The overrides are compiled from the fixture definitions.** A shutter, Effect or
  Maintenance channel whose `.qxf` does not mark the band the stop needs fails the build,
  naming the band to mark. Mark it in the definition; never guess a value.

The startup log counts the overrides, and the journal says what goes out and why:

```
brain: 32 e-stop overrides
rig: e-stop released — the frame goes out
rig: e-stop pressed — holding the last frame, 32 channels overridden
rig: e-stop pressed at boot — nothing goes out until it is released
rig: shutdown — releasing the sACN source (3 terminate frames)
```

Set every fixture's own **DMX-loss behaviour to hold last values**, so a cable fault behaves
like the stop rather than like a homing command.

---

# The memory Pi (`memory-pi`)

A Pi 3B in the cabinet that runs Stem's broker, records everything published on it into
GreptimeDB on an external SSD, and shows it in Grafana. Why it is built this way is in
[`DESIGN.md`](DESIGN.md) §3.9.

## Access

- SSH alias **`memory-pi`**, jumping through `claw-pi` (see [Access](#access) above): it has no
  WireGuard tunnel of its own.
- `wlan0` at **`10.0.10.2/24`**, set on the Pi itself by `etc-memory-pi/claw.nmconnection`, no
  DHCP. It is the Pi's only network; `claw-pi`'s access point routes it out over 4G.

## Setting up a fresh `memory-pi`

Headless, in the cabinet; a monitor is the last resort, after rewriting the card. The deploy
does the rest, so the card only has to get the Pi onto the access point once:

- **Write the card with Raspberry Pi Imager**: 64-bit Raspberry Pi OS Lite, hostname
  `memory-pi`, user `kjkoster` with the Mac's public key, SSH with key login only, WiFi
  `closed claw DMX` with its passphrase from `etc/claw-wifi.env`, and the country `NL`, without
  which the radio stays blocked.
- **Power it up with `claw-pi` already running**, so the access point is there for the first
  boot. It takes a DHCP address from `.100`–`.199`; find it on `claw-pi`:
  `sudo cat /var/lib/NetworkManager/dnsmasq-wlan0.leases`
- **Format the SSD**, once, if it is new or wiped — ext4, labelled `memory`, which erases it:
  `sudo mkfs.ext4 -L memory /dev/sda1`
- **Deploy** with the alias pointed at that DHCP address for now. The deploy installs the
  repo's WiFi profile, which takes `10.0.10.2` at the next boot, and everything else:
  `./deploy-memory-pi.sh`, then `ssh memory-pi sudo reboot`, then the alias back to
  `10.0.10.2`.
- **Remove Imager's network profiles**, which the repo's outranks but which would otherwise
  linger, including a DHCP one for `eth0`:
  `sudo rm /etc/netplan/90-NM-*.yaml && sudo netplan generate && sudo nmcli connection reload`

The first deploy downloads Greptime's release and the Grafana package over 4G; see
[Deploy](#deploy) below.

## What runs

| Unit | What | Installed from |
|---|---|---|
| `mosquitto.service` | Stem's broker, on port `1883` of every interface: loopback, and `10.0.10.2` for `claw-pi` and the access point | Debian's `mosquitto`, with `stem/stem.conf` in its include directory |
| `memory.service` | the health daemon: `health/memory`, `sys/memory/…`, `memory/…` | `etc-memory-pi/memory.py`, venv in `/opt/memory/venv` |
| `greptimedb.service` | GreptimeDB, data on `/tank/greptimedb`, on loopback alone: HTTP `4000`, gRPC `4001`, MySQL `4002` | the release pinned in `etc-memory-pi/greptime.version`, unpacked to `/opt/greptimedb` |
| `recorder.service` | every live message into Greptime's `rig` database | `etc-memory-pi/recorder.py`, venv in `/opt/recorder/venv` |
| `grafana-server.service` | Grafana on `127.0.0.1:3000` | Grafana's apt repository, with the settings drop-in and data source from `etc-memory-pi/grafana/` |

Every one of them reconnects for as long as it takes: both Pis power up together, and
`memory-pi` is up long before `claw-pi`'s access point. The broker comes first: Greptime and
Grafana run at `Nice=15`, Greptime's disk at idle priority, and `OOMScoreAdjust=500` has the
kernel kill one of them, not the broker, when memory runs out. They cannot be capped in
memory, because the Pi's kernel ships with the memory cgroup controller off
(`cgroup_disable=memory`); Grafana is kept small with `GOMEMLIMIT=200MiB` instead.

To watch the broker from the Mac, forward it over the jump:

```
ssh -L 1883:localhost:1883 memory-pi
mosquitto_sub -h localhost -v -t '#'
```

## Deploy

From the Mac at the repo root:

```
./deploy-memory-pi.sh
```

Rsyncs `etc-memory-pi/`, `etc/claw-wifi.env`, `etc/deploy-lib.sh` and Stem's `rig_mqtt.py`,
`rig_board.py` and `stem.conf` to `memory-pi:tensegrity-open-claw/`, then runs
`etc-memory-pi/remote-deploy.sh` there. It assumes a fresh Pi and removes nothing, and it
never touches `claw-pi`, as `deploy.sh` never touches `memory-pi`.

- **The first deploy is long.** Greptime's release (about 170 MB) and the Grafana package come
  over `claw-pi`'s 4G, which they fill: SSH to `claw-pi` times out while they download. Each
  is downloaded once — Greptime again only when its pin changes, Grafana only when it is
  missing.
- **Grafana upgrades with apt**, and a release is large and frequent. To choose when that
  happens: `sudo apt-mark hold grafana`, and `unhold` to upgrade.
- **A changed WiFi profile waits for a reboot**, because the deploy arrives over that very
  link: `ssh memory-pi sudo reboot`.

## The SSD on `/tank`

A Samsung T7, one ext4 partition labelled `memory`, mounted on `/tank` from the line in
`etc-memory-pi/fstab-tank`, which the deploy puts in `/etc/fstab`. `nofail` and a ten-second
device timeout let `memory-pi` boot without it; Greptime will not start without it. The deploy
never unmounts it: a changed line is applied with a remount, and a change of device waits for
a reboot.

## The recordings

One database, `rig`, kept forever. One table per topic's first level — `stem`, `memory`,
`brain`, `health`, `sys`, `recorder` — with the rest of the topic in the `topic` tag, numbers
in `value` (`true` and `false` as 1 and 0), words in `text`, and the time the recorder
received it in `greptime_timestamp`. Retained replays, empty payloads and anything before the
clock synchronised are not recorded, so a table appears once its topics first change live.

Is it recording:

```
ssh memory-pi journalctl -u recorder -n 20
ssh memory-pi "curl -s 'localhost:4000/v1/sql?db=rig' --data-urlencode 'sql=SHOW TABLES'"
```

`recorder/greptime` says `ok`, or why it cannot write; `recorder/written` counts what it
stored.

## Grafana

From the Mac:

```
ssh -L 3000:localhost:3000 memory-pi
```

then **`http://localhost:3000`**. Viewing needs no login — reaching it at all takes the SSH
key — and editing takes the admin login, `admin` / `admin` until Grafana asks for a new
password at the first sign-in. Its one data source, **Greptime**, is Greptime as a
MySQL server on `127.0.0.1:4002`, database `rig`; a panel is SQL with Grafana's time filter.
Both Pis' temperatures, for example, as a time series:

```sql
SELECT greptime_timestamp AS time, 'claw-pi' AS metric, value
FROM stem
WHERE topic = 'temperature_c' AND $__timeFilter(greptime_timestamp)
UNION ALL
SELECT greptime_timestamp AS time, 'memory-pi' AS metric, value
FROM memory
WHERE topic = 'temperature_c' AND $__timeFilter(greptime_timestamp)
ORDER BY time
```

The settings and the data source are provisioned from `etc-memory-pi/grafana/` on every
deploy. No dashboards are: one made in the UI is kept in Grafana's own database on
`memory-pi`'s SD card and survives deploys. Export it as JSON (**Share → Export**) for a copy
off the Pi.

---

# QLC+ on the Mac

QLC+ does two jobs here and nothing else: taking the rig over by hand, and aiming the heads'
presets. It is stock — no plugins, no patches.

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

Check *Fixture Manager → Add fixture…* lists every manufacturer and model in `fixtures/`.
QLC+ 5 skips definitions that fail to parse **silently**, so absence is the error signal.

Do not put them inside `/Applications/QLC+.app`. That library is indexed by a manifest
(`FixturesMap.xml`) and ignores anything not listed in it, editing the bundle breaks its
signature, and an update wipes it.

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

Four behaviours that are correct but surprise you the first time:

- **QLC+ transmits the full universe.** Channels you have not touched are sent as 0, not
  left to the engine. Takeover means the generative look is gone until you release — knowing
  the exact state is usually the point.
- **There is a blackout between the output going live and your first value.** Raise
  something in Simple Desk, or enable a scene's live output, before enabling the plugin
  output if that matters.
- **Handback takes about 2.5 s, not one frame.** E1.31 has a stream-terminated bit that
  releases instantly, but QLC+ does not send it, so the timeout does the work.
- **The e-stop beats a takeover.** While it is pressed the rig holds what it last showed,
  dark and still, whatever you send. Release it and your universe goes out again at once,
  heads moving to wherever you left them. A motor you run is stopped by the e-stop but
  restarts on release if its fader is still up: pull it down before you release.

Takeover needs priority **strictly above 100**, which is what the daemon transmits at. That
is deliberate: every sACN source ships defaulted to 100, so a laptop joining the network
with a live universe cannot seize the rig by accident.

## Aiming a preset

The workspace holds exactly the presets `brain/presets.rs` names, and no other scenes.
Each holds **only Pan, Pan Fine, Tilt and Tilt Fine, for every head** — the build fails on
anything else, so the dimmer you raised to see the beam must not be saved into it. The
placeholders in the workspace are rough; aim every one on the rig.

1. Take the rig over as above, so you are aiming real beams rather than guessing.
2. **Function Manager**, open the preset in the **Scene Editor**.
3. Enable its **live-output toggle**. In Simple Desk, raise each head's dimmer so you can see
   where it points — that dimmer is not part of the scene, and stays out of it.
4. Move the heads in the Scene Editor. It updates the Function in place.
5. **File → Save**, then deploy. The startup log lists the presets with every head's pan
   and tilt; check the one you re-aimed has moved.

Re-aim in the Scene Editor rather than dumping from Simple Desk. A **DMX dump always creates
a new scene, never updates one**, and it carries every channel you touched: the build then
fails on a scene that is not a preset, or on a preset with a dimmer in it. If a preset has to
be recorded from scratch, name it exactly as `brain/presets.rs` has it and untick every
channel but the four position channels of each head.

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

