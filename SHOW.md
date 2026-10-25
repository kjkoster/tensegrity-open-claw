# Tensegrity Open Claw — The Show

**What this file is for.** This show: what the audience sees and why each look is the way it
is, and how its rig is set up — the fixtures chosen, how each is configured, and the operation
that comes with them. Anything that would change with a different show belongs here. Where
the code behind it lives, and why, is [`DESIGN.md`](DESIGN.md); the Pi, the cabinet, the
network and the general build-and-deploy workflow, which stay the same from show to show, are
[`README.md`](README.md). What we own, and which owned fixtures stand in for rented ones while
testing, is [`OWNED-DEVICES.md`](OWNED-DEVICES.md).

---

# What the audience sees

## 1. Show times and show states

The rig runs to a fixed list of shows in local time, in one show state at a time, chosen each
frame by the show state machine — the brain's Director: test mode first, then the incident
schedule, then the calendar. The base states are blackout, daylight and dusk; the incidents
are searchlights and ball lighting.

The show state machine never sees the rig state, so it has no e-stop, boot or shutdown cases,
and runs on unchanged underneath a stop.

**Civil dusk, not sunset,** because the light reads only once it is actually dark.

The state is published retained on every change:

| topic | values |
|---|---|
| `brain/state/show` | `blackout`, `daylight`, `dusk`, `searchlights` or `ball_lighting` |
| `brain/state/testmode` | whether test mode is on |

The log has a `show: …` or `show: test mode …` line for each change of the show state.

## 2. The base looks

Outside a show the show state is blackout: black but for the ball's status LED, which keeps
breathing so a ball that hears the rig can be told apart from one that does not.

Inside one, the ball's motor runs for one second every two minutes, the first run as the show
opens. Until civil dusk the show state is daylight, and after it dusk, the ball's power LEDs
pulsing until civil dawn.

The music changes no show state; it reaches the rig through the washes. In the base states
each wash holds its own colour — red, green or blue — with white beside it, and one sparkle
modulation drives all three, so the rig breathes in silence and flashes on the music's onsets
in step rather than three ways at once.

The heads park dark on "Look at heaven" in blackout and daylight. At dusk they turn to "Look at
ball" and light it, each in its leg's colour, against the modulation: at most 35% when the
washes are at their darkest, dipping as they flash, and breathing opposite them in silence. So
when the rig goes dark the ball lights up instead. `DUSK_BALL_LEVEL` in
`brain/src/head_show.rs` sets that 35%.

## 3. The dots dance rather than jump

At dusk each head's dot wanders over the ball on a smooth path of its own, so the three never
move in step. In silence it drifts slowly within half its range; as the music comes in it
speeds up and widens to all of it, and each beat surges it along the path. The range is
`DANCE_RADIUS_DEG` in `brain/src/head_show.rs` (6°, about 25 cm on the ball at 2–2.5 m), and
the dots may now and then slip past the edge. The `DANCE_*` constants beside it set the pace
and the surge.

A head needs a tenth of a second or more to reach a new point, so a dot sent to a new place on
every beat would always land after it; a continuous path the beats push along never shows that
lag. The path is worked out as angles around the ball's centre and turned into pan and tilt for
the sideways mounting: tilt moves the dot one for one, while pan, turning the beam around an
axis the ball sits only a little way off, moves it by the sine of that angle and is scaled up
to match. That angle is read from each head's preset, with the middle of the tilt range taken
as the axis.

## 4. Incidents

After dusk, every 30 to 45 minutes, the rig can switch for two to four minutes to an
**incident** — Searchlights or Ball Lighting — and then return.

| incident | heads | washes | ball |
|---|---|---|---|
| Searchlights | white, from "Look at heaven"; one at a time snaps to a random point between "Sky corner A" and "Sky corner B" | the base show | the base show |
| Ball Lighting | on "Look at ball"; white, then each leg's colour, then back, every ten seconds | the same fade | white LED to blue LED and back |

**Incidents are show states with a schedule.** An incident takes the place of dusk for its few
minutes. Making them show states rather than a modulation inside dusk means every consumer
matches them exhaustively, test mode reaches them by stepping, and `brain/state/show` says
which is running; an incident that runs the base show underneath it (Searchlights' washes and
ball) reads the base state the calendar chose, which the cue carries beside the state.

The schedule runs per dark stretch and is drawn, not stored: every draw is a pure function of
a seed, the stretch's start and the incident's number, so the director recomputes the night
each frame and a restart lands back in it. The draws are uniform rather than taken from the
noise fields, whose values cluster. Times are drawn before the pick, so adding or removing an
incident changes which run but not when. An incident starts only if it ends a margin before
the dark stretch does — 20 minutes before dawn or the show's stop — which covers both with one
rule. The pick is from every incident there is.

**Confirming it landed.** At the start of each dark stretch, and at startup inside one, the
log prints the stretch's whole schedule:

```
show: dark 19:08 → 22:00, 2 incidents
show:   #0 at +37m12s: ball_lighting for 3m05s
show:   #1 at +81m40s: searchlights for 2m31s
```

The offsets count from the start of the dark stretch on the line above: dusk, or the show's
start if it opens after dusk. A restart prints the same schedule again: it is drawn from the
dark stretch and nothing else.

## 5. Only the heads fade

Washes and ball switch looks in one frame at both ends of an incident. The heads light over a
second at its start and glide, lit, to its preset at the slowest smooth rate: a beam on the
move invites curiosity, so the move is part of the look rather than something to hide. Over
the incident's last five seconds their light falls away, and dusk lights them again over a
second as they return to the ball. A snap is left to the head's own motors and stays lit.
Searchlights waits until every head has landed its glide to heaven before its first snap, so
the glide is seen whole.

Ball Lighting's fade is drawn by the brain, per frame, rather than by the ball. At five
seconds a crossfade, the bunching of multicast over WiFi steps it about a percent at a time,
which reads as smooth; a fast waveform would not.

## 6. Test mode is on the door, not on a console

Testing a show state should not wait for dusk, for silence, or for a laptop on the network, so
the face's two buttons can take the show over:

| | |
|---|---|
| hold both buttons for 2 s | test mode on, starting from the current show state; both lamps light |
| left button | the next show state: blackout → daylight → dusk → searchlights → ball lighting → blackout |
| right button | the motor, for as long as it is held |
| hold both buttons for 2 s again | test mode off; the calendar has the show again |

It ignores the calendar, the sun and the incident schedule, so every incident can be tried by
day, and under it an incident's base show is dusk, so a test by day shows the night look. An
incident in test mode runs until you step on, with no fade at the end. A step is counted on
release, which is what tells a lone press from half of a chord. The motor runs only while the
right button is held, never on its beat, so stepping show states cannot start it by surprise,
and the ball's firmware still stops any run at 2 s. The e-stop still wins, as it wins over
everything. The buttons are published as `face/left_button` and `face/right_button`.

Test mode ends itself after fifteen minutes without a press, because a rig left in it would
ignore the calendar unattended.

## 7. The presets

The show is written against four pointing presets, aimed on the rig in QLC+:

| preset | used for |
|---|---|
| "Look at heaven" | where the heads park dark, and where Searchlights starts |
| "Look at ball" | dusk, and Ball Lighting |
| "Sky corner A", "Sky corner B" | the two corners Searchlights snaps between |

---

# Setting up the rig

## 8. Fixtures

**Addresses and modes are deliberately not listed here.** `claw.qxw` is the source of truth
and it changes; read them out of QLC+ and set each fixture to match. What this table records
is *how* you set them on each model, which does not change.

| Device | Interface | Notes |
|---|---|---|
| 3× CLF Yara LED par (the washes) | DMX off the HAT | Address and mode from the front-panel LCD menu. Nothing in software can detect a wrong mode, so check each one against the patch. |
| 3× CLF Poseidon Laser Beam XS moving head (the heads) | DMX off the HAT, 5-pin XLR | Address from the LCD menu (*DMX Address*). There is only one mode, 21 channels, so nothing else to match. See [Front-panel settings on the heads](#9-front-panel-settings-on-the-heads). |
| The balls (ESP32-C3) | WiFi; USB serial/JTAG for flashing | See [The ball](#10-the-ball). |

The washes and heads are rented. For testing, the owned stand-ins in
[`OWNED-DEVICES.md`](OWNED-DEVICES.md) take their place.

`CLF-Yara.qxf` and `CLF-Poseidon-Laser-Beam-XS.qxf` are ours, written from the manuals, and
the QLC+ library has neither — **worth submitting upstream** to `mcallegari/qlcplus` once each
has been confirmed against the hardware. Open each in QLC+'s Fixture Editor and re-save first,
so the file matches what their tooling emits.

## 9. Front-panel settings on the heads

State that lives in each head rather than in this repo, and survives nothing: a head swapped,
reset or borrowed comes back with its own idea of these. Recorded here because a wrong one
does not look like a wrong setting — it looks like broken software.

| Menu | Setting | Why |
|---|---|---|
| *DMX Address → DMX Signal mode* | `Wire` | The rig drives the heads over the cable, not wireless DMX. |
| *Personal → P/T invert → Pan invert* | `OFF` | Axis not reversed. |
| *Personal → P/T invert → Tilt invert* | `OFF` | Axis not reversed. |
| *Manual control → Program → Run on power on* | `NO` | A head powered up before the rig is sending stays still instead of playing its own program. |

Pan and tilt invert may be either way round, but **every head must agree**. A head with one
axis reversed tracks DMX perfectly and lands nowhere near where the others do — which reads
as broken aiming code, not as a menu.

## 10. The ball

An ESP32-C3 that joins the Pi's WiFi AP and listens for the sACN universe. The firmware is
the `ball/` crate: `no_std` Rust on Embassy, built on the Mac for
`riscv32imc-unknown-none-elf` with the stable toolchain. It is not a member of the cargo
workspace the Pi builds; build it from inside its own directory:

```
cd ball && cargo build --release
```

Every board gets the same binary and identifies itself at boot by its WiFi station MAC.
`deploy.sh` builds and flashes it as part of every deploy.

### Provisioning a new board

A board has to be known in two places, under two different MACs:

1. Plug it into the Pi and run `./deploy.sh`. It stops at the flash step with
   `unknown device usb-Espressif_…` — add that name to `DEVICE_MAP` in `remote-deploy.sh`,
   mapped to `ball`, and deploy again.
2. `./attach.sh` and read the log. An unprovisioned board prints its station MAC and the
   `Board` row to paste, then panics and restarts, printing it again every boot. Add the row
   to `BOARDS` in `ball/src/config.rs`, name the fixture it is in `claw.qxw` — `Ball` there is
   `patch::BALL` here — and deploy once more. The address, the universe and the port come with
   the fixture.

### Watching a board

```
./attach.sh            # the only board attached
./attach.sh 0          # by index in `probe-rs list`
./attach.sh AC:A7      # by any unique part of the probe's serial
```

Runs from the Mac and attaches `probe-rs` on the Pi, which is where the boards are plugged in.

### Sleep

Outside shows the brain asks the balls to sleep, on the ball's Sleep channel: from the end of
one show until ten minutes before the next, and never in test mode or while the e-stop is
pressed. A ball sleeps in five-minute stretches of deep sleep, and every wake is a boot.

A boot listens for five seconds once its network is up, following the universe meanwhile and
sending its telemetry. It then sleeps again if the universe asks it to, or if no universe was
heard at all — the rig is off. It also sleeps again if it cannot join the access point within
about a minute, which is what a switched-off Pi looks like. Otherwise it stays awake, and from
then on only the Sleep channel puts it back to sleep. Going to sleep stops the motor first,
then says goodbye on MQTT and leaves the access point cleanly.

On MQTT a sleeping ball looks like this:

| topic | while it sleeps |
|---|---|
| `health/ball` | `disconnected`, then `connected` for a few seconds every five minutes |
| `ball/pack_v` | refreshed on every wake |
| `ball/wakes` | wakes from deep sleep in a row; back to 0 after any other reset |

A `health/ball` that says `gone` rather than `disconnected` is a ball that did not go to
sleep on purpose.
