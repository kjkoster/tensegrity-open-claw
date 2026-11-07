# TODO — the Headless Hug proof of concept

The work between the code as it is and the show in [`SHOW.md`](SHOW.md), in the order to do
it: clean up first, then Wrist's sensing and telemetry, then its motor under a console, then
the brain's programme; commissioning second to last, results and clean-up last. Steps only; a
step that is done is deleted. Where a step changes a design, the change goes in
[`DESIGN.md`](DESIGN.md), and the PoC's measurements go in [`POC-RESULTS.md`](POC-RESULTS.md).
Wrist's firmware is specified in [`CONTROL_LOOP.md`](CONTROL_LOOP.md); its open decisions are
steps here.

---

## 0. Clean up

- The PoC's workspace: `claw.qxw` patches `Face` and, once its definition exists, `Wrist`.
  Change the E1.31 output's unicast address to `10.0.10.1`, so the Mac works on the access
  point for this show; the brain binds what the workspace says.
- `OWNED-DEVICES.md` keeps the camera, the Alesis and the speakers as owned, not in the
  cabinet, and gains the 24 V supply, the 48 V supply (kept for the full-scale drive) and
  Wrist's parts: the motor, the NodeMCU, the ADXL345, the load cell and its HX711.
- Hardware out of the cabinet: the camera and its PoE injector, the audio interface, their
  cables.
- Delete `fixtures/dendrite.oscquery.json`, and `fixtures/wrist.xml`, a copy of the old
  `wrist.qxf`.

## 1. Wrist's board: sensing and telemetry

The target: E0, the accelerometer's trace on the dashboard from Wrist's board, the motor not
yet wired. The dashboard comes first because it is how everything after it is read.

- Buy: a `24V 6A` switching power supply; `4700uF 50V` capacitor; a 5 A fuse and inline
  holder; a JST XH 2.5 mm 12-pin housing and crimps, unless the motor came with a pigtail; a
  NodeMCU (ESP8266) unless one is on the shelf; an ADXL345 breakout and a cable for it; a
  bar-type load cell and an HX711 board whose RATE pin can be taken high; a microswitch for
  home; rubber bands, two hooks, a short cable and the guy wire; a small N-MOSFET and a 10 kΩ
  resistor, and three 1N4148, in case the bench tests need them; a plank, two G-clamps, four
  M4 bolts; Velcro dots; a luggage scale and string; a known mass for the load cell.
  Optional: a spare NC contact block for the e-stop button and a 24 V relay module for the
  motor feed.
- Revive `memory-pi` per README; broker at `10.0.10.2`, `recorder/greptime` saying `ok`.
- Settle the OSCQuery files. `fixtures/wrist.oscquery.json` is rewritten to the address space
  in `CONTROL_LOOP.md` §7; it and `face.oscquery.json` are first drafts. Open:
  - the prefix for our own keys (not `TENSEGRITY_`);
  - `health/…` and `sys/…` specified for every client, and a file for the brain;
  - booleans as `T`/`F` throughout;
  - whether Wrist serves its file over HTTP and mDNS.
- Settle where Wrist's firmware gets `memory-pi`'s address: the brain's is the workspace's
  unicast address, `memory-pi`'s has no single source yet.
- The OSC bridge on `memory-pi` (DESIGN §3.9): Wrist's bundles and events decoded from
  `wrist.oscquery.json` and ingested alongside MQTT; its unit and deploy in
  `etc-memory-pi/`; the schema into README.
- Provision the PoC dashboard from `etc-memory-pi/grafana/headless-hug-poc.json` with
  `allowUiUpdates: true`, so panels can be edited in the UI and exported back to the file;
  `deploy-memory-pi.sh` installs the provider and the file. Wrist's transitions, pluck
  results and protections, `brain/show` and `brain/run` as annotations. Grafana stays on
  loopback; the Mac reaches it over the SSH jump, now across the access point.
- `cortex-build`: read the `[ip mac]` bracket off a fixture's name (DESIGN §1.1) before the
  name becomes a constant; fail on a malformed bracket and on an address or MAC two fixtures
  share; hand both to firmware builds, which fail for a fixture without them; and render the
  static leases for every bracketed fixture as a `dhcp-host` drop-in for `claw-ap`, which
  the deploy installs.
- `cortex-build`: generate a C++ header for Wrist's firmware from the workspace and
  `wrist.qxf`: the universe, the port, Wrist's start address, its IP, and Mode's bands, so
  the firmware holds no second copy of the patch; the WiFi from `etc/claw-wifi.env`.
- Rewrite `fixtures/wrist.qxf` to `SHOW.md` §3.1's "2 Channel" mode: Mode in Maintenance,
  Blackout 0–15 as its no-function band, RampDown, Run and Pluck; Amplitude in Nothing.
  Import it into QLC+ and patch it at 500 as `Wrist [10.0.10.3 <its MAC>]`. Confirm the mask
  compiles to Blackout for Mode and holds Amplitude.
- `wrist/`: C++ on the Arduino core for the ESP8266, built on the Mac by `deploy.sh`, the
  OSC encoders generated from `wrist.oscquery.json` at build time; flashed over USB. Decide
  the build tool, `arduino-cli` or PlatformIO, and write it down.
- `wrist/` firmware, sensing half, per `CONTROL_LOOP.md`: WiFi at the address from the
  header, with modem sleep off; the sACN receiver, its 2.5 s timeout and Stream-Terminated;
  Blackout only, ENA released; the ADXL345 and the HX711; the lock-in running; the state and
  waveform bundles and the events; LittleFS; the LED.
- E0 from `SHOW.md` §4. Numbers into `POC-RESULTS.md`; the placement decided; the static
  pull test done.

## 2. Wrist's motor, driven from QLC+

The target: E1, Wrist homing, plucking, running and braking from QLC+ faders under takeover,
through the brain's universe, with the e-stop and signal loss leaving it limp.

- On the bench, with the 24 V supply, before wiring, and into `SHOW.md` §3.1 and the
  firmware's constants (`CONTROL_LOOP.md` §12): whether opto-on disables the driver or
  enables it, so ENA goes straight to GPIO0 or through the N-MOSFET; each opto's current at
  3.3 V, and below 7 mA the anodes to 5 V with a 1N4148 in each cathode lead; the alarm's
  resting state; the wire tension the unpowered shaft holds by its detent; the status LED at
  power-up and on a forced stall.
- Decide: a backup guy, or the bands' preload below the detent hold; into `SHOW.md` §3.1.
- Build the bridge: plank across the model's base, clamped to the table; the motor bolted on
  the load cell, the wire leaving the shaft along the cell's axis; the home switch at the
  wire's pay-out end; the wire wrapped a few turns round the bare shaft, tail tied off, then
  cable, hook, bands, hook and wire to the node. Nothing fixed to the sculpture but the wire
  and the accelerometer.
- Wire: supply → fuse → motor V+/V− with the capacitor at the motor; the NodeMCU on the
  cabinet's USB charger, sharing no ground with the motor; PU, DR, EN, AL, the home switch,
  the HX711 and the ADXL345 on the pins in `CONTROL_LOOP.md` §2. DIP: SW1–SW4 to the 800 row
  of the table on the body; SW5 so positive reels the wire in, noted in `SHOW.md` §3.1; SW6
  off.
- If the e-stop button takes a third contact block: fit it, and a relay on the motor's 24 V
  feed from it, so "limp when pressed" is hardware as well as DMX.
- `wrist/` firmware, motor half, per `CONTROL_LOOP.md`: the step generator in the Timer1 NMI;
  ENA's fail-safe; the velocity clamp and soft limits; Homing, Pluck, Run with the PLL and
  the drive phase's adaptation, RampDown; every protection in its §5.
- Calibrate the load cell per `CONTROL_LOOP.md` §7; measure the rest position, the travel and
  the tension limits for the band set; all into the firmware's constants.
- E1 from `SHOW.md` §4 under takeover from QLC+.

## 3. The brain runs the programme

- Brain: the calendar and test mode disabled, left in the code; the show always runs.
- Brain: an OSC receiver for Wrist's bundles on the access point's address, decoded from
  `wrist.oscquery.json`, feeding a seam; stale when no bundle has come for a set time.
- The Director's states — `armed-pluck`, `pluck`, `armed-run`, `run` — with the button rules
  of `SHOW.md` §2.2, `brain/run`, and the lamp patterns of §2.3, Run's two sines included;
  the experiments' amplitudes and durations as constants in `brain/src/show.rs`, named, with
  units.
- Dashboard panels for each experiment: Pluck's f, Q and axis by run; Run's lock, drive phase,
  oscillator frequency, and sway amplitude against drive amplitude.
- E3, E4, E5 from `SHOW.md` §4, each run into `POC-RESULTS.md`.

## 4. The cabinet, alongside 1–3

- Mount the 24 V supply on the DIN rail from the cabinet's mains inlet and fusing (add the
  inlet and fusing to `OWNED-DEVICES.md` if they are not there); its output through the 5 A
  fuse to a terminal block, which feeds the motor; label it 24 V.
- Reorganise: the two Pis, the hub, the modem, the SSD, the supply and its terminals each in
  their place; 5 V, 12 V and 24 V runs apart; signal leads away from mains. Photograph it for
  README's *Attached hardware*.

## 5. With the artist, once stage 2 is done

- E2, the three ways to pull, with the dashboard on the table; the choice and the reasons
  into `POC-RESULTS.md`.
- Sensor placement: where the accelerometer may live on the sculpture, with E0's three
  placements and their signal quality in hand, against what may be visible and what a housing
  can look like at full scale; the decision into `POC-RESULTS.md`.

## 6. Commissioning the whole

- Power the cabinet up cold with nobody touching anything: both Pis, broker, OSC bridge,
  brain, Wrist on the AP in Blackout, dashboard live.
- Press the e-stop in every state: Wrist brakes the sway, then releases within 30 s; release:
  Wrist homes and takes up whatever the frame asks.
- Power Wrist's board down and up mid-Run: the lamps go to steady 100% while its OSC is
  stale; it rejoins, homes and resumes the Run; the gap shows on the dashboard.
- Stop the brain mid-Run: Wrist releases at once.
- The full programme from the buttons, each run annotated.
- Walk README's setup sections against the cabinet as it is and fix the text where it lies.

## 7. Results, decisions, clean-up

- `POC-RESULTS.md`: every experiment's numbers by `brain/run`, the dashboard panels and the
  SQL behind them committed under `etc-memory-pi/grafana/`, and the answers to the three
  questions, with the measurement behind each.
- `DESIGN.md`: correct §3.10 by what was measured.
- The full-scale shopping list from the decisions: servo or NEMA 34 with E2's tension
  estimate behind it, the load cell, the anemometer, and Wrist's board, chosen on
  the control-tick timing measured throughout.
- Tag `<YYYY-MM-DD>-headless-hug-poc`; remove what only the PoC needed, keep what the real
  show will (Pluck, Run, the Director's button rules, Wrist); `SHOW.md` becomes the real
  show's file; empty this file for the next show.
