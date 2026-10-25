# Tensegrity Open Claw — Building the Halloween show

**What this file is for.** The steps from the previous show to the one in
[`SHOW.md`](SHOW.md), in the order they depend on each other. Each step says what changes
and what proves it landed. The design decisions behind them are in the first section, so the
ones to overturn are overturned before the work starts rather than after.

The steps are ordered by risk, biggest first, so the thing most likely to sink the show gets
the first days: the eyeball — a new crate, a model to pick and to fit on the Pi beside the
DMX loop — is step 1. Where a step builds on a later one, that is said at its head. The scare
can be rehearsed without the eyeball at any point, from the cabinet door in test mode; the
heart's pace needs the eyeball, or the reference footage replayed into it. Halloween is
Saturday the 31st, six days from the date of this file.

---

## 0. Decisions taken here

Each is a choice, written down so it can be changed on purpose.

| decision | taken | alternative not taken |
|---|---|---|
| How inputs reach the show | Each sensor hands the show its own snapshot over its own latest-value seam, the way the face already does: the face's `FaceState`, and the eyeball's distance with the time it was seen. The show closure holds both readers. The lamps stay driven from the outgoing frame. A sensor on another machine later would arrive through an OSC receiver publishing on the same kind of seam, and nothing after it would change. | One door, `inputs::inject`, with an `Input` enum over every kind: `cortex` would have to learn the face's shape, which lives in the brain, and the frame loop would unpack the enum back into the snapshots it started as. |
| What a silent face means | Nothing: there is no face-staleness rule. The face is a task on the frame loop's own executor, so it cannot stop while frames go on going out; a panic in either ends the process. The e-stop is already read in `face::claim`, before the first frame. | A `FACE_STALE_MS` e-stop: it guards against a failure that cannot happen. |
| Where the eyeball runs | Inside the brain, on the Pi: its own crate, `eyeball/`, a workspace member the brain depends on, running as threads beside the frame loop. One machine, no network hop between the camera's reading and the show, and nothing to deploy but the brain. | A second machine, `eyeson`, sending OSC: a machine to set up, power, reach and deploy to, for headroom the Pi may turn out to have. A process of its own on the Pi: a socket between two processes that would otherwise share a seam. |
| How the eyeball shares the Pi | Its threads drop out of the brain's real-time scheduling to `SCHED_OTHER` before they start the model, so the threads ONNX Runtime spawns inherit that too, and the model gets at most three cores. A detector that eats every cycle then slows itself, never a DMX frame, Stem or `sshd`. | Inheriting `brain.service`'s `SCHED_FIFO`: a CPU-bound thread at the frame loop's own priority, starving everything below it up to the kernel's real-time throttle. |
| Where the model lives | Compiled into the brain with `include_bytes!`, as the recordings are, and loaded from memory. The binary on the Pi is the whole show. | A file beside the binary: one more thing to ship, and to be missing. |
| What presence is | One float: the furthest person's distance to the trigger line in paths — 1 at the yard entry, 0 at the line — or the tombstone. Two benchmarks define the yard; no polygon, no count, no metres. | A count and a normalised progress: more than the show reads. Metres: a tape measure and a number to keep right, for a show that only ever compares distances to each other. |
| The tombstone | `+∞`. An empty yard is infinitely far; every comparison then falls the safe way, and there is no magic number. | `NaN`: compares false both ways, which is a footgun; a sentinel like `-1`: collides with a real negative distance past the line. |
| Where the mouth runs | Its own crate, `mouth/`, running as a thread in the brain like the ear was, fed by a latest-value seam from the show. The cut and the beat are frame-synchronous; a separate process would add a hop between them. `cortex` holds only what every rig shares. | Its own process (`DESIGN.md` §1.6): a dead DAC costs the sound either way, and the thread reports it. |
| How the mouth hears a trigger | A counter per sound in the cue, and mute as a level. The mouth reads the latest cue every period, four or five times a frame, and only a counter that has moved reads once — the face's presses work the same way. | A list of triggers per frame: replayed on every period the frame is still the latest. |
| How a recording becomes samples | The build writes each WAV's raw PCM to `OUT_DIR`; the generated source wraps `include_bytes!` of it in a 2-byte-aligned static and views it as `&'static [i16]` with `slice::from_raw_parts`, in a `const`. The Pi and WAV are both little-endian. | A `const fn` turning the bytes into an `[i16; N]`: hundreds of thousands of compile-time iterations for a thunderclap, which runs into rustc's `long_running_const_eval` lint. A literal array: megabytes of generated source. |
| Telemetry clients inside the brain | One connection, the brain's. The face, the mouth and the eyeball publish their subtrees, `face/…`, `mouth/…` and `eyeball/…`, through it, and have no `health/` topic of their own. A client and a will are for a process that can die on its own: the brain, the ball, Stem. | A client each, as the face and the ear had: none can die without the brain, so their wills only repeat `health/brain`, at the cost of a connection, a thread and a farewell apiece. |
| `rig-mqtt` is renamed `stem` | The crate is the Rust programs' view of Stem, the process they report to, so it is named for it and moves into `stem/` beside the daemon, the broker configuration and `rig_mqtt.py`, its Python half. `cortex::telemetry`'s client moves into it, behind a `std` feature, so Stem's Rust side is one crate. | A directory of its own: Stem split across two places. |
| Which heads strobe | Heads 2 and 3, the inner pair; 1 and 4 steady. | Any pair. A constant. |
| Wash mode | 3-channel RGB. | 5-channel: strobe and macros the show does not use and the e-stop must mask. |
| Where the camera connects | Its PoE injector's data port straight into the Pi's `eth0`, the camera re-addressed to `10.0.1.90`. The Ethernet hub goes in only when a laptop needs the wire as well. | The camera on the Pi's WiFi: every video frame over 2.4 GHz. |
| The eyeball's model | A person detector, not a pose model — boxes are enough and feet are the bottom of a box. Start with YOLOX-Nano (Apache-2.0, ONNX published), through ONNX Runtime (`ort`); measure the frame rate on the Pi at 320 and 416 px before settling. | YOLOv8/11-nano: faster to get going, AGPL. |
| How the eyeball reaches the picture | MJPEG over HTTP (`/axis-cgi/mjpg/video.cgi`), decoded with `zune-jpeg`: no inter-frame state, no jitter buffer, no ffmpeg to link. | RTSP/H.264: a video decoder in the brain's build. |
| Who debounces presence | The show. The eyeball reports the distance it saw in each frame; hysteresis is the show's policy. | The eyeball tracking people: more code where it is hardest to test. |
| The annotated feed | Wanted, as the operator's view of what the rig sees. Encoded on its own thread and only while a browser is connected, so an idle feed costs the detector nothing. Bound to the WireGuard address, `10.8.0.3:8080`, like the sACN receiver, so the Mac opens it directly. | Always-on encoding: a JPEG encode a frame on the Pi whether or not anyone is looking. The wildcard: the 4G uplink's public address would show the yard, and the children in it, to the internet. |
| How the eyeball measures | Two marks fixed in the camera's own picture, and the camera aimed so the rose and the path's entry land on them, watching the annotated feed; feet are the bottom of a box, distance is linear between the marks. No calibration, only aiming. Needs the camera square to the path, which the feed shows at a glance. | A ground-plane homography from four marks: right at any angle, four points to measure and a matrix to get wrong. |
| The ball | Stays in the repo as a component; not patched in this show. The deploy skips wireless firmware when the workspace patches none. | Deleting it: the next Tensegrity project wants it back. |
| The ear | Deleted, as asked. Its last version is on the `2026-10-23-ADE-Amsterdam` tag, which already marks the previous show; no new tags. | `last-version-with-claw-show` and `last-version-with-ear`: two more names for the commit that tag names. |
| Fixture names | By position, `Head 1`–`Head 4` left to right from the street, `Wash door`. Roles (which head carries the rose) are the show's choice and change; positions do not. | By role. |
| Dark, on a head | Dimmer zero, with the strobe channel in its open band. The head's definition has no closed band, and the manual gives the strobe channel as a bare 0–255. | Shutter closed: there is nothing to close it to unless measuring finds a band. |
| The heads' light strips | The `Light Strips` channel joins the Effect group in the definition, with its `Off` band marked `NoFunction`. The show parks it, and the e-stop's mask sends it to `Off` with no new rule. | Leaving it in the Colour group: the e-stop holds Colour where it was, so strips a console lit would stay lit under the stop. |

### Open

Each needs a choice before the step that uses it.

| question | why it matters | recommendation |
|---|---|---|
| The strobe's rate | `STROBE_RATE` is "fast", which is no value, and the strobe band is unmeasured. A fast strobe aimed at children's faces sits in the 3–30 flashes a second that photosensitive-epilepsy guidance warns against. | `STROBE_RATE` in flashes a second, at most 3, read off the measured band; or no strobe, and let the hops carry the lightning. |
| `bLnd` on the heads | `SHOW.md` §12 says `bLAc`; `README.md`'s e-stop section says every fixture holds its last values. The manual gives `bLAc` as "return to zero", which may send the head home, and `HoLd` keeps it lit wherever it was — mid-thunderclap included, for the five seconds a brain restart takes. | Find out on one head what `bLAc` does. If it darkens and stays put, use it and make the README's rule "dark and still"; if it homes, decide between a homing head and a lit one. |
| Is there a rose gobo | The gobo channel is one undivided "Fixed Gobo" band, 8–63, and the manual shows no gobo images. The idle look rests on one of them being a rose. | Step through the band on a head before anything is built around it. |
| What arms the scare | The scare fires when the distance is at or past zero. Someone coming back from the door, or out of the house, appears past the line from the tombstone and is scared on the way out. | Fire only on a crossing: the distance at or below zero after a positive value. Appearing past the line from the tombstone does not count. |
| Is the door in the picture | `respite` holds while anyone is seen. If the camera does not see the doorstep, a group collecting sweets is the tombstone and the rose returns while they stand there. | Frame the door in, or make `LEAVE_SECS` long enough to cover a doorstep visit. |
| A second group during `respite` | Respite ends only on an empty yard, so a group arriving behind the first is not scared. | Accept it: a scare re-armed under people still in the yard would hit them a second time. |
| The heartbeat's length | It must be shorter than one beat at `SCARE_BPM`: 0.46 s at 130. The recording in `audio/` is 0.76 s. | Let beats overlap: the mixer mixes voices anyway, and the rose takes the loudest of the envelopes playing. The check becomes "shorter than a beat at rest". |
| The rose's colour | Step 7's table says "colour of your choosing". | A `ROSE_COLOUR` constant beside the others, read as a band name off the colour wheel. |
| Test mode from `blackout` | Turned on outside show hours, test mode stays dark until the right button is pressed, so the rose cannot be looked at without a scare first. | Test mode turned on in `blackout` enters `idle`. |
| The eyeball's threads | Your rule is Embassy on std too. Nothing in the eyeball runs on a timer: capture is paced by the camera, detection by itself, the preview by its client. | OS threads with latest seams between them, like the brain's other blocking producers, and nothing on the frame loop's executor. |
| Whether the Pi keeps up | A Pi 3B+ with the DMX loop on it may manage two or three frames a second where a laptop managed ten. The debounces and the staleness window are written for ten: `SCARE_CONFIRM_SECS` is "two frames", `EYEBALL_STALE_SECS` "twenty". A spare Pi 4 runs the same software unchanged. | Measure first (step 1). Then set the two constants from the measured rate, and swap in the Pi 4 if the 3B+ cannot hold about five frames a second. |
| ONNX Runtime inside the brain | It is C++. A crash in it ends the brain, which systemd restarts in seconds of signal loss. `tract-onnx` is pure Rust, cannot crash that way, and needs no download, at some cost in speed. | `ort` while it holds the frame rate; `tract` if the speed allows it, or if `ort` has no build for the Pi's target. |

---

## 1. The eyeball, in the brain

A new crate, `eyeball/`, a workspace member the brain depends on, built on the Pi with the
rest of the brain and running as threads inside it. Nothing from the old code is reused but
the camera's VAPIX settings; `DESIGN.md` §3.6 describes it.

- [ ] **First, what the Pi manages.** On `claw-pi`: `rustc -vV` (the host triplet — `ort`
      ships prebuilt ONNX Runtime for `aarch64-unknown-linux-gnu` and not for 32-bit ARM),
      `cat /proc/device-tree/model` and `free -m`. Then a one-file crate there that loads
      YOLOX-Nano and times a frame at 320 and 416 px with three threads, with `brain`
      running. `tract-onnx` gets the same test. That settles the model, the runtime and the
      open question on whether the Pi keeps up, before anything else is built.
- [ ] **Core pinning**: sit down with which thread runs where — the frame loop, the
      detector and ONNX Runtime's pool, the mouth, Stem (pinned to cores 0–2 today) — and
      decide what pins where, from the code (`sched_setaffinity` per thread) or the units.
      Then pull every configuration file that touches it and lives only on the Pi —
      `cmdline.txt` if a core is isolated, `/etc/systemd/system.conf` if defaults change —
      into `etc/` and git, installed by the deploy like `config-claw.txt`.
- [ ] **`rig-mqtt` becomes `stem`, with the std client**: the crate moves into `stem/`
      (`Cargo.toml`, `src/`) and is renamed, in the workspace members, the brain, `cortex`
      and `ball/`, and in `deploy.sh`'s list; `cortex::telemetry`'s client moves into it
      behind a `std` feature, with the broker's address a parameter, and the brain follows.
- [ ] **The camera once**: if the `root` password really is lost, factory-reset it (control
      button held while powering up) and set one; re-address it to `10.0.1.90`; put the
      password in the eyeball's configuration only.
- [ ] **Reference footage, the poor man's test set**: with the camera on its mount, after
      dark and under its own infrared, record the MJPEG stream (`curl` on
      `/axis-cgi/mjpg/video.cgi` into a file) of yourself on the path — walking up to the
      line and stopping, walking up and on to the door, coming back from the door, standing
      at the door, walking past on the pavement, and a few people at once if you can get
      them. `capture.rs` takes a file in place of the camera, so the model, the confidence
      and the two marks are judged against the same walks, and again after every change; and
      a recording played into the brain is how the heart is heard quickening without anyone
      on the path.
- [ ] `eyeball/src/capture.rs`: the MJPEG stream over HTTP (`ureq`), split on the boundary,
      decoded with `zune-jpeg`; reconnect with backoff. The reader keeps only the newest
      frame and drops what the detector has not got to, so a slow model lowers the rate
      rather than adding lag. On every start, set the camera over VAPIX: 640×480 at
      `CAMERA_FPS` (10), IR-cut filter off, exposure ≤ 10 ms, and the image rotation for the
      new mount; read it back and publish any disagreement. The parameter names, the
      sequence and the read-back are in the old eyeball on the `last-version-with-mage` tag
      (`eyeball/eyeball.py`), in Python; port them, they are the part that took finding out.
- [ ] `eyeball/src/detect.rs`: the chosen runtime and model, the model compiled in with
      `include_bytes!` and loaded from memory; letterbox, run, non-maximum suppression, keep
      class *person* above `CONFIDENCE`. The thread drops to `SCHED_OTHER` before it builds
      the session, so the runtime's pool inherits it, and the pool is at most three threads.
- [ ] `eyeball/src/yard.rs`: feet = bottom-centre of each box; distance = where that x falls
      between `TRIGGER_COLUMN` (0) and `ENTRY_COLUMN` (1), both fractions of the frame width,
      negative left of the trigger column; boxes right of the entry column are the street and
      are dropped; the furthest person is the smallest distance; no box between the columns is
      `+∞`.
- [ ] The seam: once per processed frame, the distance and the time it was seen go out on
      the eyeball's own latest-value seam, and never otherwise; the brain's show closure
      holds the reader beside the face's, and a quiet eyeball goes stale. It is the seam an
      OSC receiver would publish on, should a sensor ever move to another machine.
- [ ] `eyeball/src/preview.rs`: the annotated MJPEG feed (`tiny_http`), bound to
      `10.8.0.3:8080`, the WireGuard address, retrying the bind with backoff like the sACN
      receiver: boxes, ground points, the two marks labelled `rose` and `entry`, the
      distance, the frame rate. Drawn into a copy of the latest frame on its own thread,
      encoded with `jpeg-encoder` (`zune-jpeg` only decodes), and only while at least one
      client is connected — no viewer, no encode.
- [ ] MQTT through the brain's connection: `eyeball/distance` (what it saw, each frame, in
      paths rounded to a hundredth, or `tombstone`), `eyeball/fps`, `eyeball/people`
      (telemetry only), `eyeball/camera`.
- [ ] `eyeball/src/config.rs`: camera host, user, password, or a footage file in its place;
      `TRIGGER_COLUMN` and `ENTRY_COLUMN` as fractions of the frame width; `CONFIDENCE`; the
      feed's address.

**Landed when** the preview shows you on the path with the distance falling,
`eyeball/distance` and `brain/presence` follow you, the journal's frame timing does not
move while the detector runs, and the show scares you.

## 2. Fixture definitions

Both are already in `fixtures/`. `HQ-Power-VDPLPS36B2-LED-Pinspot-PAR36.qxf` is ours, both
modes from the datasheet, and needs nothing. `UKing-ZQ02015.qxf` came from QLC+'s library and
has been partly measured: the strobe channel's open band at 0, the motor speed's fast-to-slow
polarity, and `NoFunction` bands on `Automatic Mode` and `Reset` are in, and its colour bands
match the manual. The band constants the
show asks for are generated already, on the model's struct as `<CHANNEL>_<BAND>`; a label
used twice within the model is left unnamed, so every name below must be unique.

- [ ] In `UKing-ZQ02015.qxf`, measured on a head with Simple Desk, not taken from the manual:
  - the gobo channel: split "Fixed Gobo" (8–63) into a band per gobo, the rose labelled
    `Rose`, and relabel 0–7 `Open`;
  - the strobe channel: the strobe band, labelled `Strobe`, with its preset saying which end
    is fast; a closed band only if the head has one;
  - `Light Strips`: into the Effect group, its 0–4 band `Off` with preset `NoFunction`;
  - the real pan and tilt travel: the definition says 540° and 180°, from the library,
    unmeasured.
- [ ] Install both into QLC+ and relaunch it; both must appear in *Add fixture…*.
      Absence is the error signal: QLC+ 5 skips a definition that fails to parse silently.

## 3. Clear the old show

The previous show is the `2026-10-23-ADE-Amsterdam` tag, on the last commit that touched
code; nothing needs tagging.

- [ ] Delete the ear: `cortex/src/ear/` whole, and `cortex/src/sparkle.rs`, which only maps
      its features; the Ear, Mapping and Sparkle sections of `cortex/src/config.rs`; its
      thread, its client and its seam in `cortex::run`. `Rig::show` loses its
      `&AudioFeatures`. The ALSA and `rusb` dependencies leave `cortex` for the mouth's crate,
      and the capture thread's open, reopen and USB probe go with them as its starting point
      (step 6). `cortex/src/envelope.rs` has no user now and stays: it is the heart's glide.
- [ ] Delete the old show in `brain/src/`: `head_show.rs`, `legs.rs`, `wash_show.rs`,
      `ball_show.rs`, `ball_lighting.rs`, `schedule.rs`, and `show_state.rs` and
      `test_mode.rs`, which step 7 writes anew. `face.rs` and `face_show.rs` stay.
      `brain/presets.rs` and `brain/shows.rs` stay, emptied in step 9.
- [ ] `deploy.sh` / `etc/remote-deploy.sh`: build and flash wireless firmware only when the
      workspace patches a wireless fixture; otherwise log `no wireless fixtures patched` and
      skip the step. Until then every deploy fails at the ball build, because `patch::BALL`
      is gone.

**Landed when** `cargo build` in the workspace fails only on the missing show, and
`./deploy.sh` reaches the Pi.

## 4. `cortex-build`

*Builds on step 3, the cleared tree, and compiles against the patch from steps 2 and 8.*

The band constants the show asks for are generated already (step 2); a name the code uses
that the definition lacks is a compile error at the use site.

- [ ] Sounds, a third job for `cortex-build` beside the ingest and the calendar, called from
      the rig crate's `build.rs` with the sound table:
  - **Check**: read each WAV in the sound table and fail on a wrong rate, depth or channel
    count, naming the file and the expected form; fail if the heartbeat is longer than a beat
    at the rate the open question on its length settles.
  - **Typed constants**: strip each WAV to raw little-endian PCM in `OUT_DIR`, and generate
    `sounds.rs` with, per recording, `pub static HEARTBEAT: Sound` holding its samples as a
    `&'static [i16]`, its table gain, and its length in frames and in DMX frames. The samples
    are `include_bytes!` of the raw file in a 2-byte-aligned static, viewed as `[i16]` in a
    `const` — no literal in the generated source, nothing parsed at run time, and the type is
    the samples, not bytes.
  - **Envelope**: run an analyser over the heartbeat — RMS over each 25 ms window at the
    40 Hz frame rate, a short decay so a window between the lub and the dub does not drop to
    nothing, normalised so the loudest window is 255 — and emit it as `HEARTBEAT_ENVELOPE:
    [u8; N]`, one byte per DMX frame of the recording. The rose plays this from the beat.
    `cargo` watches `sounds/`, so a new recording is a rebuild.
- [ ] The dark-stretch resolution stays; this show does not read it.

## 5. The face

*Builds on step 3.*

- [ ] `FaceState`'s `right_held` becomes `right_presses`, a counter that moves on the press,
      like the chords; the chord rule stays in the face. Its e-stop path is unchanged. Its own
      client goes: its topics go out through the brain's.

**Landed when** the e-stop still works and `face/…` still reports, under the brain's
connection.

## 6. The mouth, its own crate

*Builds on step 3.*

- [ ] `mouth/`, a new workspace member the brain depends on, with ALSA and `rusb` and its own
      `config.rs`; `cortex` knows nothing of it. Its `Sound` type is what the generated
      `sounds.rs` (step 4) is made of.
- [ ] An ALSA playback thread on `plughw:CARD=io2,DEV=0`, 48 kHz, stereo, period 256 frames,
      two periods of buffer (~11 ms), started from the ear's capture thread: the same open,
      negotiation, xrun recovery and reopen with backoff, turned round. It mixes voices from
      the compiled-in recordings into each period, each at its table gain, under a master
      gain `MOUTH_GAIN`.
- [ ] The seam: the brain's show closure publishes a `MouthCue` each frame — a trigger
      counter per sound and `mute` — on a latest-value seam the mouth reads every period. A
      counter that has moved starts that sound's voice at the next period boundary; `mute`
      zeroes the next buffer and kills every voice, so the cut lands within a period of the
      frame.
- [ ] The e-stop mutes it, as it darkens the lamps: the brain's closure sets `mute` while
      the face says the e-stop is pressed, which covers boot hold too, since the closure runs
      every frame whether or not anything goes out. A takeover does not mute.
- [ ] Health: through the brain's client, the topics of [`SHOW.md`](SHOW.md) §13. The `usb/`
      ones come from the ear's `rusb` probe, which already finds the device by VID:PID and
      reads its strings; it gains the speed and the port path. The `alsa/` ones are read back
      from the opened stream's hardware params, not from what was asked for. Published on
      open and again on every reopen, so a re-enumeration is visible. A failed open retries
      with backoff and never stalls the frame loop.
- [ ] In `mouth/src/config.rs`: the ear's capture settings turned into playback ones, its USB
      identity as `USB_VENDOR` and `USB_PRODUCT`, and `MOUTH_GAIN`.

**Landed when** `brain` plays the heartbeat at startup in a test build, and pulling the USB
cable shows in `mouth/state` and nothing in the journal's frame timing.

## 7. The show in `brain`

*Builds on steps 2–6, 8 and 9: the show is where everything meets.*

- [ ] `brain/src/heart.rs`: the oscillator — phase, period set per beat, the rate gliding
      toward its target through `cortex::envelope::Envelope` with `RATE_TAU` both ways,
      started at `REST_BPM`; the rose's dimmer read from `HEARTBEAT_ENVELOPE` by frames since
      the beat, `ROSE_FLOOR` below it; a beat counter for the mouth.
- [ ] `brain/src/presence.rs`: one distance, the eyeball's, stale → the tombstone; the two
      debounces `SCARE_CONFIRM_SECS` and `LEAVE_SECS`, and what arms the scare, as the open
      question settles it; publish `brain/presence` as the distance in paths, rounded to a
      hundredth, or `tombstone`, when it changes.
- [ ] `brain/src/show.rs`: the Director with the six states of [`SHOW.md`](SHOW.md) §2, the
      timers, the owl, test mode (chord; the right button fires the scare on press from any
      state, restarting one in progress; the left button is only half the chord; 15-minute
      timeout), the lamps: dark outside test mode, both lit in it, and the buttons dead
      outside it but for the chord. The constants of §10 with their reasons. The heart's
      target rate is linear in distance over `HEART_RANGE`, clamped, and rests on the
      tombstone; there is no state for someone being in the yard. The thunderclap's hops and
      the owl's gaps come from `cortex::draw::Draw`, keyed on the state's start, the head and
      the hop's number, so each head has its own stream. A hop is uniform over the disc
      (radius `√u · R`, not `u · R`, or they cluster at the centre), added to the preset's
      pose in degrees, clamped to the head's travel, and sent with `Slew::jump`. A disc in
      pan and tilt is an ellipse in the yard, narrower in pan the more steeply a head looks
      down; the old dance's `on_ball` corrected for that, and nothing here needs it exact.
- [ ] Per state, what goes into the frame — asking the patch for the rose, white, open and
      strobe bands, never a byte. "Dark" is dimmer 0 with the strobe channel open. Every
      head's `Light Strips`, `Automatic Mode` and `Reset` are parked in every state.

  | state | heads | wash | mouth |
  |---|---|---|---|
  | `blackout` | dark, on "Rose" | off | silent |
  | `idle` | `ROSE_HEAD`: "Rose", gobo Rose, `ROSE_COLOUR`, dimmer = heart envelope paced by the distance, held dark for the first `BUILDUP_SECS` after `respite` while the heads travel; others dark | off | heartbeat on each beat; owls |
  | `cut` | dark; still on "Rose" | off | mute |
  | `buildup` | "Thunderclap", gobo Open, colour White, motor speed fastest, dark | off | mute |
  | `thunderclap` | dimmer full, `STROBE_HEADS` on the strobe band at `STROBE_RATE`, others open; each head hops every `THUNDER_HOP_SECS` to a fresh uniform point inside `THUNDER_RADIUS_DEG` of its "Thunderclap" pose, motor speed fastest | off | thunderclap |
  | `respite` | dark, on "Thunderclap" | `DOOR_AMBER` | silent |

- [ ] `brain/state/show` and `brain/state/testmode` retained; the `show: a → b — why` log
      lines.

**Landed when** the show can be scared from the cabinet door with the eyeball absent, as
[`SHOW.md`](SHOW.md) §16 step 6.

## 8. The QLC+ workspace (`claw.qxw`, QLC+ 5.2.1 on `riverrain`)

Do this in QLC+, not in a text editor: the build ingests whatever QLC+ writes, and a
hand-edited file is a second author. Keep `claw.qxw` open in a text editor beside it to check
each save, because QLC+ does not show you everything it stores.

### 8.1 Fixtures

In *Fixture Manager*:

- [ ] Delete `Head red leg`, `Head green leg`, `Head blue leg`, `Wash red leg`,
      `Wash green leg`, `Wash blue leg` and `Ball`. Leave `Face` (address 1, 2 channels).
- [ ] Add four **UKing ZQ02015** in the `12 Channel` mode, the one with Pan Fine and Tilt
      Fine, named exactly `Head 1`, `Head 2`, `Head 3`, `Head 4`, at addresses
      **11, 23, 35, 47**, so they sit at 11–22, 23–34, 35–46, 47–58. Gaps are fine, overlaps
      fail the build.
- [ ] Add one **HQ Power VDPLPS36B2 LED Pinspot PAR36** (the door wash), `3 Channel` mode,
      named `Wash door`, at address **64** — a single DIP switch on the fixture.
- [ ] Save. In the file, each `<Fixture>` should read `<Address>` **one less** than QLC+
      showed you (`<Address>10</Address>` for `Head 1`, `63` for `Wash door`): the file is
      0-based. Universe `0` on every one.

The names become `patch::HEAD_1` … `patch::HEAD_4`, `patch::WASH_DOOR` and `patch::FACE`.

### 8.2 Scenes

In *Function Manager*:

- [ ] Delete the four scenes `Look at ball`, `Look at heaven`, `Sky corner A`, `Sky corner B`.
      The build fails on any scene `brain/presets.rs` does not list.
- [ ] Create a Scene named exactly **`Rose`**. In the *Scene Editor* add the four heads and
      set **only** Pan, Pan Fine, Tilt and Tilt Fine for each — rough values for now: the rose
      head pointing down at the path, the other three tilted into the balcony. Untick every
      other channel; add neither `Wash door` nor `Face`.
- [ ] Create **`Thunderclap`** the same way, all four heads roughly at the path.
- [ ] Save. In the file each scene must have exactly **four** `<FixtureVal>` lines, IDs
      matching the four heads, each holding exactly four channel/value pairs on the mode's
      pan, pan-fine, tilt and tilt-fine channel indices, e.g.

      ```
      <FixtureVal ID="11">0,128,1,0,2,40,3,0</FixtureVal>
      ```

      A fifth pair, or a line for the wash, is what the build will refuse, naming the
      preset, the fixture and the channel.
- [ ] Leave the E1.31 output as it is: unicast `10.8.0.3:5568`, priority 200, universe 1.
      `UniverseChannels` recounts itself.

The names become `presets::ROSE` and `presets::THUNDERCLAP`; aiming them for real is
[`SHOW.md`](SHOW.md) §16.

## 9. `brain` tables

*Needs the definitions from step 2 and the workspace from step 8; without them the show
has nothing to compile against.*

- [ ] `brain/presets.rs`: `"Rose"`, `"Thunderclap"`.
- [ ] `brain/shows.rs`: the city, and `("2026-10-31 18:00", "2026-10-31 21:00")`.
- [ ] `brain/sounds.rs`, new: the three recordings and their gains — see
      [`SHOW.md`](SHOW.md) §10. Everything else about a recording the build reads from the
      file (step 4).
- [ ] Post-process the recordings in `audio/` into `sounds/`. `audio/` holds them as found —
      `heartbeat.mp3` (0.76 s, mono), `owl-screech-1.mp3` (3.1 s, stereo),
      `thunderclap.mp3` (19.8 s, stereo), all MP3 at 44.1 kHz — and stays on the Mac;
      `sounds/` holds what the build reads, and ships. For each:
  - decode and convert to 48 kHz 16-bit mono WAV, `afconvert -f WAVE -d LEI16@48000 -c 1`;
  - trim the leading silence, MP3's encoder delay included, so the first sample is the
    sound: the heartbeat's lub lands on the beat and the thunderclap's crack on the light;
  - the heartbeat: one lub-dub, as short as it will go, and its length against the open
    question on it;
  - the thunderclap: cut to the length the lit thunderclap should last, since the state lasts
    exactly as long as the file — 19.8 s of strobe and hops is a long time — with a fade at
    the end;
  - the owl: a fade at each end, so it starts and stops without a click;
  - peaks normalised to just under full scale, and the levels between them left to the gain
    table.
  
  Named `heartbeat.wav`, `owl.wav`, `thunderclap.wav`. Add `sounds/` and `eyeball/` to `deploy.sh`'s list.

**Landed when** `cargo build` fails on `presets::ROSE` being unused and nothing else.

## 10. Stem and the cabinet

Stem needs nothing: it keeps no list of services.

- [ ] Two Ethernet leads and a spare: the PoE injector's data port to the Pi's `eth0`, and the
      injector to the camera. The Ethernet hub, on its own supply, only if a laptop needs the
      wire as well. The annotated feed is at `http://10.8.0.3:8080/` from the Mac.
- [ ] Speakers under the balcony, the io|2's outs to them, mix knob to the computer side.
- [ ] Heads on the balcony, left to right as named; DMX daisy-chained from the HAT through
      the four heads to the wash; 120 Ω terminator in the wash's DMX out. Set each head's
      address, mode and `bLnd` as the open question settles it; the wash's DIPs.
- [ ] `OWNED-DEVICES.md`: the speakers' make and model.

## 11. Commissioning

[`SHOW.md`](SHOW.md) §16, in its order. The two numbers it produces, `BUILDUP_SECS` and the
speaker levels, are the last edits before the 31st.
