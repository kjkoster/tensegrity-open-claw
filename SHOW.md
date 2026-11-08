# Headless Hug — Proof of Concept

**What this file is for.** The show file for the proof of concept: what the rig does, why, the
fixtures it uses and how each is set up. The audience is us. The show is a set of experiments
the rig runs on a one-metre model of the *Headless Hug* sculpture, stepped through from the
cabinet door and watched live in Grafana on `memory-pi`. Its results go in
[`POC-RESULTS.md`](POC-RESULTS.md). How to get from here to a running PoC is in
[`TODO.md`](TODO.md). The component it introduces, Wrist, is described in
[`DESIGN.md`](DESIGN.md), and its firmware in [`CONTROL_LOOP.md`](CONTROL_LOOP.md), not here;
this file says only how this show uses it.

The previous show's components are on their tags: `last-version-with-eyeball-for-group-detection`,
`last-version-with-retina` and `last-version-with-mouth`.

---

## 1. What the PoC is for

The full-scale piece is a nine-metre tensegrity mast that must sway back and forth, in a
controlled manner, unattended, with an audience present, driven through a guy wire at the
structure's own first natural frequency. Three assumptions in that plan are measured here, on
the model, with the drive chain in miniature:

1. **Wrist senses the sway** — frequency, amplitude and phase — from an accelerometer on the
   structure, and there is a placement where it does so best.
2. **Wrist moves the structure controllably** — the sway follows the amplitude it is sent, the
   motion stays in one plane, and the drive brakes and stops when told.
3. **Wrist finds and holds the resonance by itself**, repeatably: frequency and Q from a
   pluck, the drive's phase learned while it runs, nothing calibrated by hand.

The drive is compliant by design (`DESIGN.md` §3.10); a stiff line is not tried.

What does not scale — the amplitude bound, the guy tension, the motor — is out of scope. What
does scale is the code, the fixture definitions, the measurement method and the decisions.

## 2. What the audience sees

The model on its wooden base, a small accelerometer on it on a cable; a wire from a node on it
down through a run of rubber bands to a motor beside the base, wrapped round the motor's shaft,
the motor on a load cell; the cabinet; a laptop on the access point showing the Grafana
dashboard. On the cabinet door the left lamp says what the rig is doing, the two lamps together
plot the drive and the sway while it runs, the two buttons drive the programme, the e-stop
brakes the drive and lets it go limp.

### 2.1 No calendar, no test mode

The show always runs: from boot to power-off the Director runs the programme. The calendar and
test mode stay in the code, disabled: the calendar never takes the Director to blackout, and
the chord does nothing, so the right lamp never says test mode.

### 2.2 The programme

Two experiments, always one of them *armed*. The **left button selects**, the **right button
runs**, and while an experiment runs the right button **aborts** it. Nothing starts on a press
of the button that selects.

```
   armed Pluck ◀──── left ────▶ armed Run
     │      ▲                    │      ▲
right│      │ done/abort    right│      │ done/abort
     ▼      │                    ▼      │
    Pluck ──┘                   Run ────┘
```

- At boot the Director is in **armed Pluck**, Wrist in Blackout.
- Armed, the brain sends Wrist's Mode as Blackout: Wrist's idle state, coils released. Ending
  or aborting an experiment is that Blackout, which Wrist takes by braking the sway to quiet
  first. Blackout is a Mode only: Amplitude is a size, and 0 means no size, not Blackout.
- The Director has no blackout state of its own. Wrist is otherwise in Blackout only when the
  e-stop's mask or a lost universe puts it there (§2.4).
- **Running** an experiment returns to the same experiment armed when it finishes or is
  aborted, so the right button repeats it. The left button is ignored while one runs.
- Every state is published as `brain/show`, retained, as the code already does
  (`armed-pluck`, `pluck`, `armed-run`, `run`). Each start increments `brain/run`,
  retained — a run counter, not a second state topic — so a Grafana annotation and a line in
  `POC-RESULTS.md` name the same run.

The experiments, each with what it is for. Their amplitudes and durations are named constants
in `brain/src/show.rs`, with units.

| experiment | what the rig does | what it answers |
|---|---|---|
| **Pluck** | The brain sends Pluck at the pluck amplitude. Wrist homes, brakes the model to quiet if it is not, tugs once, records the ring-down for 30 s, adopts f and Q if the peaks are clean, persists them and returns to rest. The brain holds Pluck long enough for all of that, then finishes. | 3: resonance by free decay, no hands |
| **Run** | The brain sends Run at the run amplitude for a few minutes, then finishes. Wrist homes, starts the model swinging if it is still, locks to the sway and drives in phase with its velocity, learning the drive's phase as it goes. | 2 and 3: the structure swings controllably at resonance, found and held by Wrist |

Artist sessions and commissioning use QLC+'s takeover from any state, as on every show; the
Director does not know and does not need a state for it. Mode and Amplitude on faders are how
Wrist is driven at any amplitude, and in RampDown, which the programme never sends.

### 2.3 The lamps

| situation | left lamp | right lamp |
|---|---|---|
| armed, experiment *k* of 2 | *k* flashes of 100 ms at full, then 1.5 s dark, repeating | off |
| Pluck | steady 100% | off |
| Run | the driven sine | the measured sine |
| finished | two 1 s fades up and down, then the armed pattern | off |
| aborted | three 100 ms flashes, then the armed pattern | off |

**In Run, the two lamps plot the two waveforms side by side.** The left lamp pulses with the
driven sine, (1 + sin(φ + φ_off))/2; the right with the measured one, the sway on Wrist's
accelerometer scaled to its amplitude, (1 + sway/a)/2. Side by side they show the drive leading
the sway. Both come from Wrist's OSC to the brain: the driven phase from its state bundle,
advanced at the oscillator's frequency between bundles so the lamp moves smoothly at the frame
rate; the measured from its waveform bundle as it arrives. The brain writes both into the
face's channels like any other look. While Wrist's OSC is stale, both lamps are at steady
100%.

### 2.4 The e-stop, signal loss, and what the drive does

The e-stop is the rig's show-stop, as the README has it, and the show runs on underneath it.
What is specific to this show is what Wrist does — set by this show's `.qxf` and Wrist's
firmware, in `CONTROL_LOOP.md`, used here:

- **Pressed:** the mask puts Wrist's Mode in its no-function band, **Blackout**. Wrist cannot
  tell that from a Blackout the show asks for: if it was driving, it brakes the sway under
  power, for at most 30 s, then releases its coils, as if the motor's power had been removed.
- **Released:** the live frame returns. Position was lost while the coils were released, so
  Wrist homes, then takes up whatever the frame asks, a Run included. Nothing in the fixture
  stops motion resuming on its own. The show runs on under the stop and reads the buttons, so
  an experiment aborted with the right button while the stop is pressed makes the release a
  Blackout, and nothing moves.
- **Universe lost:** no frame for 2.5 s, or the brain ending its stream as it stops, and Wrist
  releases its coils at once, with no brake.
- **Boot with the stop pressed:** nothing goes out, Wrist stays in Blackout, as every fixture
  does.

## 3. The fixtures

The workspace patches two fixtures: `Face` as the code has it, and `Wrist`.

### 3.1 Wrist

The sway drive. What it is and why is `DESIGN.md` §3.10; its hardware, firmware, pins and
constants are `CONTROL_LOOP.md`'s; its parts are in `OWNED-DEVICES.md`. The brain sets only
its mode and amplitude, and reads its OSC for the face. How this show sets it up:

**Patch.** `fixtures/wrist.qxf`, mode "2 Channel", at the top of the universe, address 500,
named `Wrist [10.0.10.3 <its MAC>]`.

| ch | name | group | this show's use |
|---|---|---|---|
| 1 | Mode | Maintenance; no-function band **Blackout** (0–15) | Blackout 0–15 · RampDown 16–63 · Run 64–191 · Pluck 192–255 |
| 2 | Amplitude | Nothing | 0–255 → 0 … `A_max` steps: in Run the drive's amplitude, in Pluck the size of the tug |

**Motor.** The closed-loop stepper; its label, connector and limits are in
`OWNED-DEVICES.md`. Its DIP switches for this show:

| switch | setting | why |
|---|---|---|
| SW1–SW4 | the 800 row of the table on the body | 800 pulses per revolution: the model's fastest motion, about 8 rev/s, is 6.4 kHz of pulses, inside the 12.5 kHz the firmware can make; 1600 would not be |
| SW5 | as set at commissioning | the motor's direction (off counter-clockwise, on clockwise), set so positive reels the wire in |
| SW6 | off, always | "internal debugging, do not switch on" |

**Supply.** The motor supply at **38 V** (`OWNED-DEVICES.md` says why it runs there), through
a 5 A fuse, with 4700 µF / 50 V across V+ and V− at the motor, fixed down rather than
hanging from its leads, and both pins of each power pair connected. It is on order.
If the e-stop button has a spare contact block, a relay on the motor's
feed from that contact, its contacts rated for at least 50 V DC, makes "limp when pressed" a
hardware fact as well as a DMX one, at the cost of the brake; without one, Blackout over DMX is what the PoC has, and the full-scale rig
gets the relay regardless.

**Board.** The NodeMCU on 5 V from the cabinet's USB charger, wired as `CONTROL_LOOP.md` §2
has it, ENA through an S8050. Powered and running no code, the board leaves the motor free;
unpowered, it leaves the driver enabled and the shaft held under power (both tested
2026-11-07), so the motor's supply goes off before the board's.

**Wire and home.** As `CONTROL_LOOP.md` §2 has it, the wire running to the chosen node. The
rest position, the travel and the tension limits for the band set are measured at
commissioning and coded into the firmware. The motor on its load cell sits on a wooden bridge
spanning the model's base, clamped to the table; nothing is fixed to the sculpture but the
wire and the accelerometer.

### 3.2 The accelerometer

An ADXL345 breakout Velcroed to the model, on a cable to Wrist's board run apart from the
motor's. Wrist picks the sway axis at each Pluck, so the breakout's orientation does not
matter. The three placements tried in E0 are the top node, a mid-height node, and a strut.

### 3.3 Memory and the dashboard

`memory-pi` as the README has it, with one provisioned dashboard for this show,
`etc-memory-pi/grafana/headless-hug-poc.json`: Wrist's state and waveform bundles, its
transitions, pluck results and protections as annotations, and `brain/show` and `brain/run` as
annotations. Wrist speaks OSC only, no MQTT: it sends every bundle twice, to the brain for the
lamps and to `memory-pi`, whose OSC bridge ingests it alongside MQTT, so the trace runs on
while the brain is down. The Mac is on the access point for the sessions
and reaches Grafana over the SSH jump as today; QLC+'s output for this show unicasts to
`10.0.10.1` for the same reason, which the brain's receiver binds because the workspace says
so.

## 4. The experiments, in order

Each with what it needs working, what is read off the dashboard, and what decides it. The
numbers go in `POC-RESULTS.md`, by `brain/run`.

**E0 — Sensing, by hand.** *Needs:* Wrist's board and the accelerometer, its telemetry in
Memory, the dashboard. *Do:* with Wrist in Blackout, pluck the model a
centimetre by hand; three times per placement, three placements. *Read:* off the raw trace,
f₁ from ten periods counted and Q from the cycles to half amplitude; second-mode and
cross-axis content per placement. Also by hand: the static pull test, a luggage scale at the
top, grams per millimetre of deflection. *Decides:* whether the model has a usable first mode
(Q < 3 means a heavier top or a tighter net before anything else), the placement, and the
reference numbers for everything after.

**E1 — Wrist from a fader.** *Needs:* Wrist's motor side, the load cell calibrated, QLC+ on
the access point. *Do:* under takeover, Mode to Pluck at a small Amplitude and watch it home
and tug; Run, Amplitude up and down; RampDown, then Blackout; Run again and stop the brain,
whose stream ends, and watch the coils release at once (stopping QLC+ only hands the universe
back to the brain); e-stop while running and watch it brake, then release; release the stop
and watch it home and resume. *Read:* the motor's position against the wire, the load cell against
the bands, the transitions; the accelerometer's trace clean while the motor runs. *Decides:*
Wrist is a fixture.

**E2 — Three ways to pull.** *Needs:* E1, the artist. *Do:* under takeover, the wire at (a) the
top node, (b) a mid node, (c) a pair of guys from two anchors at 90°, one on Wrist and one
sprung; Run at three amplitudes. *Read:* sway amplitude per step of drive amplitude,
cross-axis amplitude, how it looks. *Decides:* the pull geometry for full scale, with the
artist.

**E3 — Pluck.** *Needs:* the Pluck experiment. *Do:* ten Plucks. *Read:* f₁, Q, the axis and
the clean peaks per run, and their spread against E0. *Decides:* question 3, the resonance
found; pass is ten within 2% of each other and of E0.

**E4 — Run.** *Needs:* E3. *Do:* Run. *Read:* time to lock; the drive phase Wrist acquires and
how it settles; the steady sway amplitude against Q times the static deflection the drive
amplitude makes; rise time, and brake time when the Run ends; the two lamps side by side.
*Decides:* questions 2 and 3, the resonance held and the amplitude followed.

**E5 — A long run.** *Needs:* E4. *Do:* under takeover, Run at one amplitude for an afternoon.
*Read:* how the drive phase and the oscillator's frequency drift and whether the adaptation
follows them without falling back to acquiring; the persisted values; every protection that
fires. *Decides:* whether the loop holds the resonance unattended, which is the full scale's
premise.

Throughout, without being an experiment: Wrist's control-tick timing and the age of its last
frame, with WiFi and the step pulses sharing one core. These decide nothing here; they are the
evidence for the full scale's choice of board.

## 5. Not in this show

A stiff line. Any amplitude bound or supervisor beyond Wrist's own protections. Wind and
weather. A sensor apart from Wrist, and Bluetooth anywhere. The ESP-based brain, RDM, the
dials, the e-paper. Sound and vision.
