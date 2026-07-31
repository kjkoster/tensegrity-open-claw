# Tensegrity Open Claw

Code and hardware configuration for the Tensegrity Open Claw sculpture. **This is the single
setup reference** — everything you must configure on the Pi and the fixtures lives here.

The system is two parts:

- **Pi controller (`claw-pi`)** — a Raspberry Pi running the `brain` daemon. Captures audio,
  runs the generative engine, and emits one DMX universe **two ways** at 40 Hz: out the wired
  RS-485 HAT as raw DMX-512, and over the network as E1.31 sACN. The wire is what drives the
  rig; the sACN copy is there for monitoring and for QLC+ to work against.
- **Wired DMX** — the sculpture's fixtures on the HAT: a wash and a moving head per leg,
  plus a pinspot. What is patched where lives in `open-claw.qxw`, not in this file.

Deep dives. None of them describe what the code does today — that is what this file and the
code itself are for:

- [`DESIGN.md`](DESIGN.md) — the system's architectural decisions, and what is not built yet.
- [`SHOW.md`](SHOW.md) — the show: moods, behaviours, and the pipeline that will drive them.
- [`PET-THE-DOG.md`](PET-THE-DOG.md) — watchdog layering for unattended operation.
- [`TODO.md`](TODO.md) — the empirical tests, and open items.

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
  `10.0.10.1`, leased from `10.0.10.10+`. The Pi's `hostapd.conf` is the only source of
  truth for the SSID and passphrase.
- The **Ethernet port** is exposed to a dev laptop for SSH.

### 4G uplink

> ⚠️ **Not yet captured in the repo.** The deployed Pi reaches the internet over a 4G modem,
> but no config for it lives in this tree. To document: modem device + connection method
> (USB dongle vs. tethering; ModemManager/`mmcli`, `usb0` DHCP, or `ppp`), APN/SIM, routing
> metrics (4G as default route while the AP `10.0.10.0/24` and dev Ethernet `10.0.1.0/24` stay
> local), and boot-time bring-up so a reboot reconnects.

## Attached hardware

**Addresses and modes are deliberately not listed here.** `open-claw.qxw` is the source of
truth and it changes; read them out of QLC+ and set each fixture to match. What this table
records is *how* you set them on each model, which does not change.

| Device | Interface | Notes |
|---|---|---|
| Alesis io\|2 USB audio | USB (`plughw:CARD=io2,DEV=0`) | ALSA capture; confirm with `arecord -L`. VID:PID `0x13b2:0x0008`. Set in `brain/src/config.rs`. |
| Zihatec RS422/485 HAT Rev D | 40-pin header + hardware UART | Wired DMX-512 output. |
| 3× CLF Yara LED par | DMX off the HAT | Address and mode from the front-panel LCD menu. Nothing in software can detect a wrong mode, so check each one against the patch. |
| 3× UKing ZQ-B243 moving head | DMX off the HAT | Address and mode from the front-panel menu. Also set `bLnd` → `bLAc`, so signal loss blacks the head out instead of starting an auto or sound-active program. |
| HQ Power VDPLPS36B2 pinspot | 3-pin XLR off the HAT | Mode *and* address are DIP switches, not a menu — set the binary pattern from the datasheet in `reference/`. |

## OS configuration (once)

### Serial / UART for wired DMX

The mini-UART (`ttyS0`) can't hold 250 kbaud, so move the PL011 onto the header and free it
from the login console.

1. `/boot/firmware/config.txt`, add:
   - `dtoverlay=disable-bt` — makes `/dev/serial0 → ttyAMA0`.
   - `init_uart_clock=16000000` — lets 250000 baud divide cleanly.
   - `gpio=18=op,dh` — GPIO18 HIGH (transmit-enable) from boot, so the break code never
     touches the RS-485 direction line.
2. `raspi-config` → Interface → Serial: **login shell = No, hardware = Yes** (drops
   `console=serial0,115200` from `cmdline.txt`).
3. `sudo systemctl disable --now serial-getty@ttyAMA0.service hciuart`
4. Reboot, then confirm `ls -l /dev/serial0` → `ttyAMA0`.

`brain` opens `/dev/serial0` through the `zihatec-rs-485-dmx` crate, which preflights both of
these at startup and panics with the remediation if they're wrong.

### DMX timing hardening

Cheap DMX receivers are strict about framing. Three things went in — one essential, two
insurance:

- **Full 512-slot frame (the actual fix).** `brain` pads the wired frame to a full 512-slot
  universe; do not ship a short frame. Why a short one fails — and why it fails while still
  showing a healthy "DMX detected" — is in [`DESIGN.md`](DESIGN.md) §2.2.
- **`disable_pvt=1`** in `config.txt` — removes Broadcom firmware voltage/temperature timing
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

- Unit `brain.service` → `/etc/systemd/system/brain.service` (`ExecStart=/usr/local/bin/brain`,
  `Restart=always`, `RestartSec=5`, `After=network.target`, RT scheduling as above).
- `sudo systemctl enable --now brain` / `sudo systemctl restart brain`.
- **Disabled** for the DMX serial to work: `serial-getty@ttyAMA0.service`, `hciuart`.
- The 4G/WiFi bring-up units belong here too once documented.

`brain.service` is the only unit `remote-deploy.sh` installs. The health daemon
(`stem.service`), the local MQTT broker and the systemd watchdog settings are designed in
[`SHOW.md`](SHOW.md) and [`PET-THE-DOG.md`](PET-THE-DOG.md) but **not built** — when they
land, how to configure them belongs in this file. There is also a 4G watchdog script running
on the Pi that is not in the repo at all; getting it versioned is an open item in
[`TODO.md`](TODO.md).

## Build & deploy

From the Mac at the repo root:

```
./deploy.sh
```

Rsyncs `brain/` and `open-claw.qxw` to `claw-pi`, then runs `remote-deploy.sh` there, which
builds `brain` natively, installs `/usr/local/bin/brain` + the systemd unit, and restarts the
daemon.

The workspace goes with the sources because the build compiles **both the fixture patch and
the scenes** in — so saving in QLC+ and deploying is all it takes to get an edited scene onto
the rig.

### What the build does with the workspace

`brain/build.rs` — *Scene* — ingests `open-claw.qxw` at build time. Worth understanding,
because it is why saving in QLC+ is the whole workflow, and because every way it can go wrong
is a failed build with a message rather than a wrong-looking rig.

**What it reads.** Two sources of truth, neither of them Rust. The workspace says *where*
fixtures sit; the `.qxf` definitions in `fixtures/` say *what their channels mean*. Scene
reads both, resolves each patched fixture against its definition's mode, and generates a
struct per fixture into `patch.rs` — a named field per channel, `DMX_SLOTS` included.

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
brain: 7 fixtures from open-claw.qxw
brain:   pinspot @ 1–5 — HQ Power VDPLPS36B2 LED Pinspot PAR36, 5 Channel
brain: 1 scenes from open-claw.qxw
brain:   front_wash_warm (18 values)
```

Check the slot spans there after any repatch — that log, not the workspace, is what the
running binary actually believes.

---

# QLC+ on the Mac

QLC+ does two jobs here and nothing else: taking the rig over by hand, and programming
scenes. It is stock — no plugins, no patches.

The master workspace is [`open-claw.qxw`](open-claw.qxw), and it is the **source of truth**
for what is patched where. `brain/build.rs` reads it at build time along with the `.qxf`
definitions in `fixtures/`, so a change in QLC+ reaches the rig by saving and deploying —
there is no second copy of the patch in the Rust sources to keep in step.

## Installing the fixture definitions (once, and after any change to `fixtures/`)

```
cp fixtures/*.qxf ~/Library/Application\ Support/QLC+/Fixtures/
```

Then **quit and relaunch QLC+** — it builds its definition cache at startup.

Check *Fixture Manager → Add fixture…* lists `CLF → Yara` and
`HQ Power → VDPLPS36B2 LED Pinspot PAR36`. QLC+ 5 skips definitions that fail to parse
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
   brain: 1 scenes from open-claw.qxw
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
