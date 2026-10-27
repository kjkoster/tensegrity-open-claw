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
| How a recording becomes samples | `mouth-build` decodes the file as it was found (`symphonia`), resamples it to 48 kHz (`rubato`), keeps it stereo, trims below −60 dB of its peak at both ends, normalises its peak to −0.3 dBFS less its gain, pads it to whole DMX frames, and writes the frames to `OUT_DIR` as little-endian bytes. The generated source views `include_bytes!` of them as `&'static [[u8; 4]]` with `as_chunks`, in a `static`, and the mixer reads each sample with `i16::from_le_bytes`. | Converting by hand into WAVs in a second directory: a step to forget, and a second copy of every recording. An aligned static viewed as `[i16]` with `slice::from_raw_parts`: `unsafe`, and the byte order implicit. A `const fn` turning the bytes into an `[i16; N]`: runs into rustc's `long_running_const_eval` lint. |
| The volume of each recording | A gain in dB per recording in `brain/sounds.rs`, a `const` read by the build script, which applies it to the samples; the mixer only adds voices up. No master gain, the speakers' knobs set the whole. | A cargo feature: features are on-or-off and unify across the build, so they cannot carry a number. An environment variable: invisible in git, and different on every machine that builds. |
| What arms the scare | Only a crossing: the distance at or past the line after a positive value, held for `SCARE_CONFIRM_SECS`. Someone appearing past the line from the tombstone — back from the door, out of the house — is not scared. | Any distance at or past the line: scares people on their way out. |
| Test mode from `blackout` | Enters `idle`, so the rose and the heart can be looked at without a scare first. | Staying dark until the right button is pressed. |
| The flash before the thunderclap | A state of its own, `lightning`: two frames at full, silent, and the thunderclap cued on the frame after, so the light leads the crack. `LIGHTNING_SECS` is 50 ms, rounded to whole frames, at least one; two frames chosen at the bench. | The flash as the first frames of `thunderclap`: the recording and its envelope would no longer start together. |
| Presence before the eyeball | The yard is empty outside test mode. In test mode the left button steps it: nobody, 0.5, 0.1. 0.1, not 0, so it races the heart and never scares; the right button scares. | A fake distance that wanders by itself: a heart nobody asked to quicken. |
| The face's lamps during the show | The left lamp is the rose's level in every mode, so the show can be watched against the sound from the door; the right lamp is test mode. | Both lamps for test mode only, as before: nothing to see of the show until the heads are driven. |
| Telemetry clients inside the brain | One connection, the brain's. The face, the mouth and the eyeball publish their subtrees, `face/…`, `mouth/…` and `eyeball/…`, through it, and have no `health/` topic of their own. A client and a will are for a process that can die on its own: the brain, the ball, Stem. | A client each, as the face and the ear had: none can die without the brain, so their wills only repeat `health/brain`, at the cost of a connection, a thread and a farewell apiece. |
| `rig-mqtt` is renamed `stem` | The crate is the Rust programs' view of Stem, the process they report to, so it is named for it and lives in `stem/` beside the daemon, the broker configuration and `rig_mqtt.py`, its Python half. It stays purely `no_std`, for firmware; the brain's client, which needs sockets and threads, stays in `cortex` as `cortex::telemetry`. | A directory of its own: Stem split across two places. The client inside the crate behind a `std` feature: std in a crate kept `no_std` for firmware. |
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
| Is the door in the picture | `respite` holds while anyone is seen. If the camera does not see the doorstep, a group collecting sweets is the tombstone and the rose returns while they stand there. | Frame the door in, or make `LEAVE_SECS` long enough to cover a doorstep visit. |
| A second group during `respite` | Respite ends only on an empty yard, so a group arriving behind the first is not scared. | Accept it: a scare re-armed under people still in the yard would hit them a second time. |
| The heartbeat's length | It must be shorter than one beat at `SCARE_BPM`: 0.46 s at 130. The recording in `audio/` is 0.76 s. | Let beats overlap: the mixer mixes voices anyway, and the rose takes the loudest of the envelopes playing. The check becomes "shorter than a beat at rest". |
| The rose's colour | The show's table in 4.4 names `ROSE_COLOUR`, and nothing gives it a value yet. | A constant beside the others, naming a band of the colour wheel, chosen on the bench head. |
| The eyeball's threads | Your rule is Embassy on std too. Nothing in the eyeball runs on a timer: capture is paced by the camera, detection by itself, the preview by its client. | OS threads with latest seams between them, like the brain's other blocking producers, and nothing on the frame loop's executor. |
| Whether the Pi keeps up | YOLOX-Nano is expected at 150–300 ms a frame on the Pi 3B+ — three to six frames a second — and 60–120 ms on a Pi 4. The debounces and the staleness window were written for ten: `SCARE_CONFIRM_SECS` is "two frames", `EYEBALL_STALE_SECS` "twenty". A spare Pi 4 runs the same software unchanged. | Measure first (chunk 3). Then set the two constants from the measured rate, and swap in the Pi 4 if the 3B+ cannot hold about five frames a second. |
| ONNX Runtime inside the brain | It is C++. A crash in it ends the brain, which systemd restarts in seconds of signal loss. Its prebuilt binaries exist for 64-bit ARM, which is what the Pi runs (`aarch64`, Debian 13, glibc 2.41), and are downloaded by the Pi's first build of the brain, over 4G, once. `tract-onnx` is pure Rust, cannot crash that way, needs no download and runs YOLOX's operators, at some cost in speed. | `ort` while it holds the frame rate; `tract` if the speed allows it. |

---

## 1. Clear the decks

A brain that runs with nothing on it. Done: a placeholder show that keeps the rig in
`blackout` with test mode on the door; the ear gone (on the `last-version-with-ear` tag, whose
capture thread is the mouth's starting point in chunk 2); the `stem` crate holding Stem's Rust
half; one broker connection, with the face publishing through it; and the workspace patching
this show's fixtures, with no scenes yet. The previous show is the `2026-10-23-ADE-Amsterdam`
tag.

- [ ] **The Pis' power.** `get_throttled` reports under-voltage and throttling since boot
      (`0x50000`) on `claw-pi` nearly idle, and the eyeball's three cores of detector in
      chunk 3 will draw more. `claw-pi` and `memory-pi` both move off the USB hub's charge
      port onto the Durata multiport charger (5 V, 4.4 A across its ports), each on a short,
      thick lead; the SSD stays on `memory-pi`'s USB. Its 5.0 V leaves less margin above the
      Pis' 4.63 V under-voltage threshold than an official 5.1 V supply, so the leads matter.
      Both must then run under load without under-voltage, watched on
      `stem/throttling/now/under_voltage` and `memory/throttling/now/under_voltage` — before
      the eyeball's timing means anything. OWNED-DEVICES.md's hub, charger and `memory-pi`
      rows change with it.
- [ ] **`memory-pi` hangs at power-up, at random.** Red and green both lit solid, stuck before
      Linux starts, so no software on it can help; a soft reboot never does it. First, about
      five power cycles without the SSD: if every one boots, the SSD's start-up draw dipping
      the 5 V is the cause. Then a buffer capacitor, which is fitted in any case: a low-ESR
      electrolytic of 1000–2200 µF across 5 V and GND on the GPIO header — pin 2 or 4 to
      pin 6, after the Pi's input fuse — with the polarity right. Not on the 4-pin PoE
      header, which carries the Ethernet pairs, not 5 V. If it still hangs without the SSD, the
      cap does not answer it: try another supply, then another SD card, and otherwise move the
      card and the SSD to the spare Pi 4.
- [ ] **`memory-pi` struggles and fails once running.** Unplugged on 2026-10-27, so until it
      is back the rig has no broker and no recordings: every client retries with backoff and
      the show runs on, unreported. Troubleshoot later, beside the hang at power-up and the
      power item above, which may share a cause.
- [ ] **`claw-pi`'s cellular profile into the repo**, the one NetworkManager profile there the
      repo does not carry. Only when a 4G drop is affordable: every connection to both Pis
      runs over it. The profile today is APN `internet`, no PIN, DHCP for IPv4 and IPv6, and
      no routing metrics; the 4G watchdog works on the modem device, not the profile's name,
      so renaming it is safe.
      - `etc/claw-cellular.nmconnection`, named like `claw-ap` and `claw-eth0`: `type=gsm`,
        `apn=internet`, `ipv4.method=auto`, `ipv6.method=auto` as today. No `uuid`.
      - `etc/remote-deploy.sh` installs it with the other profiles and reloads NetworkManager,
        but never brings it up: a deploy over 4G arrives through it, and reactivating it would
        cut the deploy off halfway. A change takes effect at the modem's next reconnect.
      - Once, on `claw-pi`, after that deploy: `sudo nmcli connection delete cellular`.
      - README.md's 4G warning box becomes a plain note that the profile is in the repo, and
        says 4G is the default route because it is the only connection with a gateway; the
        routing metrics it mentions do not exist.
- [ ] The `stem` crate does without `alloc` wherever it can, for the firmware to come: the
      packets written into a caller's buffer rather than returned as a `Vec`.

**Landed when** `./deploy.sh` deploys clean, the e-stop holds and releases, the chord lights
the lamps, `face/…` and `brain/…` report under one connection, and `ps -T` on the brain shows
no `ear` and one telemetry thread.

## 2. Sound

The recordings, compiled in from `brain/sounds/` as they were found, and the mouth that plays
them. Done and running: `mouth/`, `mouth-build/` and `brain/sounds.rs`. A bench test that
played every recording in turn showed them playing cleanly and in step with the left lamp;
the show itself plays them now, through `brain/src/mouth_show.rs`. The lamps' PWM is at
20 kHz, above what leaked into the audio at 1 kHz.

- [ ] The 4G dongle's transmit bursts, a "ticka-ticka-ticka" under the sound that stops with
      the dongle out. Cable management: the dongle on a shielded USB extension, out of the
      cabinet, somewhere with good reception so it transmits quietly; a short USB lead for
      the io|2 with a ferrite at each end, its excess not coiled, on a hub port away from the
      dongle; power, data and audio in separate bundles, crossing at right angles; shielded,
      balanced leads to the speakers; the face's leads twisted with their grounds, shortened,
      and a ferrite at the GPIO header.
- [ ] Choose the final recordings by ear on the speakers; the ones in `brain/sounds/` are
      not final. Once they are, the heartbeat's trimmed length against the open question on
      it, and the check that it is shorter than a beat into the show, as a `const` assertion
      on `HEARTBEAT.envelope.len()`, since it is the show's rule and not the mouth's.
- [ ] The gains in `brain/sounds.rs`, by ear on the speakers, once their knobs are set by the
      thunderclap.

**Landed when** the bench test plays the heartbeat, the owl and the thunderclap through both
speakers in step with the left lamp, the e-stop cuts the sound within a buffer, and pulling the
io|2's USB shows in `mouth/state` and nothing in the journal's frame timing.

## 3. The eyeball, in the brain

A new crate, `eyeball/`, a workspace member the brain depends on, built on the Pi with the
rest of the brain and running as threads inside it. Nothing from the old code is reused but
the camera's VAPIX settings; `DESIGN.md` §3.6 describes it. All of it is tested at the bench,
with the camera on the table plugged into the Pi; nothing here needs a show.

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
      Today the brain has five threads, every one `SCHED_FIFO` 50 and free to run on any
      core: the main thread (the Embassy executor: the frame loop and the face), two unnamed
      ones (most likely `embassy-time`'s timer driver, which wakes the frame loop, and the
      signal thread `cortex::run` spawns), `sacn-in` and `telemetry-brain`. The mouth and the
      eyeball's threads are still to come. Only the frame loop, its timer driver and the
      mouth need real-time scheduling; the rest could drop to `SCHED_OTHER` as the eyeball's
      do. No core is isolated in `cmdline.txt`, and `/etc/systemd/system.conf` changes
      nothing, so there is nothing on the Pi to pull in yet. Whatever the session adds to
      either goes into `etc/` and git, installed by the deploy like `config-claw.txt` —
      `cmdline.txt` carries the SD card's `PARTUUID`, so the repo's copy cannot simply
      overwrite it.
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
  - the gobo channel: split "Fixed Gobo" (8–63) into a band per gobo, every one of the
    heads' gobos, each labelled by what it projects and the rose as `Rose`, and relabel 0–7
    `Open`;
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

Done and running, tried at the bench from the door: the Director in `brain/src/show.rs` with
its seven states, the timers,
the owl, test mode and the constants; the heart in `heart.rs`; presence in `presence.rs`, its
debounces and arming, with the yard empty outside test mode and the left button stepping it
inside; the show's sounds in `mouth_show.rs`; the face's left lamp as the rose and its right
lamp as test mode; `brain/show`, `brain/testmode`, `brain/presence` and the `show: a → b —
why` lines.

- [ ] Presence from the eyeball once chunk 3 lands: its distance in place of the tombstone
      outside test mode, stale after `EYEBALL_STALE_SECS` → the tombstone.
- [ ] The thunderclap's hops: from `cortex::draw::Draw`, keyed on the state's start, the
      head and the hop's number, so each head has its own stream. A hop is uniform over the
      disc (radius `√u · R`, not `u · R`, or they cluster at the centre), added to the
      preset's pose in degrees, clamped to the head's travel, and sent with `Slew::jump`. A
      disc in pan and tilt is an ellipse in the yard, narrower in pan the more steeply a head
      looks down; the old dance's `on_ball` corrected for that, and nothing here needs it
      exact. The head constants of [`SHOW.md`](SHOW.md) §10 with their reasons.
- [ ] Per state, what goes into the frame for the heads and the wash — asking the patch for the rose, white, open and
      strobe bands, never a byte. "Dark" is dimmer 0 with the strobe channel open. Every
      head's `Light Strips`, `Automatic Mode` and `Reset` are parked in every state.

  | state | heads | wash | mouth |
  |---|---|---|---|
  | `blackout` | dark, on "Rose" | off | silent |
  | `idle` | `ROSE_HEAD`: "Rose", gobo Rose, `ROSE_COLOUR`, dimmer = heart envelope paced by the distance, held dark for the first `BUILDUP_SECS` after `respite` while the heads travel; others dark | off | heartbeat on each beat; owls |
  | `cut` | dark; still on "Rose" | off | mute |
  | `buildup` | "Thunderclap", gobo Open, colour White, motor speed fastest, dark | off | mute |
  | `lightning` | dimmer full, open, still | off | silent |
  | `thunderclap` | dimmer full, `STROBE_HEADS` on the strobe band at `STROBE_RATE`, others open; each head hops every `THUNDER_HOP_SECS` to a fresh uniform point inside `THUNDER_RADIUS_DEG` of its "Thunderclap" pose, motor speed fastest | off | thunderclap |
  | `respite` | dark, on "Thunderclap" | `DOOR_AMBER` | silent |

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

## Small optimisations

Work the brain does at run time that the build could do once. None is needed for the show;
each can land whenever it is convenient.

- [ ] The sACN header as a template: `sacn_out::encode` allocates a `Vec` and rewrites the
      whole header every frame, though the packet identifier, the three layer lengths, the
      source name, the priority, the universe and the slot count are fixed per build.
      Generate the header once, keep one buffer in the frame loop, and write only the
      sequence and the slots into it — the frame loop's only per-frame allocation gone. The
      release packet is the same template with the terminated bit set.
- [ ] A fixed CID: `sacn_out::new_cid()` makes one from the clock and the process ID, so
      every restart appears on the network as a new source, where E1.31 expects a device to
      keep its CID. Derive it at build time, say from a hash of the rig's name, and
      `new_cid` goes.
- [ ] Park values in the patch: `Channel::park()` scans the channel's capabilities for its
      `NoFunction` band on every call, and the show will park several channels per head every
      frame. The build already finds those bands for the e-stop's mask; emit the park value
      per channel with them, and `park()` is one write.
- [ ] `patch::FRAME_WIDTH`: `frame_width(rig.patch)` derives the highest patched slot at
      startup, which the build already knows. A generated constant replaces the function and
      the comment defending it.
