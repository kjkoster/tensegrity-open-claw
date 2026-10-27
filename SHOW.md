# Tensegrity Open Claw — The Show: Front Yard, Halloween 2026

**What this file is for.** This show: what the audience sees and why each look is the way it
is, and how its rig is set up — the fixtures chosen, how each is configured, and the operation
that comes with them. Anything that would change with a different show belongs here. Where
the code behind it lives, and why, is [`DESIGN.md`](DESIGN.md); the Pi, the cabinet, the
network and the general build-and-deploy workflow, which stay the same from show to show, are
[`README.md`](README.md). What we own is [`OWNED-DEVICES.md`](OWNED-DEVICES.md); this is a
private project, so it is also what the show is built from. The steps to get from the
previous show to this one are [`TODO.md`](TODO.md).

---

# What the audience sees

## 1. The premise

A front yard, a path from the street to the front door, a balcony above it with four moving
heads in a line. The street hears a slow heartbeat. On the path near the house, one head
projects a rose that pulses with it; now and then an owl calls somewhere far off. Someone
who walks up the path quickens the heart. Someone who gets too close to the rose is cut off
in darkness and silence, hears the heads turn above them in the dark, and is caught in white
light and a thunderclap. Then the heads go out, the front door glows amber until they leave,
and the rose and the heartbeat return for the next group.

## 2. Show times and show states

The rig runs to a fixed list of shows in local time (`brain/shows.rs`), in one show state at
a time, chosen each frame by the show state machine — the brain's Director: test mode first,
then the calendar, then, inside a show, presence and the timers. Outside a show, and inside one
until civil dusk, the state is `blackout`: dark and silent. From dusk to the show's end it is
one of the six below. A show is listed from the morning, so the rig is up and reporting all
day and the show proper starts by itself when it is dark enough.

The show state machine never sees the rig state, so it has no e-stop, boot or shutdown cases,
and runs on unchanged underneath a stop.

| state | what the audience sees | leaves when |
|---|---|---|
| `idle` | the rose on the path, pulsing with a heartbeat that quickens as someone comes up the path; owls | someone reaches the trigger line |
| `cut` | nothing: blackout and silence, hard | `CUT_SECS` have passed |
| `buildup` | still nothing, but the heads are heard turning | `BUILDUP_SECS` have passed |
| `lightning` | a flash, before anything is heard | `LIGHTNING_SECS` have passed: two frames |
| `thunderclap` | the heads white, two strobing, two steady; the thunderclap | the recording ends |
| `respite` | the heads dark; the front door amber | the yard is empty |

There is no state for "someone is in the yard": the heart's pace is a function of the
distance, and an empty yard is simply the distance at which it rests. A state boundary there
would need a debounce of its own and would say nothing the distance does not.

`cut` → `buildup` → `lightning` → `thunderclap` → `respite` is one sequence, the scare; it is
several states rather than one because every consumer matches them exhaustively, test mode can
watch each, and `brain/show` says which is running. From `respite` the show returns to `idle`.

The state is published retained on every change:

| topic | values |
|---|---|
| `brain/show` | `blackout`, `idle`, `cut`, `buildup`, `lightning`, `thunderclap` or `respite` |
| `brain/testmode` | whether test mode is on |
| `brain/presence` | the distance the show decided on this frame, in paths, or `tombstone` — see [§6](#6-presence-one-distance) |

The log has a `show: …` line for each change of the show state, with what caused it.

```
show: idle → cut — the furthest at -0.02, past the trigger line
show: cut → buildup
show: buildup → lightning
show: lightning → thunderclap
show: thunderclap → respite — the recording ended
show: respite → idle — the yard has been empty for 3.0 s
```

## 3. Idle: the rose and the heart

One head, `ROSE_HEAD`, projects the rose gobo on the path near the house, on the "Rose"
preset. The other three are dark on the same preset, parked tilted into the balcony — not at
their thunderclap positions, so that all four are heard turning when the time comes, and not
pointing anywhere a stray flash would matter.

The heart is one oscillator the light and the sound both follow, so the rose and the lub-dub
never drift apart. Each beat triggers the heartbeat recording once, and the rose's dimmer
follows **the recording's own loudness**: the build runs an analyser over the heartbeat file
and emits its envelope as one value per DMX frame, so at 40 Hz the rose simply plays that
table from the beat, scaled between `ROSE_FLOOR` and full — the lub and the dub land where
the recording puts them, and a different recording brings its own pulse with it. Nothing
about the recording's shape is written into the show.

**The pace is the distance.** The target rate is `REST_BPM` at `HEART_RANGE` and beyond,
`SCARE_BPM` at the trigger line, linear between; the tombstone — nobody in the yard — is the
one exception, and rests. The actual rate glides toward the target with time constant
`RATE_TAU`: detection jitter must not make the heart stutter, and a heart changes pace
gradually anyway. Someone walking up the path quickens it; someone leaving lets it settle; a
detector that loses someone for a frame moves the target for a frame, which the glide
swallows. The rate is set once per beat, at the beat, so a lub-dub is never cut short by a
change of pace; between beats the show only decides what the next period will be.

The owl calls at random, a gap drawn uniformly between `OWL_MIN_SECS` and `OWL_MAX_SECS`
after the last call, in `idle` only. A scare interrupts one, and the next is drawn afresh
from the return to `idle`.

## 4. The scare

When the distance reaches zero — the furthest person at or past the trigger line — and stays
there for `SCARE_CONFIRM_SECS`, two of the eyeball's frames rather than one bad box, the scare
starts.

**`cut`, `CUT_SECS`.** Every head's dimmer to zero, the door wash off, the
mouth muted, all in one frame. There is no fade anywhere: the point is the cut. The audio
mute is a hard zero in the next buffer the mouth fills, within a few milliseconds of the frame;
how the mouth keeps that latency low is in [`DESIGN.md`](DESIGN.md).

**`buildup`, `BUILDUP_SECS`.** Still dark and silent. The heads move to "Thunderclap", their
gobo wheels to open and their colour wheels to white, at full movement speed. There is no
feedback from a head, so the state is a timer: `BUILDUP_SECS` is measured on the rig, the longest
move from "Rose" to "Thunderclap" plus a margin, and the two presets are aimed so that move is
one a cheap head completes in it. Which is why the parked heads in "Rose" tilt into the
balcony rather than away from it.

**`lightning`, two frames.** Everything at full for `LIGHTNING_SECS`, still silent. The
thunderclap is cued on the frame after
and is heard a buffer later, so the flash leads the crack by about 60 ms, the way lightning
does.

**`thunderclap`, the length of the recording.** The heads light white: `STROBE_HEADS` strobe
at `STROBE_RATE` through their shutter channel, the other two are steady, all at full. The
thunderclap plays from its first sample. The state lasts exactly as long as the recording,
which the show knows from the file at build time, so a different recording changes nothing
else.

The heads do not hold still while they are lit. Each one hops, at full speed, between random
points inside a circle of `THUNDER_RADIUS_DEG` around its own "Thunderclap" pose, picking a
fresh point every `THUNDER_HOP_SECS`, each head on its own draws so the four never move in
step. The preset is the centre and comes from the workspace like any other; the circle's
size and the pace of the hops are the show's choice. The points are drawn uniformly over the
circle, not around its edge, so the beams sweep across the people rather than orbit them, and
a hop is a snap left to the head's own motors, lit — beams lurching in the lightning are the
look, and nothing here is smoothed. A head needs a tenth of a second or more to land a hop,
so `THUNDER_HOP_SECS` stays well above that, or the heads never arrive anywhere and only
shudder.

## 5. Respite: the door

The heads go dark where they stand; the door wash lights the
front door in `DOOR_AMBER`. Both switch in one frame. The state holds while anyone is in the
yard, and ends when the distance has been the tombstone for `LEAVE_SECS`, so a detector that
loses the group for a frame does not reset the show on them.

**The reset.** The door goes dark at once. The heads move dark to "Rose", and after
`BUILDUP_SECS` the rose lights and the heart starts, at rest. A show opens the same way.

## 6. Presence: one distance

The show has one input besides the clock: **the distance from the person who has come
furthest into the yard to the trigger line** — positive on the street side of the line, zero
on it, negative past it — or the **tombstone** when nobody is in the yard. The yard has two
benchmarks and nothing else: the yard entry, where the street ends, and the trigger line, in
front of the rose. **The unit is the path**: the entry is at 1, the trigger line at 0, and
nothing is ever measured in metres. The camera looks at the path from the side, so both
benchmarks are simply two places in its fixed picture, and a distance is where a person's feet
fall between them. Someone beyond the entry is on the street and is not in the yard; someone
between the two is, at their distance; someone past the line is at a negative one.

The tombstone is `+∞`: infinitely far is what an empty yard means, every comparison in the
show then falls the safe way without a special case, and there is no magic number to
remember.

The distance comes from one place: **the eyeball**, inside the brain, watching the camera.
It hands the show the distance of the furthest person it sees between the benchmarks, or the
tombstone, once per camera frame it has processed — a few a second on the Pi, so a value
stands for several of the rig's frames, and the heart's glide is what turns those steps into
a pace. Stale for `EYEBALL_STALE_SECS` is the tombstone too: an eyeball that has lost its
camera idles the show on the rose, which is the safe default, and `eyeball/camera` says why.
Recorded footage played into the eyeball in place of the camera is how the heart is heard
quickening with nobody on the path.

The distance and the face's buttons each reach the show as their own snapshot; what the
eyeball does is in [§14](#14-the-eyeball).

## 7. The lamps

The two green lamps in the door's buttons are patched as the `Face` fixture and follow the
frame like any other, so a takeover and the e-stop reach them with no special case.

| | left lamp | right lamp |
|---|---|---|
| normally | the rose | dark |
| test mode | the rose | lit |

The left lamp is the rose's level, whether or not the heads are driven: the heartbeat's pulse
in `idle`, the flash and the thunderclap's envelope in the scare, dark otherwise. So the show
can be watched against the sound from the door. The right lamp lit means test mode, and so
that the rig is not on the calendar; outside test mode the buttons do nothing but enter it.

## 8. Test mode is on the door, not on a console

Testing a show state should not wait for the show's hours or for a visitor, so the face's two
buttons can take the show over:

| | |
|---|---|
| hold both buttons for 2 s | test mode on, in the current show state, or `idle` from `blackout`; the right lamp lights |
| right button, on press | the scare, from wherever the show is: `cut` → `buildup` → `lightning` → `thunderclap` → `respite` |
| left button, on press | presence, in turn: nobody, someone halfway up the path (0.5), someone just short of the trigger line (0.1) |
| hold both buttons for 2 s again | test mode off; the calendar has the show again, and the yard is empty |

The right button fires the scare the moment it goes down, whatever state the show is in —
mid-scare included, which restarts it from `cut`. The left button stands in for the eyeball,
which is not built yet: outside test mode the yard is empty. Its nearest step is short of the
line, so it quickens the heart to nearly the scare rate and never scares by itself. Nothing
else changes in test mode: the states end the way they always do, `respite` on an empty yard,
`idle` on the trigger line, and the heart follows presence. The e-stop still wins, as it wins
over everything.

One consequence of acting on press: a chord counts a press of the button it was begun with.
Begin it with the left button, which only steps presence, or accept one last scare on the way
out.

Test mode ends itself after fifteen minutes without a chord, because a rig left in it would
ignore the calendar unattended.

## 9. The presets

The show is written against two pointing presets, aimed on the rig in QLC+:

| preset | used for |
|---|---|
| "Rose" | `ROSE_HEAD` on the path near the house; the other three dark, tilted into the balcony |
| "Thunderclap" | the centre of each head's circle in the thunderclap: all four at the people on the path, at the trigger line, beam at head height |

Each holds Pan, Pan Fine, Tilt and Tilt Fine for every head and nothing else, as the build
requires. The rose head's gobo and colour are not in the preset: the show asks the fixture
definition for the band of the rose gobo and of white, by name, and sets them itself.

## 10. The numbers

Everything above that is a choice lives in `brain/src/show.rs`, with the reason beside each;
the sound table is `brain/sounds.rs`. The starting values:

| constant | value | |
|---|---|---|
| `REST_BPM`, `SCARE_BPM` | 55, 130 | resting heart; the heart at the trigger line |
| `HEART_RANGE` | 1.0 | the distance, in paths, over which the heart goes from rest to the scare rate: the whole walk from the entry; less to leave the first steps at rest |
| `RATE_TAU` | 1.5 s | how fast the heart changes pace: the time constant of the glide, so the rate closes 63 % of the gap to its target in 1.5 s and 95 % in 4.5 s |
| `ROSE_FLOOR` | 25 % | the rose at the quietest point of the envelope; full at its loudest |
| `OWL_MIN_SECS`, `OWL_MAX_SECS` | 45 s, 120 s | the gap between owls |
| `SCARE_CONFIRM_SECS`, `LEAVE_SECS` | 0.2 s, 3 s | the two debounces on presence: into the scare, out of respite |
| `CUT_SECS`, `BUILDUP_SECS` | 1 s, 1 s | the two dark seconds; `BUILDUP_SECS` is measured, see [§16](#16-commissioning) |
| `LIGHTNING_SECS` | 50 ms | the flash before the thunderclap: two frames |
| `STROBE_HEADS`, `STROBE_RATE` | heads 2 and 3, fast | which heads strobe in the thunderclap |
| `THUNDER_RADIUS_DEG`, `THUNDER_HOP_SECS` | 15°, 0.4 s | the circle each head hops around in the thunderclap, and how often it hops |
| `DOOR_AMBER` | (255, 110, 0) | the door, as R, G, B on the wash |
| `EYEBALL_STALE_SECS` | 2 s | after which the eyeball's silence is the tombstone; with `SCARE_CONFIRM_SECS`, set from the frame rate measured on the Pi |

The sound table names each recording and its gain; everything else about a recording the
build reads from the file. The gains are set by ear at the bench, and are 0 dB, the loudest a
recording plays, until then.

| sound | file in `brain/sounds/` | gain | |
|---|---|---|---|
| heartbeat | `heartbeat.m4a` | 0 dB | one lub-dub; must be shorter than the fastest beat, which the build checks; its envelope is the rose's pulse |
| owl | `owl.mp3` | 0 dB | far off |
| thunderclap | `thunderclap.m4a` | 0 dB | its length is the length of the state |

---

# Setting up the rig

## 11. Fixtures

**Addresses and modes are deliberately not listed here.** `claw.qxw` is the source of truth
and it changes; read them out of QLC+ and set each fixture to match. What this table records
is *how* you set them on each model, which does not change.

| Device | Interface | Notes |
|---|---|---|
| 4× U\`King ZQ02015 moving head (the heads) | DMX off the HAT, 3-pin | Named `Head 1` to `Head 4`, left to right as seen from the street. Address (`Addr`) and mode (`CHnd`) from the display menu; `12CH`, the mode that carries Pan Fine and Tilt Fine, which the presets require. See [§12](#12-front-panel-settings-on-the-heads). |
| 1× HQ Power VDPLPS36B2 LED par (the door wash) | DMX off the HAT, 3-pin | Named `Wash door`. HQ Power sells it as a pinspot; it is a wash. Mode and address are DIP switches: set the binary pattern from the datasheet. 3-channel RGB mode — the 5-channel mode adds a strobe and macros the show does not use and the e-stop would have to mask. |
| The face | GPIO | Unchanged in the cabinet. |
| The mouth (the Alesis io\|2 and two Monkey Banana powered speakers) | USB on the hub; TRS to the speakers, left and right | See [§13](#13-sound). |
| The eyeball's camera (the Axis M3105-LVE) | Ethernet, through its PoE injector, into the Pi's `eth0` | See [§14](#14-the-eyeball). |

Not in this show: the ball, the laser, and anything rented. The deploy builds no wireless
firmware when the workspace patches no wireless fixture.

Both definitions are in `fixtures/`. `HQ-Power-VDPLPS36B2-LED-Pinspot-PAR36.qxf` is ours,
from the datasheet. `UKing-ZQ02015.qxf` came from QLC+'s library and is partly measured: the
strobe channel's open band, the motor speed's polarity, the no-function bands of
`Automatic Mode` and `Reset`, and colour bands that match the manual. Still to measure: a band
per gobo on the one undivided "Fixed Gobo" band, the rose among them; the strobe band; the
`Light Strips` channel's off band; and the real travel — the definition says 540° pan and
180° tilt, and the manual gives no figures. The show asks the definition for all of these; a
guess in the file is a guess on the night.

The heads have LED strips on their bodies, on the `Light Strips` channel. The show keeps them
off.

## 12. Front-panel settings on the heads

State that lives in each head and survives nothing: a head swapped or reset comes back with
its own idea of these. Recorded because a wrong one looks like broken software.

| Setting | Value | Why |
|---|---|---|
| DMX address, DMX mode | from `claw.qxw` | Nothing in software can detect a wrong mode; check each against the patch. |
| `bLnd` | `bLAc` | Signal loss blacks a head out instead of starting its own program. The e-stop and a cable fault then look alike. |
| `rPAN`, `rTiL` | `no` on every head | A head with one axis reversed tracks DMX perfectly and lands nowhere near the others, which reads as broken aiming code. |
| `Soud` | `oFF` | A head powered up before the rig sends stays still. |

## 13. Sound

The brain plays through the Alesis io|2 — the same interface the ear captured through — into
two Monkey Banana powered speakers, in stereo. Nothing captures any more.

- **Wiring.** The io|2's left and right main outs (TRS), one to each speaker. Turn its
  direct-monitor mix knob fully to the computer side, so the unused mic inputs cannot bleed
  into the speakers, and set the output level with the speakers' own volume knobs; the brain
  plays each recording at its gain in the sound table, and has no master gain of its own.
- **Placement.** The heartbeat is for the street and the thunderclap for the person on the
  path, from the same speakers: low, by the path, under the balcony, out of sight. Set the
  level by the thunderclap — it is the loud one — and let the heartbeat sit where the gains
  put it.
- **Recordings** are in `brain/sounds/`, as they were found — AAC in `.m4a`, MP3 — and are
  **compiled into the brain**: decoded, resampled to 48 kHz stereo 16-bit, trimmed of what is
  inaudible at either end, brought to full scale, and stored with their envelope, one byte per
  DMX frame. Nothing is read from disk at startup or during a show, there is no decoder in
  the running program, and the binary on the Pi is the whole show. A file the build cannot
  read is a build error naming it; the show adds that the heartbeat is shorter than the
  fastest beat.
- **On MQTT**, through the brain's client — the mouth is a thread in the brain, so it lives
  and dies with it and `health/brain` covers it:

  | topic | |
  |---|---|
  | `mouth/state` | `playing`, `muted` by the show, or `down` when the interface failed |
  | `mouth/error` | why the interface is down, present only while it is |
  | `mouth/xruns` | buffer underruns since the brain started: the show is stuttering, make the buffer bigger or the Pi quieter |
  | `mouth/usb/vid_pid`, `mouth/usb/bus`, `mouth/usb/address`, `mouth/usb/speed` | the interface as the kernel sees it: identity, where on the bus, and `full`/`high` — a full-speed enumeration on a high-speed device is a bad cable or a bad hub port |
  | `mouth/usb/sysfs` | its `/sys/bus/usb/devices/…` path, so `lsusb -t` and the journal can be read against it |
  | `mouth/usb/serial`, `mouth/usb/product` | from `/sys`, blank where the device has none |
  | `mouth/alsa/card`, `mouth/alsa/rate_hz`, `mouth/alsa/period`, `mouth/alsa/buffer` | what the stream was actually opened at — the rate and sizes asked for are not always the ones granted |

  A missing or dead interface costs the sound and never a frame: the show runs on silent, and
  `mouth/state` says so. Re-enumeration shows as the `usb/` topics changing under a running
  brain, which is the signature of a flaky hub port rather than a software fault.

The e-stop silences the mouth along with the frame. A console takeover does not: sound is not
part of the universe, so the show's sound runs on underneath a takeover.

## 14. The eyeball

The Axis M3105-LVE watches the yard; the eyeball, inside the brain on the Pi, finds people in
the picture, places their feet on the path, and hands the show how far the furthest of them
is from the trigger line. Its design is [`DESIGN.md`](DESIGN.md) §3.6.

**Network.** The camera's PoE injector plugs straight into the Pi's `eth0`, `10.0.1.0/24`:
the Pi at `10.0.1.1`, the camera at `10.0.1.90`. The Ethernet hub goes between them only when
a laptop needs the wire too.

**Camera.** Mounted so the path runs across the picture, square on: the yard entry at the
right edge, the trigger line towards the left, the whole walk between them, and the pavement
out of the picture altogether — the marks only bound the picture left to right, so anyone the
camera can see walking past behind the entry would count as in the yard. Framing is what
keeps them out. It runs under its
own infrared illuminator with the IR-cut filter off, which the eyeball sets over VAPIX on every
start, as the old one did and for the same reasons: infrared beats visible light by a wide
margin at night, the rose and the amber door barely register in it, and an exposure capped at
10 ms keeps a walking leg a shape rather than a streak. The thunderclap saturates the picture
for a few seconds; nothing is detected during it and nothing needs to be. The camera's
password is the eyeball's configuration and nowhere else.

**The two benchmarks.** There is no calibration, only aiming. The two benchmarks are two
marks fixed in the camera's picture — the trigger column towards the left, the entry column
towards the right, at `TRIGGER_COLUMN` and `ENTRY_COLUMN` in `eyeball/src/config.rs` as
fractions of the frame width — and the camera is aimed so the yard lands on them: open the
annotated feed, which draws both marks, and move the camera until the trigger mark sits on
the rose and the entry mark sits on the path's entry. Panning slides the picture along the
path; how far back the camera is mounted sets how much path lies between the marks, so the
mount's distance from the path is the other thing to move. The marks themselves are the last
resort, for a mount that cannot move. A person is the trigger-side bottom corner of their
box — the leading foot, not the middle of the box, because the detector sometimes draws one
box over a bunch of children, and the one in front is the one who matters. Their distance is
where that point falls between the two marks, 1 at the entry and 0 at the line.
Left of the trigger mark is negative; right of the entry mark is the street, and does not
count. The furthest person is the smallest distance; nobody between the marks is the
tombstone. Walk the path and watch the feed's number fall from 1 to 0. The number is linear
in the picture, so it is only as linear in the yard as the camera is square to the path; if
halfway along reads well off 0.5, the camera is angled — turn it rather than add a mark;
nothing in the show needs it exact.

**The annotated feed.** `http://10.8.0.3:8080/`, from the Mac over WireGuard, is the
camera's picture with the eyeball's reading drawn on it: every box, each box's leading foot,
the two benchmark columns, the distance it is handing the show, and the frame rate it
is managing. It is how the camera is aimed and the benchmarks are set, and on the night it is
how the operator sees what the rig sees; it is the operator's view, not part of what the yard
sees. Over 4G it is paid for out of the data bundle for as long as it is open.

It costs detector time: drawing on a frame is nothing, but compressing the result to JPEG
again is not, on a Pi. So the feed is encoded on its own thread from the latest annotated
frame, and **only while a browser is connected** — with nobody watching it costs nothing at
all, and with someone watching `eyeball/fps` says what it costs.

**What it hands the show**, once per camera frame it has processed and never otherwise: the
distance from the furthest person to the trigger line, in paths — 1 at the entry, 0 at the
line — or `+∞`, the tombstone, with the time it was seen. The show holds the last.

**The detector.** YOLOX-Nano, Megvii's own ONNX export, reading only *person*. At this
picture — people three to six metres off, boxes over a hundred pixels tall — the accuracy
between small detectors is noise next to their run time, and Nano is the one that fits on the
Pi. It counts a box from a confidence of 0.3 and merges overlapping boxes only above an
overlap of 0.6, both looser than usual: a phantom can only land between the marks and the
show debounces it, while a missed child costs the scare, and children seen side-on overlap.

**How often that is.** The detector sets the pace, not the camera, and on the Pi it shares
four cores with the DMX loop: the model runs at normal priority on at most three of them,
below the brain's real-time threads, so a slow model costs frames of the eyeball's and never
a frame of the rig's. Nano is expected at 150–300 ms a frame on the Pi 3B+ and 60–120 ms on a
Pi 4, measured before anything else is built; about five a second is what the show wants, and
two frames are what `SCARE_CONFIRM_SECS` is meant to cover. Below that, the Pi 4 goes in. The camera is asked for 640×480 MJPEG at a rate a little above it — a stream the
detector cannot keep up with would queue frames and the distance would lag reality by the
queue, so the eyeball always takes the newest frame and drops what has piled up behind it,
and publishes `eyeball/fps` so the queue shows up as a rate below the camera's rather than as
a show that reacts late.

**On MQTT**, through the brain's connection: `eyeball/distance` (what it saw, each frame, in
paths, or `tombstone` — beside `brain/presence`, which is what the show made of it),
`eyeball/fps` (frames processed per second), `eyeball/people` (boxes between the benchmarks,
telemetry only — the show never sees a count), `eyeball/camera` (`ok`, or why not).

## 15. Show times

`brain/shows.rs` lists the shows: test days as they come, and Halloween.

```
CITY = "Delft"
("2026-10-27 09:00", "2026-10-27 22:00")      // test day
("2026-10-31 09:00", "2026-10-31 22:00")      // Halloween
```

Each runs from the morning so the rig is up and reporting all day; it is `blackout` by
daylight, and the show proper begins at civil dusk, which the build works out for Delft —
around 17:50 on the 31st, in CET since the clocks changed on October 25th. The startup log
lists each show with its dark stretch and says which clock it believes, as
[`README.md`](README.md) describes.

## 16. Commissioning

In this order, because each step needs the one before it.

1. **Addresses and modes** on every fixture, read from QLC+. The startup log's slot spans are
   what the brain believes; check them.
2. **The e-stop**, before anything is aimed at anyone: press it with the rig running and read
   `rig: e-stop pressed` and the override count.
3. **Aim "Rose"**, then **"Thunderclap"**, in the Scene Editor as [`README.md`](README.md)
   describes, on the rig after dark. Save, deploy, read the presets back from the log.
4. **Measure `BUILDUP_SECS`**: take the rig over in QLC+, put every head on "Rose", switch to
   "Thunderclap" and time the slowest to settle. Set `BUILDUP_SECS` to that plus a margin. If it is
   much more than a second, aim the parked heads closer to their thunderclap positions rather
   than stretching the darkness.
5. **Levels**: the thunderclap first, then listen to the heartbeat from the street.
6. **The show by button**: test mode on, fire the scare with the right button, from `idle`,
   from `respite`, and once mid-scare. Nothing here needs the eyeball.
7. **The camera**: with the annotated feed open, aim it square at the path until the trigger
   mark sits on the rose and the entry mark on the path's entry; walk the path and watch the
   feed, `eyeball/distance` and `brain/presence`, and listen to the heart quicken. Then the
   show by walking it.
