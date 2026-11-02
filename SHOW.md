# Headless Hug — Proof of Concept

**What this file is for.** The show file for the proof of concept: what the rig does, why, the
fixtures it uses and how each is set up. The audience is us. The show is a set of experiments
the rig runs on a one-metre model of the *Headless Hug* sculpture, stepped through from the
cabinet door and watched live in Grafana on `memory-pi`. Its results go in
[`POC-RESULTS.md`](POC-RESULTS.md). How to get from here to a running PoC is in
[`TODO.md`](TODO.md). The components it introduces — Wrist, Dendrite, the inner ear — are
described in [`DESIGN.md`](DESIGN.md), not here; this file says only how this show uses them.

The previous show's components are on their tags: `last-version-with-eyeball-for-group-detection`
and `last-version-with-mouth`.

---

## 1. What the PoC is for

The full-scale piece is a nine-metre tensegrity mast that must sway back and forth, in a
controlled manner, unattended, with an audience present, driven through a guy wire by a crank
at the structure's own first natural frequency. Three assumptions in that plan are measured
here, on the model, with the drive chain in miniature:

1. **Dendrite senses the sway** — frequency, amplitude and phase — and there is a placement on
   the structure where it does so best.
2. **Wrist moves the structure controllably** — amplitude follows the crank, the motion stays
   in one plane, and the drive stops when told.
3. **The brain finds the resonance by itself**, repeatably, from its own sensor and actuator.

A fourth question rides along: **which regime the drive is in.** A stiff line makes the model
follow the crank (amplitude equals stroke; resonance only lowers the load); a compliant line
makes the crank pump a resonance (amplitude grows over cycles; phase is the control). Every
experiment is run with both, by putting an elastic section into the line or leaving it out,
because the full-scale design branches on it.

What does not scale — the amplitude bound, the guy tension, the servo — is out of scope. What
does scale is the code, the fixture definitions, the measurement method and the decisions.

## 2. What the audience sees

The model on its wooden base; a line from a node on it down over a screw-eye to a crank on a
motor beside the base; the cabinet; a laptop on the access point showing the Grafana
dashboard. On the cabinet door the left lamp says what the rig is doing, the two buttons drive
the programme, the e-stop lets the crank go limp. The right lamp and the chord keep their
meaning from the code: test mode.

### 2.1 The calendar

The calendar stays, as in every show: `brain/shows.rs` lists a window per experiment day
(09:00–23:00, Amsterdam), and outside a window the Director is in blackout — lamps off, Wrist
in Free. Test mode, by the chord, is the code's way to run outside a window for a while, and
is kept unchanged for that purpose.

### 2.2 The programme

Five experiments, always one of them *armed*. The **left button selects**, the **right button
runs**, and while an experiment runs the right button **aborts** it. Nothing starts on a single
press of the button that selects, so a pluck can be set up with one hand on the model.

```
                 left                 left                 left
   armed Listen ──────▶ armed Kick ──────▶ armed Run ──────▶ armed Sweep ──────▶ armed Lock
        ▲  │               │  ▲              │  ▲               │  ▲               │   │
        │  │ right         │  │ right        │  │ right         │  │ right         │   │ right
        │  ▼               ▼  │              ▼  │               ▼  │               ▼   │
        │ Listen          Kick              Run               Sweep              Lock  │
        │  │ done/abort    │ done/abort      │ done/abort       │ done/abort       │    │
        └──┴───────────────┴────────────────┴──────────────────┴──────────────────┴────┘
                              (back to the same experiment, armed)          left from Lock ──▶ armed Listen
```

- At boot, and after blackout, the Director is in **armed Listen**, Wrist in Park.
- **Running** an experiment returns to the same experiment armed when it finishes or is
  aborted, so the right button repeats it. The left button is ignored while one runs.
- Every state is published as `brain/show`, retained, as the code already does
  (`armed-listen`, `listen`, `armed-kick`, `kick`, `run`, `sweep`, `lock`, `blackout`). Each
  start increments `brain/run`, retained — a run counter, not a second state topic — so a
  Grafana annotation and a line in `POC-RESULTS.md` name the same run.
- The line's state, stiff or compliant, is not something the brain can know. The operator
  publishes it from the Mac as `operator/line` (`stiff` or `compliant`, retained) before a
  run; the recorder stores it with the time, Grafana shows it as an annotation, the brain
  never reads it.

The experiments, each with what it is for:

| experiment | what the rig does | what it answers |
|---|---|---|
| **Listen** | Wrist stays in Park. The brain waits up to 60 s for a transient on Dendrite above the pluck threshold — the operator plucks the model by hand — then takes the next ten seconds as free decay, estimates f₁ and Q, publishes `brain/sway/f1_hz` and `brain/sway/q` retained, and finishes. | 1, and 3 by hand: the estimator on a human pluck, before the motor exists |
| **Kick** | Listen, with Wrist's Kick as the pluck: the brain bumps Wrist's Kick counter, Wrist makes one quarter-turn out and back, the decay is taken from the moment the crank is back in Park. | 3, method A: resonance by free decay, no hands |
| **Run** | Speed mode at `f1_hz` for sixty seconds, ramped up over the first five, then Park. | 2: the structure follows the crank at resonance |
| **Sweep** | Speed mode from 0.5 f₁ to 1.5 f₁ over 120 s; the brain publishes amplitude and the lag between crank angle and sway phase as it goes (`brain/sway/amp`, `brain/sway/lag_deg`); then Park. | 3, method B: the amplitude peak and the lag crossing 90° — visible only with compliance in the line |
| **Lock** | Lock mode: thirty seconds pumping at +90°, thirty holding at 0°, then braking at −90° until `brain/sway/amp` is below the lock floor; then Park. | the phase lock over the frame, pumping and braking — meaningful only with compliance |

Artist sessions and commissioning use QLC+'s takeover from any state, as on every show; the
Director does not know and does not need a state for it.

### 2.3 The lamps

The left lamp is the show's only light. The right lamp is test mode's, as the code has it.

| situation | left lamp |
|---|---|
| blackout | off |
| armed, experiment *k* of 5 | *k* flashes of 100 ms at full, then 1.5 s dark, repeating |
| Listen or Kick, waiting for the transient | steady 40% |
| any experiment, PLL locked | brightness follows the sway: (1 + sin φ)/2, so the lamp breathes with the model |
| any experiment, PLL not locked | steady 100% |
| Kick, the kick itself | 300 ms at full |
| finished | two 1 s fades up and down, then the armed pattern |
| aborted | three 100 ms flashes, then the armed pattern |

### 2.4 The e-stop, signal loss, and what the crank does

The e-stop is the rig's show-stop, as the README has it, and the show runs on underneath it.
What is specific to this show is what Wrist does — set by this show's `.qxf` and Wrist's
firmware, described in `DESIGN.md`, used here:

- **Pressed:** the mask puts Wrist's Mode in its no-function band, **Free**: the driver is
  disabled and the crank goes limp, as if the motor's power had been removed. Nothing turns
  under power while the stop is pressed. The same happens when Wrist loses the universe.
- **Released:** the live frame returns. Wrist comes out of Free **unhomed** — it cannot have
  seen the crank being turned while limp — and accepts only Hold, Home and Park until it has
  been parked again; a Run or Lock in the frame is refused and reported, not resumed. So the
  operator releases the stop, presses the right button to abort to Park, and Wrist homes and
  parks; then the experiment can be run again. The show never sees the stop; Wrist's rule is
  what stops motion from resuming on its own.
- **Boot with the stop pressed:** nothing goes out, Wrist stays Free, as every fixture does.

## 3. The fixtures

The workspace patches two fixtures: `Face` as the code has it, and `Wrist`.

### 3.1 Wrist

A JD57 integrated closed-loop stepper (NEMA 23) with a crank disc on its shaft, controlled by a
Waveshare ESP32-S3-RS485-CAN that follows the universe over the access point, and — for this
show — the Bluetooth central that relays Dendrite's samples to the brain as OSC. What it is and
why is `DESIGN.md`'s. How this show sets it up:

**Patch.** `fixtures/wrist.qxf`, mode "8 Channel", at the top of the universe, address 500.

| ch | name | group | this show's use |
|---|---|---|---|
| 1 | Mode | Maintenance; no-function band **Free** | Free · Hold · Home · Park · Speed · Lock |
| 2–3 | Frequency coarse/fine | — | mHz; crank revolutions per second in Speed, sway Hz in Lock (one revolution per sway cycle, so the same number); allowed 100–5000 |
| 4–5 | Phase coarse/fine | — | 0–65535 → 0–360°; Lock only |
| 6 | Offset | — | 0–255 → −90° … +90°, 128 = 0°; Lock only |
| 7 | Ramp | — | value × 0.02 rev/s² (0 reads as 1); limits every change of frequency and of mode |
| 8 | Kick | Maintenance; no-function band 0 | a counter 1–255; a change seen while Parked and homed is one kick |

**Motor.** Labelled "JD57 Series Closed Stepper Motor", **24–36 V DC**; the label is what
counts, over the seller's listing and over the manual, which is for a sibling revision.
Current is set at the factory, 4 A, with no switch for it. One 12-way 2.5 mm XH header, left
to right: AL− AL+ EN− EN+ DR− DR+ PU− PU+ V− V− V+ V+, the power pins doubled. Six DIP
switches:

| switch | setting | why |
|---|---|---|
| SW1–SW4 | ON, OFF, ON, ON | 1600 pulses per revolution, per the table on the body; 5 rev/s is 8 kHz of pulses |
| SW5 | as set at commissioning | the motor's direction (off counter-clockwise, on clockwise), which is Wrist's positive direction |
| SW6 | off, always | "internal debugging, do not switch on" |

What the sibling's manual says and the PoC relies on: the inputs take 5–24 V at 7–16 mA; a
pulse is at least 2.5 µs high and 2.5 µs low; direction must be steady 5 µs before the pulse
edge; 200 kHz at most. The alarm is an opto-isolated open collector, 30 V and 50 mA; it
latches, and resets only when the pulses stop or the power is cycled, and a reset while
pulses continue restarts the motor abruptly, so Wrist stops pulsing before it clears an
alarm. The alarm line is the motor's only feedback: no encoder output, no in-position output,
no serial port.

Supply: a **24 V, 5–6 A switching supply**, through a 5 A fuse, with 4700 µF / 50 V across V+
and V− at the motor. If the e-stop button has a spare contact block, a relay on the motor's
feed from that contact makes "limp when pressed" a hardware fact as well as a DMX one; without
one, Free over DMX is what the PoC has, and the full-scale rig gets the relay regardless.

**Board.** 7–36 V in, straight from the motor's 24 V supply. Five signals: PU and DR on the
outside connector's GPIO1/GPIO2; **EN**, AL and the **home switch** on a pigtail from the
internal header. PU, DR and EN through a 74HCT14 on the header's 5 V (the S3 is 3.3 V); AL−
to ground, AL+ to a GPIO with its pull-up; the microswitch between a GPIO with its pull-up
and ground. EN is wired because Free must actually de-energise the driver. Address
`10.0.10.3`, set on itself from `etc/claw-wifi.env`. RS-485 unused this show; termination
jumper off.

**Crank and home.** An 8 mm flange coupling on the shaft, drilled at 3, 5 and 8 mm radius; a
tab on the disc trips the microswitch once per revolution. Home is the switch's rising edge;
**neutral is home plus `HOME_OFFSET_DEG`**, measured at commissioning as the angle at which the
crank arm is perpendicular to the line with the model at rest, and written into Wrist's
config with the date. Homing turns the crank slowly (0.2 rev/s) in the positive direction
until the switch; Park means "home if unhomed, then go to neutral". In Speed and Lock the
switch is also the index: two revolutions without it, or an edge more than 5° from where
it should be, and Wrist goes Free and says why.

**Line.** 0.3 mm braided Dyneema from the crank pin over a screw-eye at the model's base to
the chosen node — the stiff line. The compliant line adds 10 cm of 1 mm shock cord or a
rubber band in series, pretensioned so it never slackens at the largest sway. The motor sits
on a wooden bridge spanning the model's base, clamped to the table; nothing is fixed to the
sculpture.

### 3.2 Dendrite, in this show

A micro:bit V2 on a battery pack, Velcroed to the model, streaming its accelerometer to Wrist
over Bluetooth; Wrist relays to the brain. The three placements tried in Listen are the top
node, a mid-height node, and a strut. Dendrite's protocol, the relay and the estimator are
`DESIGN.md`'s.

### 3.3 Memory and the dashboard

`memory-pi` as the README has it, with one provisioned dashboard for this show,
`etc-memory-pi/grafana/headless-hug-poc.json`: Dendrite's axes, `brain/sway/*`, `wrist/*`,
`brain/show` and `brain/run` and `operator/line` as annotations, `dendrite/link`. The Mac is
on the access point for the sessions and reaches Grafana over the SSH jump as today; QLC+'s
output for this show unicasts to `10.0.10.1` for the same reason, which the brain's receiver
binds because the workspace says so.

## 4. The experiments, in order

Each with what it needs working, what is read off the dashboard, and what decides it. The
numbers go in `POC-RESULTS.md`, by `brain/run`.

**E0 — Listen, by hand.** *Needs:* Dendrite, Wrist as bridge (motor side not yet), the
estimator, Memory. *Do:* Listen; pluck a centimetre; three times per placement, three
placements; `operator/line` set to `stiff`. *Read:* f₁ and Q as published, checked against ten
periods counted on the raw trace and cycles to half amplitude; second-mode and cross-axis
content per placement; the Bluetooth link's drops and counter gaps over the afternoon. Also by
hand: the static pull test, a luggage scale at the top, grams per millimetre of deflection.
*Decides:* whether the model has a usable first mode (Q < 3 means a heavier top or a tighter
net before anything else), the placement, and the reference numbers for everything after.

**E1 — Wrist from a fader.** *Needs:* Wrist's motor side, QLC+ on the access point. *Do:*
Home; Park; Speed at 1, 2, 3 rev/s; bump Kick; Hold; stop QLC+ and watch it go Free; e-stop
and watch it go Free; release and confirm it refuses Speed until Parked. *Read:*
`wrist/angle_deg` against the disc; Dendrite's trace unbroken while the motor runs — the two
share a radio. *Decides:* Wrist is a fixture.

**E2 — Three ways to pull.** *Needs:* E1, the artist. *Do:* under takeover, line at (a) the
top node, (b) a mid node, (c) a pair of lines from two anchors at 90°, one on the crank and
one sprung; Speed at f₁ from E0; 3, 5 and 8 mm radii; stiff and compliant. *Read:* amplitude
per millimetre of radius, cross-axis amplitude, how it looks. *Decides:* the pull geometry for
full scale, with the artist.

**E3 — Kick.** *Needs:* the Kick experiment. *Do:* ten Kicks. *Read:* f₁ and Q per run and
their spread against E0. *Decides:* question 3 by method A; pass is ten within 2% of each
other and of E0.

**E4 — Run and Sweep.** *Needs:* E3. *Do:* Run, then Sweep, stiff; then both compliant.
*Read:* in Run, amplitude against radius (stiff: equal; compliant: Q times) and
`brain/sway/lag_deg`; in Sweep, the amplitude and lag curves against frequency. *Decides:*
the regime, by measurement; and question 3 by method B.

**E5 — Lock.** *Needs:* E4, compliant. *Do:* Lock. *Read:* growth at +90°, hold at 0°, decay
at −90°; `brain/sway/phase_err_deg`, the difference between the phase the brain commanded and
the crank angle it got back stamped on Dendrite's samples. *Decides:* whether the phase lock
works with the Dendrite→Wrist→brain→Wrist round trip in it, and by how many degrees.

Throughout, without being an experiment: the Bluetooth link's drops, reconnects and gaps, and
Wrist's step timing with the link up and down. These decide nothing here; they are the
evidence for where the central lives at full scale.

## 5. Not in this show

A load cell in the line (the full-scale tension method; the Sweep's lag answers the same
question on the model). Any amplitude bound or supervisor beyond Free on stop and loss. Wind
and weather. Bluetooth on the brain. The ESP-based brain, RDM, the dials, the e-paper. Sound
and vision.
