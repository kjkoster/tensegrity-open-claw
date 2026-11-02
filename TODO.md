# TODO — the Headless Hug proof of concept

The work between the code as it is and the show in [`SHOW.md`](SHOW.md), in the order to do
it: upgrade and clean up first, then the PoC with the most uncertain thing first, commissioning
second to last, results and clean-up last. Steps only; a step that is done is deleted. Where a
step changes a design, the change goes in [`DESIGN.md`](DESIGN.md), and the PoC's
measurements go in [`POC-RESULTS.md`](POC-RESULTS.md).

---

## 0. Upgrade and clean up

- Upgrade everything to its latest release and make the rig work again, as every show starts
  (DESIGN §1.8): Trixie packages on both Pis, the Rust toolchain, the crates, QLC+ on the Mac,
  Grafana and Greptime's pin on `memory-pi`.
- The PoC's workspace: `claw.qxw` patches `Face` and, once its definition exists, `Wrist`.
  Change the E1.31 output's unicast address to `10.0.10.1`, so the Mac works on the access
  point for this show; the brain binds what the workspace says.
- `OWNED-DEVICES.md` keeps the camera, the Alesis and the speakers as owned, not in the
  cabinet, and gains the micro:bit V2, the ESP32-C3s, the 24 V supply, the
  48 V supply (kept for the full-scale drive), the two Waveshare boards and the crank parts.
- Hardware out of the cabinet: the camera and its PoE injector, the audio interface, their
  cables.
- Cross-build `brain` on the Mac for `aarch64-unknown-linux-musl`, linked with `rust-lld`,
  and have the deploy ship the binary instead of the sources.
- Once a cross-built brain runs, free `claw-pi`'s disk of everything the build used:
  `~/tensegrity-open-claw/target`, the crate sources and `Cargo.*` the deploy no longer ships,
  and the Rust toolchain (`rustup self uninstall`, which takes `~/.cargo` and `~/.rustup`).

## 1. The riskiest thing first: Dendrite to Wrist to the brain, live

Two Rust firmwares on two radios not used from Rust before — a BLE peripheral on the
micro:bit and a BLE central on the S3 sharing its radio with WiFi — relaying into the brain
and onto a dashboard. If it is going to hurt, week one is where. Memory comes first in this
stage because the dashboard is how everything after it is read. The motor is stage 2.

- Buy: a micro:bit battery pack (2×AAA, JST); a luggage scale and string; Velcro dots.
- Revive `memory-pi` per README; broker at `10.0.10.2`, `recorder/greptime` saying `ok`.
- Provision the PoC dashboard from `etc-memory-pi/grafana/headless-hug-poc.json` with
  `allowUiUpdates: true`, so panels can be edited in the UI and exported back to the file;
  `deploy-memory-pi.sh` installs the provider and the file. Annotations from `brain/show`,
  `brain/run` and `operator/line`. Grafana stays on loopback; the Mac reaches it over the SSH
  jump, now across the access point.
- Restore the ball's firmware from `last-version-with-ball` into a branch and lift from it
  what both new firmwares need: WiFi provisioning from `etc/claw-wifi.env`, the sACN front
  end, the `no_std` MQTT client, the deploy and flashing integration in `deploy.sh` and
  `remote-deploy.sh`. Port the WiFi and sACN parts from the C3 to the S3 (Xtensa: `espup`).
- `dendrite-wire/`, `no_std`, workspace member, shared by path with the firmware: the GATT
  UUIDs, the packed sample layout (16-bit counter, three 16-bit milli-g axes, little-endian,
  N per notification from the MTU, two at the default), the health characteristic, and the
  OSC messages `/dendrite/sample` and `/dendrite/health` with their argument lists.
- `dendrite/` firmware, Rust on the nRF52833 on Embassy, flashed with `probe-rs`: the LSM303AGR
  at its 50 Hz data rate into the packed samples; the counter; health once a second;
  advertise by name, re-advertise on disconnect. Decide the BLE stack on the first afternoon
  and write it in `DESIGN.md`: `nrf-softdevice` (Nordic's SoftDevice behind Rust bindings) or
  `trouble` on `nrf-sdc`; either notifies. Measure the battery life once.
- `wrist/` firmware, relay half, Rust on the S3: join the AP at `10.0.10.3` (add `WRIST_ADDR`
  to `etc/claw-wifi.env` and list the access point's fixed addresses in README's interface
  table); scan for Dendrite by name, connect, subscribe, reconnect forever with backoff;
  forward each sample as OSC to `10.0.10.1:9001` with the crank angle (−1 until stage 2) and
  the uptime stamped on it; publish `dendrite/x|y|z` per sample, `dendrite/rate_hz`,
  `dendrite/battery_mv`, `dendrite/link` retained, `health/wrist`. Decide the BLE stack and
  write it down: `trouble` over `esp-wifi`'s controller (`no_std`, like the ball) or
  `esp32-nimble` under `esp-idf-svc` (std, the more travelled central). The criterion is
  holding the connection while the sACN receiver is up, which stage 2 tests; if the `no_std`
  route cannot, switch rather than fight.
- `innerear/`, `no_std`, workspace member: the PLL (principal axis, second-order loop, bandwidth
  a tenth of the tracked frequency, lock quality), the free-decay estimator (f₁ from zero
  crossings, Q from the log-decrement, a cycle count and an untrusted flag below three
  cycles), lag and phase error from stamped samples. Unit tests on synthetic decays with
  known f₁ and Q.
- Brain: the OSC receiver thread on the access point's address, UDP 9001, feeding the seam;
  staleness at 500 ms, `brain/sway/stale`; the `innerear` crate wired in; `brain/sway/*`
  decimated to 5 Hz.
- Brain: the Director's first two states, `armed-listen` and `listen`, the lamp patterns for
  them, `brain/run`, and `brain/shows.rs` with the experiment days' windows.
- E0 from `SHOW.md` §4, with `operator/line` published from the Mac with `mosquitto_pub -r`.
  Numbers into `POC-RESULTS.md`; the placement decided; the static pull test done.

## 2. Wrist: the crank as a DMX fixture

- Buy: a `24V 6A` switching power supply; `74HCT14` DIP or a 4-channel level-shifter module;
  `4700uF 50V` capacitor; a 5 A fuse and inline holder; a JST XH 2.5 mm 12-pin housing and
  crimps, unless the motor came with a pigtail; `aluminum flange coupling 8mm shaft`; a
  microswitch lever arm if the one on the shelf has none; 0.3 mm braided Dyneema; 1 mm shock
  cord or rubber bands; screw-eyes; a Dupont pigtail for the internal header; a plank, two
  G-clamps, four M4 bolts. Optional: two 10 mm ball-bearing pulleys; a spare NC contact block
  for the e-stop button and a 24 V relay module for the motor feed.
- On the bench, with the 24 V supply and a lead, before wiring, and into `SHOW.md` §3.1: with
  EN open, does the shaft hold or turn freely, and does 5 V across EN+/EN− flip it (Free must
  be the state with no holding torque); the alarm contact's resting state, open or closed, so
  `wrist/alarm` reads the right way up; the status LED's pattern at power-up and on a forced
  stall.
- Build the bridge: plank across the model's base, clamped to the table, motor bolted with
  its shaft horizontal and the disc over the edge, screw-eye at the model's base in line with
  the crank. Nothing fixed to the sculpture.
- Wire: supply → fuse → motor V+/V− with the capacitor at the motor; supply → board's 7–36 V
  input; board 5 V → 74HCT14; GPIO1/GPIO2 → HCT → PU/DR; a header GPIO → HCT → EN; AL and the
  microswitch to header GPIOs with pull-ups. DIP: SW1–SW4 ON, OFF, ON, ON (1600 pulses per
  revolution); SW5 for the direction chosen as positive, noted in `SHOW.md` §3.1; SW6 off.
- Drill the flange coupling at 3, 5 and 8 mm radius; a shoulder screw as the pin; a tab for
  the microswitch.
- If the e-stop button takes a third contact block: fit it, and a relay on the motor's 24 V
  feed from it, so "limp when pressed" is hardware as well as DMX.
- `fixtures/wrist.qxf`, 8 channels per `SHOW.md` §3.1: Mode and Kick in the Maintenance group
  with Free and 0 as their no-function bands; the rest in no group. Patch it at 500. Confirm
  the mask compiles to Free for Mode and 0 for Kick and holds the rest.
- `wrist/` firmware, motor half: the sACN receiver beside the central, step timer on the
  other core; EN, PU, DR at 1600 pulses per revolution, pulses at least 2.5 µs high and low,
  DR steady 5 µs before a pulse edge; AL latched, cleared only after the pulses have stopped,
  because a reset while pulsing restarts the motor abruptly; the mode machine
  Free/Hold/Home/Park/Speed/Lock; unhomed after
  any Free, accepting only Hold/Home/Park until parked; homing at 0.2 rev/s to the switch's
  rising edge with a 1.5-revolution alarm; `HOME_OFFSET_DEG` in `wrist/src/config.rs`; Park
  as home-then-neutral; the switch as index in Speed and Lock (two revolutions without it, or
  an edge more than 5° off, → Free and alarm); Kick on a counter change while parked and
  homed, re-baselined on leaving Free, abandoned on a mode change; Ramp limiting every
  change; the frequency band 100–5000 mHz; `wrist/mode`, `wrist/homed`, `wrist/refused`,
  `wrist/alarm` retained on change, `wrist/angle_deg` and `wrist/freq_mhz` at 10 Hz. Stamp
  the real crank angle on the relayed samples from here on.
- Lock's local oscillator, nudge and latency extrapolation; stub as Speed until stage 3.
- Commission `HOME_OFFSET_DEG`: under takeover, Speed at 50 mHz, Hold when the arm is
  perpendicular to the line, read `wrist/angle_deg`, write it in the config with the date,
  deploy.
- E1 from `SHOW.md` §4 under takeover from the Mac on the access point, Dendrite's trace on
  the dashboard throughout.

## 3. The brain runs the programme

- The remaining Director states — `armed-kick`, `kick`, `armed-run`, `run`, `armed-sweep`,
  `sweep`, `armed-lock`, `lock` — with the button rules and lamp patterns of `SHOW.md` §2;
  the Listen window opening on a transient above the pluck threshold; the Kick counter;
  Run's five-second ramp; Sweep's frequency schedule; Lock's three phases and its floor; all
  as constants in `brain/show`, named, with units.
- Dashboard panels for each experiment: the sweep's amplitude and lag against frequency,
  Lock's growth and decay, `brain/sway/phase_err_deg`.
- E3, E4, E5 from `SHOW.md` §4, stiff then compliant, each run into `POC-RESULTS.md`.

## 4. The cabinet, alongside 1–3

- Mount the 24 V supply on the DIN rail from the cabinet's mains inlet and fusing (add the
  inlet and fusing to `OWNED-DEVICES.md` if they are not there); its output through the 5 A
  fuse to a terminal block, which feeds the motor and the Waveshare board; label it 24 V.
- Reorganise: the two Pis, the hub, the modem, the SSD, the supply and its terminals each in
  their place; 12 V and 24 V runs apart; signal pigtails away from mains. Photograph it for
  README's *Attached hardware*.
- The second Waveshare board is not a spare: it is reserved for the wired-DMX node that takes
  the universe off the HAT so the brain can move to a Pi 4. Box it, label it, note the plan in
  `TODO.md`'s next section when this one is emptied.

## 5. With the artist, once stage 2 is done

- E2, the three ways to pull, with the dashboard on the table; the choice and the reasons
  into `POC-RESULTS.md`.
- Eye placement: where Dendrite may live on the sculpture, with E0's three placements and
  their signal quality in hand, against what may be visible and what a housing can look like
  at full scale; the decision into `DESIGN.md` §3.11.

## 6. Commissioning the whole

- Power the cabinet up cold with nobody touching anything: both Pis, broker, brain, Wrist on
  the AP and parked with Dendrite connected, dashboard live.
- Press the e-stop in every state: Wrist limp within a frame, nothing turning under power;
  release: Wrist refuses Speed and Lock until the right button has aborted to Park, then
  homes and parks.
- Power Wrist's board down and up mid-Run: limp on loss, rejoins, unhomed, refuses until
  parked; Dendrite reconnects to it without help.
- Reset the micro:bit mid-Run: `brain/sway/stale` rises, the Run continues, the sensor comes
  back, the gap shows on the dashboard.
- Kick ten times in a row; f₁ within 2% across the ten.
- The full programme from the buttons, stiff line, then compliant, each run annotated.
- Walk README's setup sections against the cabinet as it is and fix the text where it lies.

## 7. Results, decisions, clean-up

- `POC-RESULTS.md`: every experiment's numbers by `brain/run`, the dashboard panels and the
  SQL behind them committed under `etc-memory-pi/grafana/`, and the answers to the three
  questions and the regime question, with the measurement behind each.
- `DESIGN.md`: correct §1.9 and §3.10–3.12 by what was measured: the BLE stacks
  chosen, the regime the full scale is designed for, where the central lives (the link's
  drop and gap numbers are the evidence), the pre-show pluck as the arming measurement, and
  what the PoC did not settle.
- The full-scale shopping list from the decisions: servo or NEMA 34 with E2's tension
  estimate behind it, the gearbox, the load cell, the anemometer, the C3 Dendrite's IMU, a
  Bluetooth adapter for the Pi if the central moves there before the brain does.
- Tag `poc-headless-hug`; remove what only the PoC needed (Sweep, `operator/line`), keep what
  the real show will (Listen, Kick, Run, Lock, the Director's button rules, Wrist, Dendrite,
  the relay until it moves); `SHOW.md` becomes the real show's file; empty this file for the
  next show.
