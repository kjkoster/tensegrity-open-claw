# Tensegrity Open Claw — Building the Halloween show

**What this file is for.** The steps from the previous show to the one in
[`SHOW.md`](SHOW.md), in the order they depend on each other. Each step says what changes
and what proves it landed. The design decisions behind them are in the first section, so the
ones to overturn are overturned before the work starts rather than after.

The rig goes up in the yard only on the day itself: logistics and the weather allow nothing
earlier. So the work is five chunks, each standing on its own and testable on its own at the
bench — the Pi, the cabinet and, where a chunk needs one, a head and the camera on the table —
and the rig in the yard comes last:

1. **Clear the decks**: a brain that runs with nothing on it.
2. **Sound**: the recordings and the mouth.
3. **The eyeball**: the camera, the detector and the distance, with no show needed.
4. **The show**: the fixture definitions and the workspace as far as the show needs them, and
   the show itself.
5. **The rig**: everything in the yard, on the day.

Each ends in a brain that runs, so every evening ends on something working. The scare can be
rehearsed from the cabinet door in test mode from chunk 4 on; the heart's pace needs the
eyeball, or the reference footage replayed into it. Halloween is Saturday the 31st, six days
from the date of this file.

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
| The eyeball's model | A person detector, not a pose model — boxes are enough. **YOLOX-Nano**, `yolox_nano.onnx` from Megvii's own `0.1.1rc0` release of YOLOX, unmodified: Apache-2.0, 416×416 input, 1.1 GFLOPs, input `images`, output `output`, opset 11. Committed to `eyeball/models/`. Expected 150–300 ms a frame on the Pi 3B+, 60–120 ms on a Pi 4, through ONNX Runtime (`ort`, pinned at `=2.0.0-rc.10`). Only class 0, *person*, is read. At this picture — people three to six metres off, boxes over a hundred pixels tall — the accuracy between small detectors is noise next to their run time. | YOLOX-Tiny: 6.5 GFLOPs, too slow on the Pi's A53 cores beside the DMX loop; the step up on a Pi 4 with room, same code. YOLOv8/11-nano: AGPL. A CrowdHuman-trained detector: better on a dense bunch, but no clean first-party export, and the show reads only the front of the group. |
| How the eyeball reaches the picture | MJPEG over HTTP (`/axis-cgi/mjpg/video.cgi`), with Digest authentication, which is what an Axis camera asks for. Each part's `Content-Length` is read and exactly that many bytes taken. Decoded, letterboxed and re-encoded for the feed with `image` (whose JPEG decoder is `zune-jpeg`), so one crate does all the picture work. | RTSP/H.264: a video decoder in the brain's build. Scanning for JPEG markers instead of reading `Content-Length`: works until a frame contains the byte pair by chance. Allowing Basic authentication on the camera: a setting made by hand, lost on a factory reset. |
| Who debounces presence | The show. The eyeball reports the distance it saw in each frame; hysteresis is the show's policy. | The eyeball tracking people: more code where it is hardest to test. |
| The annotated feed | Wanted, as the operator's view of what the rig sees. Encoded on its own thread and only while a browser is connected, so an idle feed costs the detector nothing. Bound to the WireGuard address, `10.8.0.3:8080`, like the sACN receiver, so the Mac opens it directly. | Always-on encoding: a JPEG encode a frame on the Pi whether or not anyone is looking. The wildcard: the 4G uplink's public address would show the yard, and the children in it, to the internet. |
| How the eyeball measures | Two marks fixed in the camera's own picture, and the camera aimed so the rose and the path's entry land on them, watching the annotated feed; distance is linear between the marks. No calibration, only aiming. Needs the camera square to the path, which the feed shows at a glance. | A ground-plane homography from four marks: right at any angle, four points to measure and a matrix to get wrong. |
| Which point of a box is the person | Its trigger-side bottom corner: the leading foot. | The bottom centre: on a box the detector has merged over three children, the centre is the child in the middle of the bunch, not the one in front. |
| How sure the detector must be | `CONFIDENCE` 0.3, and non-maximum suppression at an IoU of 0.6. Low and loose on purpose: a false box can only land between the two marks and the show debounces everything, so a missed child costs more than a phantom; and children overlap in a side view, where the usual 0.45 would merge them into one. | The usual 0.5 and 0.45: tuned for photographs, not a bunch of children seen from the side. |
| The ball | Removed from the code and the deploy, on the `last-version-with-ball` tag; its section stays in `DESIGN.md` and its definition in `fixtures/`, for when it comes back. `cortex-build`'s `addressing()` and the `stem` crate's `no_std` stay for the firmware to come. | Keeping it in the tree: a firmware build and a flashing step on every deploy, for a fixture this show does not patch. |
| The ear | Deleted, as asked. Its last version is on the `last-version-with-ear` tag; its capture thread is the mouth's starting point. | Keeping it running unread: a thread and an ALSA stream on the interface the mouth needs. |
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
| The rose's colour | The show's table in 4.4 names `ROSE_COLOUR`, and nothing gives it a value yet. | A constant beside the others, naming a band of the colour wheel, chosen on the bench head. |
| Test mode from `blackout` | Turned on outside show hours, test mode stays dark until the right button is pressed, so the rose cannot be looked at without a scare first. | Test mode turned on in `blackout` enters `idle`. |
| The eyeball's threads | Your rule is Embassy on std too. Nothing in the eyeball runs on a timer: capture is paced by the camera, detection by itself, the preview by its client. | OS threads with latest seams between them, like the brain's other blocking producers, and nothing on the frame loop's executor. |
| Whether the Pi keeps up | YOLOX-Nano is expected at 150–300 ms a frame on the Pi 3B+ — three to six frames a second — and 60–120 ms on a Pi 4. The debounces and the staleness window were written for ten: `SCARE_CONFIRM_SECS` is "two frames", `EYEBALL_STALE_SECS` "twenty". A spare Pi 4 runs the same software unchanged. | Measure first (chunk 3). Then set the two constants from the measured rate, and swap in the Pi 4 if the 3B+ cannot hold about five frames a second. |
| ONNX Runtime inside the brain | It is C++. A crash in it ends the brain, which systemd restarts in seconds of signal loss. Its prebuilt binaries exist for 64-bit ARM, which is what the Pi runs (`aarch64`, Debian 13, glibc 2.41), and are downloaded by the Pi's first build of the brain, over 4G, once. `tract-onnx` is pure Rust, cannot crash that way, needs no download and runs YOLOX's operators, at some cost in speed. | `ort` while it holds the frame rate; `tract` if the speed allows it. |

---

## 1. Clear the decks

A brain that runs with nothing on it: Stem's Rust side in one crate, and one broker
connection. The old show is already replaced by a placeholder that keeps the rig in
`blackout` with test mode on the door, the ear is gone (on the `last-version-with-ear` tag,
whose capture thread is the mouth's starting point in chunk 2), and the workspace already
patches this show's fixtures, with no scenes yet. The previous show is the
`2026-10-23-ADE-Amsterdam` tag.

- [ ] **`rig-mqtt` becomes `stem`, with the std client**: the crate moves into `stem/`
      (`Cargo.toml`, `src/`) and is renamed, in the workspace members, the brain and
      `cortex`, and in `deploy.sh`'s list; `cortex::telemetry`'s client moves into it behind a
      `std` feature, with the broker's address a parameter, and the brain follows. The crate
      stays `no_std` without that feature, and does without `alloc` wherever it can, for the
      firmware to come.
- [ ] The face: `FaceState`'s `right_held` becomes `right_presses`, a counter that moves on
      the press, like the chords; the chord rule stays in the face. Its e-stop path is
      unchanged. Its own client goes: its topics go out through the brain's.

**Landed when** `./deploy.sh` deploys clean, the e-stop holds and releases, the chord lights
the lamps, `face/…` and `brain/…` report under one connection, and `ps -T` on the brain shows
no `ear` and one telemetry thread.

## 2. Sound

The recordings, compiled in, and the mouth that plays them. Tested on the placeholder show.

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

  Named `heartbeat.wav`, `owl.wav`, `thunderclap.wav`. Add `sounds/` to `deploy.sh`'s list.
- [ ] `brain/sounds.rs`, new: the three recordings and their gains — see
      [`SHOW.md`](SHOW.md) §10. Everything else about a recording the build reads from the
      file.
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
- [ ] `mouth/`, a new workspace member the brain depends on, with ALSA and `rusb` and its own
      `config.rs`; `cortex` knows nothing of it. Its `Sound` type is what the generated
      `sounds.rs` is made of.
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
- [ ] The placeholder show plays each recording on a right-button press in test mode, in
      turn.

**Landed when** the placeholder plays the heartbeat, the owl and the thunderclap through a
speaker on the bench, the e-stop cuts the sound within a buffer, and pulling the io|2's USB
shows in `mouth/state` and nothing in the journal's frame timing.

## 3. The eyeball, in the brain

A new crate, `eyeball/`, a workspace member the brain depends on, built on the Pi with the
rest of the brain and running as threads inside it. Nothing from the old code is reused but
the camera's VAPIX settings; `DESIGN.md` §3.6 describes it. All of it is tested at the bench,
with the camera on the table plugged into the Pi; nothing here needs a show.

- [ ] **The Pi's power, first.** `get_throttled` reports under-voltage and throttling since
      boot (`0x50000`) with the Pi nearly idle; three cores of detector will draw more. A
      supply that holds 5.1 V at the Pi under load — an official 2.5 A one, on a short, thick
      lead — before the timing below means anything, and `stem/stats/throttling/` watched
      after.
- [ ] **What the Pi manages.** The Pi is a 3B+ on 64-bit Trixie, so `ort`'s prebuilt
      ONNX Runtime fits; 905 MB of RAM and 1.8 GB free on the root partition. Download
      `yolox_nano.onnx` from the YOLOX `0.1.1rc0` release assets and commit it to
      `eyeball/models/`. Then a one-file crate on the Pi that loads it and times a frame with
      three threads, with `brain` running, watching `vcgencmd get_throttled` and the
      temperature while it runs; `tract-onnx` gets the same test. That settles the runtime and
      the open question on whether the Pi keeps up, before anything else is built. If
      compiling `ort` and `image` runs the Pi out of memory, build with `-j2`.
- [ ] **Core pinning**: sit down with which thread runs where — the frame loop, the
      detector and ONNX Runtime's pool, the mouth, Stem (pinned to cores 0–2 today) — and
      decide what pins where, from the code (`sched_setaffinity` per thread) or the units.
      Today the brain has nine threads, every one `SCHED_FIFO` 50 and free to run on any
      core: the main thread (the Embassy executor: the frame loop and the face), two unnamed
      ones (most likely `embassy-time`'s timer driver, which wakes the frame loop, and
      `signal-hook`'s signal thread), `sacn-in`, `ear`, `libusb_event` (from `rusb`), and one
      telemetry thread per client, `telemetry-brain`, `telemetry-face`, `telemetry-ear`.
      After this show's changes the ear and two of the telemetry threads are gone, and the
      mouth and the eyeball's threads are new. Only the frame loop, its timer driver and the
      mouth need real-time scheduling; the rest could drop to `SCHED_OTHER` as the eyeball's
      do. No core is isolated in `cmdline.txt`, and `/etc/systemd/system.conf` changes
      nothing, so there is nothing on the Pi to pull in yet. Whatever the session adds to
      either goes into `etc/` and git, installed by the deploy like `config-claw.txt` —
      `cmdline.txt` carries the SD card's `PARTUUID`, so the repo's copy cannot simply
      overwrite it.
- [ ] **`eth0` gets its address.** It is down and unconfigured: NetworkManager's default
      `Wired connection 1`, which asks for DHCP that nothing on the camera's wire answers.
      Give it a static `10.0.1.1/24` with no gateway, from a file in `etc/` and git that the
      deploy installs, like the access point's configuration.
- [ ] **The camera once**: if the `root` password really is lost, factory-reset it (control
      button held while powering up) and set one; re-address it to `10.0.1.90`; put the
      password in the eyeball's configuration only. Its PoE injector's data port into the
      Pi's `eth0`.
- [ ] **Reference footage, the poor man's test set**: after dark and under the camera's own
      infrared, record the MJPEG stream (`curl` on `/axis-cgi/mjpg/video.cgi` into a file)
      of yourself walking past the camera — walking up to a line and stopping, walking on
      past it, coming back, standing still, walking past behind, and a few people at once if
      you can get them. The yard's path if a dry evening allows a camera on a stand there;
      any walk across its view otherwise, since the model does not care where it is, and
      only the marks need the real yard. `capture.rs` takes a file in place of the camera, so the model, the confidence
      and the two marks are judged against the same walks, and again after every change; and
      a recording played into the brain is how the heart is heard quickening without anyone
      on the path.
- [ ] `eyeball/src/capture.rs`: the MJPEG stream over HTTP (`ureq`, with `digest_auth` to
      answer the camera's `401`), `video.cgi?resolution=640x480&fps=<rate>`, the rate a
      little above what the detector manages. The body is `multipart/x-mixed-replace`: read
      header lines to the blank one, parse `Content-Length`, read exactly that many bytes of
      JPEG; reconnect with backoff on any error. The reader decodes nothing: it puts each
      JPEG into a one-slot mailbox, replacing what is there, and the detector *takes* it — a
      take, not a latest-value read, which would run the model on the same frame twice. On
      every start, set the camera over VAPIX: 640×480, IR-cut filter off, exposure ≤ 10 ms,
      and the image rotation for the new mount; read it back and publish any disagreement.
      The parameter names, the sequence and the read-back are in the old eyeball on the
      `last-version-with-eyeball` tag (`eyeball/eyeball.py`), in Python; port them, they are the
      part that took finding out.
- [ ] `eyeball/src/detect.rs`: the thread drops to `SCHED_OTHER` first, then builds the
      session from the model compiled in with `include_bytes!` (`commit_from_memory`),
      optimisation level 3, three intra-op threads, one inter-op. Per frame:
  - **letterbox** with `image`: decode, resize to fit 416×416 keeping the aspect ratio, pad
    with grey **114** at the **bottom and right** (not centred), channels in **BGR** order,
    values **as they are, 0–255**, no normalisation (the export scales inside its graph),
    NCHW `f32`. Keep the scale `r` for the way back;
  - **run** it: input `images` `[1, 3, 416, 416]`, output `output` `[1, 3549, 85]`;
  - **decode**: 3549 rows = 52² + 26² + 13², one grid per stride 8, 16, 32 in that order,
    row-major; each row `[cx, cy, w, h, objectness, class_0 … class_79]`, and the box is
    **undecoded**: `x = (cx + grid_x) · stride`, `y = (cy + grid_y) · stride`,
    `w = exp(w) · stride`, `h = exp(h) · stride`; score = **objectness × class_0**; keep
    above `CONFIDENCE`; divide by `r` for frame pixels;
  - **non-maximum suppression** at IoU 0.6: sort by score, keep a box, drop those that
    overlap it more, repeat — ten lines, no crate.

  The ways it goes quietly wrong: centred padding shifts every box; 0–1 input makes the model
  see black and find nothing, with no error; skipping the `+grid` and `exp` puts tiny boxes
  in the top-left corner; objectness or class alone is not the score.
- [ ] `eyeball/src/yard.rs`: a box's point is its trigger-side bottom corner, the leading
      foot; distance = where its x falls between `TRIGGER_COLUMN` (0) and `ENTRY_COLUMN` (1),
      both fractions of the frame width, negative past the trigger column; boxes past the
      entry column are the street and are dropped; the furthest person is the smallest
      distance; no box between the columns is `+∞`.
- [ ] The seam: once per processed frame, the distance and the time it was seen go out on
      the eyeball's own latest-value seam, and never otherwise; the brain's show closure
      holds the reader beside the face's, and a quiet eyeball goes stale. It is the seam an
      OSC receiver would publish on, should a sensor ever move to another machine.
- [ ] `eyeball/src/preview.rs`: the annotated MJPEG feed (`tiny_http`), bound to
      `10.8.0.3:8080`, the WireGuard address, retrying the bind with backoff like the sACN
      receiver: boxes, each box's leading foot, the two marks labelled `rose` and `entry`,
      the distance, the frame rate. Drawn into a copy of the latest frame on its own thread,
      encoded with `image`, and only while at least one client is connected — no viewer, no
      encode.
- [ ] MQTT through the brain's connection: `eyeball/distance` (what it saw, each frame, in
      paths rounded to a hundredth, or `tombstone`), `eyeball/fps`, `eyeball/people`
      (telemetry only), `eyeball/camera`.
- [ ] `eyeball/src/config.rs`: camera host, user, password, or a footage file in its place;
      `TRIGGER_COLUMN` and `ENTRY_COLUMN` as fractions of the frame width; `CONFIDENCE` (0.3)
      and the suppression's IoU (0.6); the camera's frame rate; the feed's address.

**Landed when** the feed at `http://10.8.0.3:8080/` shows you walking across the camera's
view with a box and a leading foot, `eyeball/distance` falls as you cross from the entry mark
to the trigger mark, replayed footage gives the same numbers on every run, and the journal's
frame timing does not move while the detector runs.

## 4. The show

The fixture definitions and the workspace, as far as the show needs them to compile and to be
seen working, then the show. Tested at the bench with one head and the wash on the HAT's DMX
line, the speaker, and the camera or the reference footage; aiming for real waits for the yard
(chunk 5).

### 4.1 Fixture definitions

Both are already in `fixtures/`. `HQ-Power-VDPLPS36B2-LED-Pinspot-PAR36.qxf` is ours, both
modes from the datasheet, and needs nothing. `UKing-ZQ02015.qxf` came from QLC+'s library and
has been partly measured: the strobe channel's open band at 0, the motor speed's fast-to-slow
polarity, and `NoFunction` bands on `Automatic Mode` and `Reset` are in, and its colour bands
match the manual. The band constants the show asks for are generated already, on the model's
struct as `<CHANNEL>_<BAND>`; a label used twice within the model is left unnamed, so every
name below must be unique, and a name the code uses that the definition lacks is a compile
error at the use site.

- [ ] With one head on the bench, find out what `bLnd` → `bLAc` does when the DMX lead is
      pulled (the open question on it), and step through the gobo band for a rose (the open
      question on that).
- [ ] In `UKing-ZQ02015.qxf`, measured on that head with Simple Desk, not taken from the manual:
  - the gobo channel: split "Fixed Gobo" (8–63) into a band per gobo, the rose labelled
    `Rose`, and relabel 0–7 `Open`;
  - the strobe channel: the strobe band, labelled `Strobe`, with its preset saying which end
    is fast; a closed band only if the head has one;
  - `Light Strips`: into the Effect group, its 0–4 band `Off` with preset `NoFunction`;
  - the real pan and tilt travel: the definition says 540° and 180°, from the library,
    unmeasured.
- [ ] Install both into QLC+ and relaunch it; both must appear in *Add fixture…*.
      Absence is the error signal: QLC+ 5 skips a definition that fails to parse silently.

### 4.2 The QLC+ workspace (`claw.qxw`, QLC+ 5.2.1 on `riverrain`)

Do this in QLC+, not in a text editor: the build ingests whatever QLC+ writes, and a
hand-edited file is a second author. Keep `claw.qxw` open in a text editor beside it to check
each save, because QLC+ does not show you everything it stores.

The patch is in, named by role: `patch::ROSE_HEAD`, `patch::LEFT_STROBE_HEAD`,
`patch::RIGHT_STROBE_HEAD`, `patch::MIDDLE_WASH_HEAD`, `patch::DOOR_WASH` and `patch::FACE`.
The build fails on any scene `brain/presets.rs` does not list.

In *Function Manager*:

- [ ] Create a Scene named exactly **`Rose`**. In the *Scene Editor* add the four heads and
      set **only** Pan, Pan Fine, Tilt and Tilt Fine for each — placeholders for now, aimed
      in the yard on the day: the rose head pointing down, the other three tilted up. Untick
      every other channel; add neither `Wash door` nor `Face`.
- [ ] Create **`Thunderclap`** the same way, all four heads roughly level, and far enough
      from "Rose" that the bench head's move between them is seen.
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
chunk 5.

### 4.3 Tables

- [ ] `brain/presets.rs`: `"Rose"`, `"Thunderclap"`.
- [ ] `eyeball/` joins `deploy.sh`'s list.

### 4.4 The show's code

*The show is where everything meets: it needs chunks 2 and 3, and 4.1–4.3.*

- [ ] `brain/src/heart.rs`: the oscillator — phase, period set per beat, the rate gliding
      toward its target through `cortex::envelope::Envelope` with `RATE_TAU` both ways,
      started at `REST_BPM`; the rose's dimmer read from `HEARTBEAT_ENVELOPE` by frames since
      the beat, `ROSE_FLOOR` below it; a beat counter for the mouth.
- [ ] `brain/src/presence.rs`: one distance, the eyeball's, stale → the tombstone; the two
      debounces `SCARE_CONFIRM_SECS` and `LEAVE_SECS`, and what arms the scare, as the open
      question settles it; publish `brain/presence` as the distance in paths, rounded to a
      hundredth, or `tombstone`, when it changes.
- [ ] `brain/src/show.rs`, replacing the placeholder: the Director with the six states of
      [`SHOW.md`](SHOW.md) §2, the timers, the owl, test mode (chord; the right button fires
      the scare on press from any state, restarting one in progress; the left button is only
      half the chord; 15-minute timeout), the lamps: dark outside test mode, both lit in it,
      and the buttons dead outside it but for the chord. The constants of §10 with their
      reasons. The heart's target rate is linear in distance over `HEART_RANGE`, clamped, and
      rests on the tombstone; there is no state for someone being in the yard. The
      thunderclap's hops and the owl's gaps come from `cortex::draw::Draw`, keyed on the
      state's start, the head and the hop's number, so each head has its own stream. A hop is
      uniform over the disc (radius `√u · R`, not `u · R`, or they cluster at the centre),
      added to the preset's pose in degrees, clamped to the head's travel, and sent with
      `Slew::jump`. A disc in pan and tilt is an ellipse in the yard, narrower in pan the more
      steeply a head looks down; the old dance's `on_ball` corrected for that, and nothing
      here needs it exact.
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

**Landed when**, at the bench, the scare runs from the cabinet door in test mode from every
state, as [`SHOW.md`](SHOW.md) §16 step 6: the bench head shows the rose, cuts, moves and
lights white, the wash lights amber, the speaker plays each sound on its cue, and the
`show: a → b — why` lines say why. Replayed footage quickens the heart, and the scare fires
on the crossing.

## 5. The rig, on the day

Everything that can only happen in the yard. The software is finished before this starts;
what is left is hanging, wiring, aiming and two numbers. Stem needs nothing: it keeps no list
of services.

- [ ] Two Ethernet leads and a spare: the PoE injector's data port to the Pi's `eth0`, and the
      injector to the camera. The Ethernet hub, on its own supply, only if a laptop needs the
      wire as well. The annotated feed is at `http://10.8.0.3:8080/` from the Mac.
- [ ] Speakers under the balcony, the io|2's outs to them, mix knob to the computer side.
- [ ] Heads on the balcony, left to right as named; DMX daisy-chained from the HAT through
      the four heads to the wash; 120 Ω terminator in the wash's DMX out. Set each head's
      address, mode and `bLnd` as the open question settles it; the wash's DIPs.
- [ ] Commissioning: [`SHOW.md`](SHOW.md) §16, in its order — addresses, the e-stop, aiming
      "Rose" and "Thunderclap", `BUILDUP_SECS`, levels, the camera and its marks, then the
      show by walking it. `BUILDUP_SECS` and the speaker levels are the last edits.
- [ ] `OWNED-DEVICES.md`: the speakers' make and model.
