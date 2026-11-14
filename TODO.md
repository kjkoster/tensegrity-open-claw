# TODO — the Headless Hug proof of concept

The work between the code as it is and the show in [`SHOW.md`](SHOW.md), in the order to do
it: Monday's test first, then Wrist's Run, then soldering, then clean up, then Wrist's sensing
and telemetry, then its motor under a console, then the brain's programme;
commissioning second to last, results and clean-up last. The full-scale rig is not here: it
gets a list of its own. Steps only; a step that is done is deleted. Where a step changes a design, the change goes in
[`DESIGN.md`](DESIGN.md), and the PoC's measurements are recorded by `brain/run`.
Wrist's open decisions are steps here, and so is the design of what its firmware does not do
yet.

---

## Test and dinner on Monday

With the test sculpture, Monday 2026-11-16. Everything bought before then.

- Buy: rubber bands, two hooks, a short cable and the guy wire; a plank and two G-clamps; for
  the motor, four M4 bolts as long as the flange, what it bolts to, a washer and a nut, with
  washers and nylon-insert locknuts.
- The 4700 µF / 50 V at the motor, once it arrives.
- Decide: a backup guy, or the bands' preload below the detent hold; into `SHOW.md` §3.1.
- Build the bridge: plank across the model's base, clamped to the table; the motor bolted on
  the load cell, the wire leaving the shaft along the cell's axis; the wire wrapped a few
  turns round the bare shaft, tail tied off, then cable, hook, bands, hook and wire to the
  node. Nothing fixed to the sculpture but the wire.
- By test, into the firmware's constants: `A_max`, the largest tug the band set takes. Which
  Direction reels in, noted for the desk.
- Pluck under takeover from QLC+: Hold, then Pluck; Amplitude small first, then up, watching
  the model.

## Wrist's Run

The target: the model swung by an open-loop sine, from the brain's `run` show state and from
QLC+ faders under takeover, its frequency found by hand.

- `fixtures/wrist.qxf`, now six channels, reinstalled in QLC+; Wrist and the brain deployed.
- On the model: Run at a small Amplitude, Frequency swept slowly through the sway, by eye, the
  right lamp against the shaft.

## Soldering

- Find a plank to mount Wrist's board and its wiring on, instead of a housing.
- Mount the motor supply on the DIN rail from the cabinet's mains inlet and fusing (add the
  inlet and fusing to `OWNED-DEVICES.md` if they are not there); its output through the 5 A
  fuse to a terminal block, which feeds the motor; label it 38 V.
- With everything mounted on the plank, into `OWNED-DEVICES.md` and the firmware's constants:
  the status LED, and the alarm's polarity on a forced stall, holding the shaft while the
  motor runs; once the wire is on, the tension the unpowered shaft holds by its detent.
- Wrist's board's supply, against the OSC refused by its WiFi stack (about 23 in 10 s at
  −51 dBm, in and out of a pluck): 220–470 µF and 100 nF across the NodeMCU's 3V3 and GND,
  on the header pins; 100–220 µF across VIN and GND if it is fed over a long lead or from
  chaos-pi's USB (no more than 220 µF there, for the port's inrush). The optos' common anode
  moved to 5 V with a 1N4148 in series with each cathode, so the step
  pulses' current is not on the radio's rail; or, short of that, 10 µF and 100 nF at the common
  anode where the cable leaves. Read the refused count in the `status:` lines before and after.
- Wire the home switch and the ADXL345 on the pins in `SHOW.md` §3.1.
- If the e-stop button takes a third contact block: fit it, and a relay on the motor's 38 V
  feed from it, its contacts rated for at least 50 V DC, so "limp when pressed" is hardware as well as DMX.

## Clean up

- `OWNED-DEVICES.md` keeps the camera, the Alesis and the speakers as owned, not in the
  cabinet, and gains the rest of Wrist's parts: the NodeMCU, the ADXL345, the load cell and
  its HX711.
- Hardware out of the cabinet: the camera and its PoE injector, the audio interface, their
  cables.
- Rename `claw-pi` to `chaos-pi`.
- `ngrep` installed on `chaos-pi` by its deploy, for watching Wrist's OSC arrive:
  `sudo ngrep -d wlan0 -W byline '' 'udp and src host 10.0.10.4'`.

## Wrist's board: sensing and telemetry

The target: E0, the accelerometer's trace on the dashboard from Wrist's board, the motor in
Blackout. The dashboard comes first because it is how everything after it is read.

- Buy: a microswitch for home; for the load cell, four bolts in
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
- `wrist/` firmware, sensing half: the ADXL345 over I²C, ±2 g full resolution, 200 Hz output
  read at 100 Hz in one 6-byte burst (not SPI: its pins are taken, and a fast four-wire bus
  down a cable to a moving structure is the fragile choice); the lock-in running; the waveform
  bundle and the events; LittleFS.
- Once the HX711 reads right at 10 samples a second, and not before, in case the board does
  not survive it: lift the chip's RATE leg (pin 15) off its ground pad and tie it to DVDD
  (pin 16), for 80 a second; confirm the readings again.
- E0 from `SHOW.md` §4, its numbers recorded by run; the placement decided; the static pull
  test done.

## Wrist's motor, driven from QLC+

The target: E1, Wrist homing, plucking and running from QLC+ faders under takeover,
through the brain's universe, with the e-stop and signal loss leaving it limp.

- The home switch on the bridge, at the wire's pay-out end.
- `wrist/` firmware, adaptive running: Run's sine with its frequency and phase no longer the
  desk's but a local oscillator's, phase-locked to the sway, so the shaft drives in phase with
  the structure's velocity at the amplitude the desk sets. Designed first: its band in Mode,
  homing, the ring-down that measures f and Q and how it is recorded, the PLL and the drive
  phase's adaptation, the velocity clamp and soft limits. The sketch so far:
  - *Position.* Positive reels in; 0 at the home switch, at the pay-out end with the bands
    still stretched; the drive centred on a rest position between it and the travel's end,
    so the wire is never slack.
  - *Sway signal.* One ADXL345 axis, the one with the largest ring-down, picked at a pluck,
    through a slow DC high-pass and nothing else: the lock-in is the band-pass.
  - *Lock-in.* `I = LPF(s·cos φ)`, `Q = LPF(s·sin φ)` over 3–5 cycles; `θ = atan2(Q, I)` the
    sway's phase against the oscillator, `a = 2√(I² + Q²)` its amplitude; worked out at
    20 Hz, not every tick, the chip having no floating point.
  - *PLL.* `e = wrap(−θ)`; `ω += K_i·e·dt`, `φ += K_p·e`, bandwidth about `f/100`, `ω`
    clamped to ±20% of the ring-down's `f`, seeded from it. Locked while `a` is above a
    floor and `|e|` under about 20° for five cycles. A loop this slow does not care about
    sensor latency or a dropped sample.
  - *Drive.* Run's sine at `φ + φ_off`; the loop does not regulate the sway's amplitude. The
    velocity clamped to what the model needs, about 1.2 × 2π·f·A at the largest f and A,
    so a bad estimate or a snapped fader can do no more than the show would.
  - *Drive phase.* `φ_off` absorbs every lag in the chain and is learned, not computed.
    Acquiring: drive at a seed amplitude, step `φ_off` by 90° when the sway does not grow in
    five cycles, keep the quadrant that grows fastest, bisect twice. Tracking, by extremum
    seeking: a ±8° square-wave dither over 30 cycles, `φ_off` moved by the difference in mean
    sway between the halves, at most 5° a step; paused while the amplitude changes; clamped to
    ±60° of the acquired value, back to acquiring if the clamp is hit repeatedly.
  - *Kept.* f, Q, the axis and `φ_off` in LittleFS, `φ_off` at most once a minute, the coded
    defaults only for a board with nothing kept; all reported over OSC, so the defaults are
    updated by hand.
  - *Reported.* Homed and locked; the oscillator's ω and φ, the PLL's error, the sway's
    amplitude, the drive's amplitude and phase offset, the regime, the shaft's position; the
    accelerometer's axes and the sway on each reading; the transitions and ring-down results
    as they happen.
- Calibrate the load cell per `SHOW.md` §3.1; measure the rest position, the travel and
  the tension limits for the band set; all into the firmware's constants.
- E1 from `SHOW.md` §4 under takeover from QLC+.

## The brain runs the programme

- Brain: test mode disabled, left in the code, as the calendar is.
- Brain: the OSC receiver takes Wrist's waveform bundle as well.
- The Director's states — `armed-pluck`, `pluck`, `armed-run`, `run` — with the button rules
  of `SHOW.md` §2.2, `brain/run`, and the lamp patterns of §2.3, Run's two sines included;
  the experiments' amplitudes and durations as constants in `brain/src/show.rs`, named, with
  units.
- Dashboard panels for each experiment: Pluck's f, Q and axis by run; Run's lock, drive phase,
  oscillator frequency, and sway amplitude against drive amplitude.
- E3, E4, E5 from `SHOW.md` §4, each run's numbers recorded.

## The cabinet, alongside Wrist and the brain

- Reorganise: the two Pis, the hub, the modem, the SSD, the supply and its terminals each in
  their place; 5 V, 12 V and 38 V runs apart; signal leads away from mains. Photograph it for
  README's *Attached hardware*.

## With the artist, once Wrist's motor runs from QLC+

- E2, the three ways to pull, with the dashboard on the table; the choice and the reasons
  recorded.

## Commissioning the whole

- Power the cabinet up cold with nobody touching anything: both Pis, broker, OSC bridge,
  brain, Wrist on the AP in Blackout, dashboard live.
- Press the e-stop in every state: Wrist releases at once; release: Wrist takes up whatever
  the frame asks.
- Power Wrist's board down and up mid-Run: the lamps go to steady 100% while its OSC is
  stale; it rejoins, homes and resumes the Run; the gap shows on the dashboard.
- Stop the brain mid-Run: Wrist releases at once.
- The full programme from the buttons, each run annotated.
- Walk README's setup sections against the cabinet as it is and fix the text where it lies.

## Results, decisions, clean-up

- The results: every experiment's numbers by `brain/run`; the model's height and mass at
  the top, its static stiffness from the pull test, the guy wire and band set with their
  preload, the backup guy or the preload below the detent hold, and the accelerometer's
  placement and why; Wrist's control-tick timing and frame age per session; the dashboard
  panels and the SQL behind them committed under `pi/memory-pi/grafana/`; the answers to the
  three questions, with the measurement behind each; and what the PoC did not settle.
- `DESIGN.md`: correct §3.10 by what was measured.
- Delete `CONTROL_LOOP.md`: its content is merged into `SHOW.md` and this file, and nothing
  links to it.
- Tag `<YYYY-MM-DD>-headless-hug-poc`; empty this file for the next show.
