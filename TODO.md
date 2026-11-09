# TODO — the Headless Hug proof of concept

The work between the code as it is and the show in [`SHOW.md`](SHOW.md), in the order to do
it: OSC from Wrist seen on `claw-pi` first, then soldering, then clean up,
then Wrist's sensing and telemetry, then its motor under a console, then the brain's programme;
commissioning second to last, results and clean-up last. The full-scale rig is not here: it
gets a list of its own. Steps only; a step that is done is deleted. Where a step changes a design, the change goes in
[`DESIGN.md`](DESIGN.md), and the PoC's measurements go in [`POC-RESULTS.md`](POC-RESULTS.md).
Wrist's firmware is specified in [`CONTROL_LOOP.md`](CONTROL_LOOP.md); its open decisions are
steps here.

---

## OSC from Wrist, seen on `claw-pi`

The target: Wrist's board on the access point, sending its `sys/…` messages, and `ngrep` on
`claw-pi` showing them arrive. `memory-pi` is out of action, so `claw-pi` is where they are
seen. The access point, Wrist's address and where its OSC goes all come from `clockwork.dot`.

- Rust: the brain sends sACN by unicast to each recipient in `clockwork.dot`, at the address
  and port `cortex-build` reads from the dot, instead of to the universe's multicast group.
  The dot has sACN unicast only; Wrist's edge is the first.
- C++: Wrist's build reads `clockwork.dot` into a C++ header, as `cortex-build` does for Rust:
  the WiFi, Wrist's address and gateway, and each OSC recipient's address and port. It fails
  where `cortex-build` fails.
- `wrist/`: C++ on the Arduino core for the ESP8266, built on the Mac by `deploy-chaos-pi.sh`, the
  OSC encoders generated from `wrist.oscquery.json` at build time; built with `arduino-cli`,
  the core and libraries pinned in a `sketch.yaml` profile. Flashed from `claw-pi`, with
  Wrist's board on its USB: `deploy-chaos-pi.sh` copies the image across and runs `esptool` there.
  Delete the bench sketch, `wrist/bench/`, once the firmware moves the motor.
- `wrist/` firmware, first: WiFi at the header's address, with modem sleep off; the `sys/…`
  messages to each OSC recipient in the dot.
- On `claw-pi`: `ngrep` installed by the deploy, and
  `sudo ngrep -d wlan0 -W byline '' 'udp and src host 10.0.10.4'` showing the messages.
  Only those to `claw-pi`'s own address, `10.0.10.2`, arrive: with `memory-pi` off the
  network nothing answers ARP for `10.0.10.3`, so Wrist's packets to it never leave the board.

## Soldering

- On the bench, into `OWNED-DEVICES.md` and the firmware's constants: the status LED and the
  alarm on a forced stall, holding the shaft while `wrist/bench` plucks; once the wire is on,
  the tension the unpowered shaft holds by its detent.
- Find a plank to mount Wrist's board and its wiring on, instead of a housing.
- Wire: the 4700 µF / 50 V at the motor once it arrives; the home switch and the ADXL345 on the pins in
  `CONTROL_LOOP.md` §2.
- If the e-stop button takes a third contact block: fit it, and a relay on the motor's 38 V
  feed from it, its contacts rated for at least 50 V DC, so "limp when pressed" is hardware as well as DMX.
- Mount the motor supply on the DIN rail from the cabinet's mains inlet and fusing (add the
  inlet and fusing to `OWNED-DEVICES.md` if they are not there); its output through the 5 A
  fuse to a terminal block, which feeds the motor; label it 38 V.

## Clean up

- `OWNED-DEVICES.md` keeps the camera, the Alesis and the speakers as owned, not in the
  cabinet, and gains the rest of Wrist's parts: the NodeMCU, the ADXL345, the load cell and
  its HX711.
- Hardware out of the cabinet: the camera and its PoE injector, the audio interface, their
  cables.
- Delete `fixtures/dendrite.oscquery.json`, and `fixtures/wrist.xml`, a copy of the old
  `wrist.qxf`.
- Rename `claw-pi` to `chaos-pi`.

## Wrist's board: sensing and telemetry

The target: E0, the accelerometer's trace on the dashboard from Wrist's board, the motor in
Blackout. The dashboard comes first because it is how everything after it is read.

- Buy: a microswitch for home; rubber bands, two hooks, a short cable and the guy wire; a
  plank, two G-clamps; for the motor, four M4 bolts as long as the flange, what it bolts to, a
  washer and a nut, with washers and nylon-insert locknuts; for the load cell, four bolts in
  its threads (M4 or M5, try them), shorter than the threads are deep, and spacers so only the
  bar's ends are clamped; Velcro dots; a luggage scale and string; a known mass for the load cell.
  Optional: a spare NC contact block for the e-stop button and a relay module for the motor
  feed, contacts rated for at least 50 V DC.
- Revive `memory-pi` per README; broker at `10.0.10.3`, `recorder/greptime` saying `ok`.
- Settle the OSCQuery files, `wrist.oscquery.json` and `face.oscquery.json`. Open:
  - `health/…` specified for every client, `sys/…` for every client but Wrist, and a file
    for the brain;
  - booleans as `T`/`F` throughout;
  - whether Wrist serves its file over HTTP and mDNS.
- The OSC bridge on `memory-pi` (DESIGN §3.9): Wrist's bundles and events decoded from
  `wrist.oscquery.json` and ingested alongside MQTT; its unit and deploy in
  `pi/memory-pi/`; the schema into README.
- Provision the PoC dashboard from `pi/memory-pi/grafana/headless-hug-poc.json` with
  `allowUiUpdates: true`, so panels can be edited in the UI and exported back to the file;
  `deploy-memory-pi.sh` installs the provider and the file. Wrist's transitions, pluck
  results and protections, `brain/show` and `brain/run` as annotations. Grafana stays on
  loopback; the Mac reaches it over the SSH jump, now across the access point.
- C++: Wrist's build reads `clockwork.qxw` and `wrist.qxf` into its header, as
  `cortex-build`'s ingest does for Rust: the universe, the port, Wrist's start address and
  Mode's bands, so the firmware holds no second copy of the patch.
- `wrist/` firmware, sensing half, per `CONTROL_LOOP.md`: the sACN receiver, its 2.5 s timeout and Stream-Terminated;
  Blackout only, ENA released; the ADXL345 and the HX711; the lock-in running; the state and
  waveform bundles and the events; LittleFS; the LED.
- Once the HX711 reads right at 10 samples a second, and not before, in case the board does
  not survive it: lift the chip's RATE leg (pin 15) off its ground pad and tie it to DVDD
  (pin 16), for 80 a second; confirm the readings again.
- E0 from `SHOW.md` §4. Numbers into `POC-RESULTS.md`; the placement decided; the static
  pull test done.

## Wrist's motor, driven from QLC+

The target: E1, Wrist homing, plucking, running and braking from QLC+ faders under takeover,
through the brain's universe, with the e-stop and signal loss leaving it limp.

- Decide: a backup guy, or the bands' preload below the detent hold; into `SHOW.md` §3.1.
- Build the bridge: plank across the model's base, clamped to the table; the motor bolted on
  the load cell, the wire leaving the shaft along the cell's axis; the home switch at the
  wire's pay-out end; the wire wrapped a few turns round the bare shaft, tail tied off, then
  cable, hook, bands, hook and wire to the node. Nothing fixed to the sculpture but the wire
  and the accelerometer.
- `wrist/` firmware, motor half, per `CONTROL_LOOP.md`: the step generator in the Timer1 NMI;
  ENA's fail-safe; the velocity clamp and soft limits; Homing, Pluck, Run with the PLL and
  the drive phase's adaptation, RampDown; every protection in its §5.
- Calibrate the load cell per `CONTROL_LOOP.md` §7; measure the rest position, the travel and
  the tension limits for the band set; all into the firmware's constants.
- E1 from `SHOW.md` §4 under takeover from QLC+.

## The brain runs the programme

- Brain: the calendar and test mode disabled, left in the code; the show always runs.
- Brain: an OSC receiver for Wrist's bundles, binding where `clockwork.dot` has its `osc`
  port, read by `cortex-build` as the sACN receiver's is, decoded from
  `wrist.oscquery.json`, feeding a seam; stale when no bundle has come for a set time.
- The Director's states — `armed-pluck`, `pluck`, `armed-run`, `run` — with the button rules
  of `SHOW.md` §2.2, `brain/run`, and the lamp patterns of §2.3, Run's two sines included;
  the experiments' amplitudes and durations as constants in `brain/src/show.rs`, named, with
  units.
- Dashboard panels for each experiment: Pluck's f, Q and axis by run; Run's lock, drive phase,
  oscillator frequency, and sway amplitude against drive amplitude.
- E3, E4, E5 from `SHOW.md` §4, each run into `POC-RESULTS.md`.

## The cabinet, alongside Wrist and the brain

- Reorganise: the two Pis, the hub, the modem, the SSD, the supply and its terminals each in
  their place; 5 V, 12 V and 38 V runs apart; signal leads away from mains. Photograph it for
  README's *Attached hardware*.
- Once it is delivered: fit the battery for the brain's ESP32-S3 on-board RTC, powered through
  the 4-pin J7 SH1.0 connector; the brain reads its time from the RTC at boot, and sets the
  RTC by NTP via the Pi.

## With the artist, once Wrist's motor runs from QLC+

- E2, the three ways to pull, with the dashboard on the table; the choice and the reasons
  into `POC-RESULTS.md`.

## Commissioning the whole

- Power the cabinet up cold with nobody touching anything: both Pis, broker, OSC bridge,
  brain, Wrist on the AP in Blackout, dashboard live.
- Press the e-stop in every state: Wrist brakes the sway, then releases within 30 s; release:
  Wrist homes and takes up whatever the frame asks.
- Power Wrist's board down and up mid-Run: the lamps go to steady 100% while its OSC is
  stale; it rejoins, homes and resumes the Run; the gap shows on the dashboard.
- Stop the brain mid-Run: Wrist releases at once.
- The full programme from the buttons, each run annotated.
- Walk README's setup sections against the cabinet as it is and fix the text where it lies.

## Results, decisions, clean-up

- `POC-RESULTS.md`: every experiment's numbers by `brain/run`, the dashboard panels and the
  SQL behind them committed under `pi/memory-pi/grafana/`, and the answers to the three
  questions, with the measurement behind each.
- `DESIGN.md`: correct §3.10 by what was measured.
- Tag `<YYYY-MM-DD>-headless-hug-poc`; empty this file for the next show.
