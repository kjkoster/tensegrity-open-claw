# Tensegrity Open Claw — Building the Halloween show

**What this file is for.** The steps from the previous show to the one in
[`SHOW.md`](SHOW.md), in the order they depend on each other. Each step says what changes
and what proves it landed. The design decisions behind them are in the first section, so the
ones to overturn are overturned before the work starts rather than after.

The rig goes up in the yard only on the day itself: logistics and the weather allow nothing
earlier. So the work is six chunks, each standing on its own and testable on its own at the
bench — the Pi, the cabinet and, where a chunk needs one, a head and the camera on the table —
and the rig in the yard comes last:

- **Upgrade everything**, and make it work again (`DESIGN.md` §1.8). Every show's first chunk.
- **The eyeball**: the camera, the detector and the distance, with no show needed. Early,
  because it has the most left to do.
- **Clear the decks**: the Pis' power, `memory-pi`, and loose ends.
- **Sound**: the recordings and the mouth.
- **The show**: the fixture definitions and the workspace as far as the show needs them, and
  the show itself.
- **The rig**: everything in the yard, on the day.

Each ends in a brain that runs, so every evening ends on something working. The scare can be
rehearsed from the cabinet door in test mode once the show is in; the heart's pace needs the
eyeball, or test mode's left button in its place. Halloween is Saturday the 31st, six days
from the date of this file.

---

## Decisions taken here

Each is a choice, written down so it can be changed on purpose.

| decision | taken | alternative not taken |
|---|---|---|
| How inputs reach the show | Each sensor hands the show its own snapshot over its own latest-value seam, the way the face already does: the face's `FaceState`, and the eyeball's distance with the time it was seen. The show closure holds both readers. The lamps stay driven from the outgoing frame. A sensor on another machine later would arrive through an OSC receiver publishing on the same kind of seam, and nothing after it would change. | One door, `inputs::inject`, with an `Input` enum over every kind: `cortex` would have to learn the face's shape, which lives in the brain, and the frame loop would unpack the enum back into the snapshots it started as. |
| What a silent face means | Nothing: there is no face-staleness rule. The face is a task on the frame loop's own executor, so it cannot stop while frames go on going out; a panic in either ends the process. The e-stop is already read in `face::claim`, before the first frame. | A `FACE_STALE_MS` e-stop: it guards against a failure that cannot happen. |
| Where the eyeball runs | Inside the brain, on the Pi: its own crate, `eyeball/`, a workspace member the brain depends on, running as threads beside the frame loop. One machine, no network hop between the camera's reading and the show, and nothing to deploy but the brain. | A second machine, `eyeson`, sending OSC: a machine to set up, power, reach and deploy to, for headroom the Pi may turn out to have. A process of its own on the Pi: a socket between two processes that would otherwise share a seam. |
| How the eyeball shares the Pi | Its threads drop out of the brain's real-time scheduling to `SCHED_OTHER` before they start the model, so the threads the model's runtime spawns inherit that too, and the model gets at most three cores. A detector that eats every cycle then slows itself, never a DMX frame, Stem or `sshd`. | Inheriting the frame loop's `SCHED_FIFO`: a CPU-bound thread at the frame loop's own priority, starving everything below it up to the kernel's real-time throttle. |
| The eyeball's threads | OS threads with latest-value seams between them, like the brain's other blocking producers, and nothing on the frame loop's executor: nothing in the eyeball runs on a timer. Capture is paced by the camera, detection by itself, the preview by its client. | Embassy tasks, the rule on std elsewhere: an executor for work with no timer in it. |
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
| Presence in test mode | Outside test mode it is the eyeball's distance. In test mode the left button steps it in the eyeball's place: nobody, 0.5, 0.1. 0.1, not 0, so it races the heart and never scares; the right button scares. | A fake distance that wanders by itself: a heart nobody asked to quicken. |
| The face's lamps during the show | The left lamp is the rose's level in every mode, so the show can be watched against the sound from the door; the right lamp is test mode. | Both lamps for test mode only, as before: nothing to see of the show until the heads are driven. |
| Telemetry clients inside the brain | One connection, the brain's. The face, the mouth and the eyeball publish their subtrees, `face/…`, `mouth/…` and `eyeball/…`, through it, and have no `health/` topic of their own. A client and a will are for a process that can die on its own: the brain, the ball, Stem. | A client each, as the face and the ear had: none can die without the brain, so their wills only repeat `health/brain`, at the cost of a connection, a thread and a farewell apiece. |
| `rig-mqtt` is renamed `stem` | The crate is the Rust programs' view of Stem, the process they report to, so it is named for it and lives in `stem/` beside the daemon, the broker configuration and `rig_mqtt.py`, its Python half. It stays purely `no_std`, for firmware; the brain's client, which needs sockets and threads, stays in `cortex` as `cortex::telemetry`. | A directory of its own: Stem split across two places. The client inside the crate behind a `std` feature: std in a crate kept `no_std` for firmware. |
| Which heads strobe | Heads 2 and 3, the inner pair; 1 and 4 steady. | Any pair. A constant. |
| Wash mode | 3-channel RGB. | 5-channel: strobe and macros the show does not use and the e-stop must mask. |
| Where the camera connects | Its PoE injector's data port straight into the Pi's `eth0`, the camera on DHCP with `10.0.1.3` reserved for its MAC, so a factory reset brings it back where it is looked for. The Ethernet hub goes in only when a laptop needs the wire as well. | The camera on the Pi's WiFi: every video frame over 2.4 GHz. |
| The eyeball's model | A person detector, not a pose model — boxes are enough. **YOLOX-Nano**, Megvii's own weights from YOLOX's `0.1.1rc0` release (Apache-2.0), exported at **320×192** by `eyeball/models/export.py` and committed beside it: half the camera's 16:9 at 640×360, which keeps the full width of its view. 155 ms a frame on `claw-pi` with the brain running, 6.5 frames a second, where Megvii's published 416×416 file takes 400. Run by **`tract`**, pure Rust from crates.io: no prebuilt library downloaded, no C++ in the brain. Only class 0, *person*, is read. | Megvii's 416×416 file as published: 2.5 frames a second. 320×256 on the camera's 4:3, which crops the sides of the view: 5.2 frames a second, people a third taller in the model's picture. ONNX Runtime: four times faster than `tract` on the Mac, but `ort` downloads a prebuilt library; only Debian's package would do, at 1.21, and not before `tract` falls short. YOLOX-Tiny: 6.5 GFLOPs, too slow on the Pi's A53 cores beside the DMX loop; the step up on a Pi 4 with room, same code. YOLOv8/11-nano: AGPL. A CrowdHuman-trained detector: better on a dense bunch, but no clean first-party export, and the show reads only the front of the group. |
| How the eyeball reaches the picture | MJPEG over HTTP (`/axis-cgi/mjpg/video.cgi`), with Digest authentication, which is what an Axis camera asks for. Each part's `Content-Length` is read and exactly that many bytes taken. Decoded, letterboxed and re-encoded for the feed with `image` (whose JPEG decoder is `zune-jpeg`), so one crate does all the picture work. | RTSP/H.264: a video decoder in the brain's build. Scanning for JPEG markers instead of reading `Content-Length`: works until a frame contains the byte pair by chance. Allowing Basic authentication on the camera: a setting made by hand, lost on a factory reset. |
| Who debounces presence | The show. The eyeball reports the distance it saw in each frame; hysteresis is the show's policy. | The eyeball tracking people: more code where it is hardest to test. |
| The annotated feed | Wanted, as the operator's view of what the rig sees. Encoded on its own thread and only while a browser is connected, so an idle feed costs the detector nothing. Bound to the WireGuard address, `10.8.0.3:8080`, like the sACN receiver, so the Mac opens it directly. | Always-on encoding: a JPEG encode a frame on the Pi whether or not anyone is looking. The wildcard: the 4G uplink's public address would show the yard, and the children in it, to the internet. |
| How the eyeball measures | Two marks fixed in the camera's own picture, and the camera aimed so the rose and the path's entry land on them, watching the annotated feed; distance is linear between the marks. No calibration, only aiming. Needs the camera square to the path, which the feed shows at a glance. | A ground-plane homography from four marks: right at any angle, four points to measure and a matrix to get wrong. |
| Which point of a box is the person | Its trigger-side bottom corner: the leading foot. | The bottom centre: on a box the detector has merged over three children, the centre is the child in the middle of the bunch, not the one in front. |
| How sure the detector must be | `CONFIDENCE` 0.09, and non-maximum suppression at an IoU of 0.6. Low and loose on purpose: a false box can only land between the two marks and the show debounces everything, so a missed child costs more than a phantom; and children overlap in a side view, where the usual 0.45 would merge them into one. 0.09 and not the 0.3 the street photos suggested, because the camera's infrared picture scores people far lower: at the bench a half-hidden person scored 0.12, and the empty yard nothing above 0.02. | The usual 0.5 and 0.45: tuned for photographs, not a bunch of children seen from the side. |
| The ball | Removed from the code and the deploy, on the `last-version-with-ball` tag; its section stays in `DESIGN.md` and its definition in `fixtures/`, for when it comes back. `cortex-build`'s `addressing()` and the `stem` crate's `no_std` stay for the firmware to come. | Keeping it in the tree: a firmware build and a flashing step on every deploy, for a fixture this show does not patch. |
| The ear | Deleted, as asked. Its last version is on the `last-version-with-ear` tag; its capture thread is the mouth's starting point. | Keeping it running unread: a thread and an ALSA stream on the interface the mouth needs. |
| Fixture names | By position, `Head 1`–`Head 4` left to right from the street, `Wash door`. Roles (which head carries the rose) are the show's choice and change; positions do not. | By role. |
| Dark, on a head | Dimmer zero, with the strobe channel in its open band. The head's definition has no closed band, and the manual gives the strobe channel as a bare 0–255. | Shutter closed: there is nothing to close it to unless measuring finds a band. |
| The heads' light strips | The `Light Strips` channel joins the Effect group in the definition, with its `Off` band marked `NoFunction`. The show parks it, and the e-stop's mask sends it to `Off` with no new rule. | Leaving it in the Colour group: the e-stop holds Colour where it was, so strips a console lit would stay lit under the stop. |

### Open

Each needs a choice before the step that uses it.

| question | why it matters | recommendation |
|---|---|---|
| The smallest people at 320×192 | 640×360 has 360 rows across the camera's 64° of height where 640×480 has 480, so at half scale a 1.2 m child at 6 m is about 29 pixels tall in the model's picture against 38 at 320×256. The street photos started losing people below about 25. | Judge on the feed, someone walking the far side of the path. If the far children drop out, a 416×224 export keeps the full width at about 36 pixels, for roughly 320×256's frame rate. |
| Phantoms at the low threshold | `CONFIDENCE` is 0.09, well below the 0.39 a post on a street photo once scored as a person. The empty bench yard scored nothing above 0.02, but that is one frame. A phantom between the marks quickens the heart. | Judge on the feed, the yard at night with nobody in it, watching `eyeball/people`; raise `CONFIDENCE` until the phantoms go, as long as the far children stay. |
| The camera's exposure under the lightning | `Sensor.Exposure=auto` under the 10 ms ceiling re-meters after every flash, so the frames right after the lightning come out dark while it recovers. `Sensor.Exposure=hold` locks it, but locks whatever it was metering the moment it is set: at start-up in the dark, a lock onto a dark field. | Keep `auto` with the ceiling, as the old eyeball did, and see on the feed with a head flashing how many frames a flash costs; `hold` only if that is more than the scare's debounce can absorb, set once the yard is lit as it will be. |
| Does the show's light blind the camera | The camera sees by its own 850 nm illuminator, with its IR-cut filter out and exposure fixed. But with the filter out the sensor sees visible light too: the lightning's full white, the strobes and the amber wash all land on it, and a frame washed out by a flash is a frame with nobody in it. The heads are LEDs and give off almost no infrared, so their visible light is all the harm they can do. | Watch the feed with a head flashing at full in view. If the flash blanks frames, an 850 nm long-pass filter over the lens blocks visible light optically; otherwise nothing. |
| Is the door in the picture | `respite` holds while anyone is seen. If the camera does not see the doorstep, a group collecting sweets is the tombstone and the rose returns while they stand there. | Frame the door in, or make `LEAVE_SECS` long enough to cover a doorstep visit. |
| The strobe's rate | `STROBE_RATE` is "fast", which is no value, and the strobe band is unmeasured. A fast strobe aimed at children's faces sits in the 3–30 flashes a second that photosensitive-epilepsy guidance warns against. | `STROBE_RATE` in flashes a second, at most 3, read off the measured band; or no strobe, and let the hops carry the lightning. |
| `bLnd` on the heads | `SHOW.md` §12 says `bLAc`; `README.md`'s e-stop section says every fixture holds its last values. The manual gives `bLAc` as "return to zero", which may send the head home, and `HoLd` keeps it lit wherever it was — mid-thunderclap included, for the five seconds a brain restart takes. | Find out on one head what `bLAc` does. If it darkens and stays put, use it and make the README's rule "dark and still"; if it homes, decide between a homing head and a lit one. |
| A second group during `respite` | Respite ends only on an empty yard, so a group arriving behind the first is not scared. | Accept it: a scare re-armed under people still in the yard would hit them a second time. |
| The heartbeat's length | It must be shorter than one beat at `SCARE_BPM`: 0.46 s at 130. The recording in `audio/` is 0.76 s. | Let beats overlap: the mixer mixes voices anyway, and the rose takes the loudest of the envelopes playing. The check becomes "shorter than a beat at rest". |

---

## Upgrade everything

Every crate, the toolchain and the packages on `claw-pi` and paho-mqtt are at their newest
already, and the repo pins GreptimeDB v1.3.0. What is left:

- [ ] `memory-pi`: `sudo apt full-upgrade`, then a reboot, which also brings
      Grafana and Mosquitto up to date.
- [ ] `memory-pi`: `./deploy-memory-pi.sh` installs GreptimeDB v1.3.0. Its scans are now
      capped at 25% of RAM, so a Grafana query too big for the Pi fails rather than swapping.

## The eyeball, in the brain

Two crates, built on the Pi with the rest of the brain and running as threads inside it:
`retina/`, the camera set up over VAPIX and its newest frame handed on, and `eyeball/`, the
detector and the distance from those frames. Nothing from the old code is reused but the
camera's VAPIX settings; `DESIGN.md` §3.6 describes it. All of it is tested at the bench, with
the camera on the table plugged into the Pi; nothing here needs a show.

- [ ] **Frame timing under load**: the journal's frame timing holding on `claw-pi` with the
      mouth playing and the detector running.
- [ ] **The show's two timing constants from the measured rate**, 6.5 frames a second:
      `SCARE_CONFIRM_SECS` was "two frames" at ten, so 0.3 s; `EYEBALL_STALE_SECS`, "twenty",
      so 3 s.
- [ ] **A long UTP lead for the camera**: from the PoE injector by the cabinet to the
      camera's mount in the yard, with a spare. Outdoor-rated if it lies in the wet.
- [ ] **The camera's light control**: it reports `Properties.LightControl` available, which
      `param.cgi` does not show: VAPIX's separate light-control API, likely the IR
      illuminator's intensity, worth a look.

**Landed when** the feed at `http://10.8.0.3:8080/` shows you walking across the camera's
view with a box and a leading foot, `eyeball/distance` falls as you cross from the entry mark
to the trigger mark, and the journal's frame timing does not move while the detector runs,
while `retina/camera` reads `ok`.

## Clear the decks

- [ ] **The Pis' power.** `get_throttled` reports under-voltage and throttling since boot
      (`0x50000`) on `claw-pi` nearly idle, and the eyeball's three cores of detector
      will draw more. `claw-pi` and `memory-pi` both move off the USB hub's charge
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

## Sound

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

## The show

The fixture definitions and the workspace, as far as the show needs them to compile and to be
seen working, then the show. Tested at the bench with one head and the wash on the HAT's DMX
line, the speaker, and the camera; aiming for real waits for the yard,
on the day.

### Fixture definitions

The band constants the show asks for are generated on the model's struct as
`<CHANNEL>_<BAND>`; a label used twice within the model is left unnamed, so every name below
must be unique, and a name the code uses that the definition lacks is a compile error at the
use site.

- [ ] With one head on the bench, find out what `bLnd` → `bLAc` does when the DMX lead is
      pulled (the open question on it).
- [ ] In `UKing-ZQ02015.qxf`, measured on that head with Simple Desk, not taken from the manual:
  - the strobe channel: the strobe band, labelled `Strobe`, with its preset saying which end
    is fast; a closed band only if the head has one;
  - the real pan and tilt travel: the definition says 540° and 180°, from the library,
    unmeasured.
- [ ] Install both into QLC+ and relaunch it; both must appear in *Add fixture…*.
      Absence is the error signal: QLC+ 5 skips a definition that fails to parse silently.

### The QLC+ workspace (`claw.qxw`, QLC+ 5.3.0 on `riverrain`)

Do this in QLC+, not in a text editor: the build ingests whatever QLC+ writes, and a
hand-edited file is a second author. Keep `claw.qxw` open in a text editor beside it to check
each save, because QLC+ does not show you everything it stores.

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
for the day.

### The show's code

*The show is where everything meets: it needs the eyeball, the sound, and the fixture
definitions, workspace and tables above.*

- [ ] The thunderclap's hops: from `cortex::draw::Draw`, keyed on the state's start, the
      head and the hop's number, so each head has its own stream. A hop is uniform over the
      disc (radius `√u · R`, not `u · R`, or they cluster at the centre), added to the
      preset's pose in degrees, clamped to the head's travel, and sent with `Slew::jump`. A
      disc in pan and tilt is an ellipse in the yard, narrower in pan the more steeply a head
      looks down; the old dance's `on_ball` corrected for that, and nothing here needs it
      exact. The head constants of [`SHOW.md`](SHOW.md) §10 with their reasons.
- [ ] What is left of the heads per state, asking the patch for bands, never a byte:
  - in `thunderclap`, `STROBE_HEADS` on the strobe band at `STROBE_RATE`, the others open.

**Landed when**, at the bench, the scare runs from the cabinet door in test mode from every
state, as [`SHOW.md`](SHOW.md) §16 step 6: the bench head shows the rose, cuts, moves and
lights white, the wash lights amber, the speaker plays each sound on its cue, and the
`show: a → b — why` lines say why. Walking up to the camera quickens the heart, and the scare
fires on the crossing.

## The rig, on the day

Everything that can only happen in the yard. The software is finished before this starts;
what is left is hanging, wiring, aiming and two numbers. Stem needs nothing: it keeps no list
of services.

- [ ] Ethernet: a short lead from the PoE injector's data port to the Pi's `eth0`, and the
      long UTP lead from the injector to the camera. The Ethernet hub, on its own supply, only if a laptop needs the
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
