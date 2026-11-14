# Headless Hug — Proof of Concept

**What this file is for.** The show file for the proof of concept: what the rig does, why, the
fixtures it uses and how each is set up. The audience is us. The show is a set of experiments
the rig runs on a one-metre model of the *Headless Hug* sculpture, stepped through from the
cabinet door and watched live in Grafana on `memory-pi`. Its results are recorded by run, as
§4 says. How to get from here to a running PoC is in
[`TODO.md`](TODO.md). The component it introduces, Wrist, is described in
[`DESIGN.md`](DESIGN.md), and its firmware is `wrist/`; this file says how this show builds and
uses it.

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
lets the drive go limp.

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

- At boot the Director is in **armed Pluck**, Wrist in Blackout until the first frame.
- Armed, the brain sends Wrist's Mode as Hold: coils enabled, the shaft locked where it is.
  Ending or aborting an experiment is that Hold. Blackout is a Mode only: Amplitude is a size,
  and 0 means no size, not Blackout.
- The Director has no blackout state of its own. Wrist is otherwise in Blackout only when the
  e-stop's mask or a lost universe puts it there (§2.4).
- **Running** an experiment returns to the same experiment armed when it finishes or is
  aborted, so the right button repeats it. The left button is ignored while one runs.
- Every state is published as `brain/show`, retained, as the code already does
  (`armed-pluck`, `pluck`, `armed-run`, `run`). Each start increments `brain/run`,
  retained — a run counter, not a second state topic — so a Grafana annotation and the
  numbers recorded for a run name the same run.

The experiments, each with what it is for. Their amplitudes and durations are named constants
in `brain/src/show.rs`, with units.

| experiment | what the rig does | what it answers |
|---|---|---|
| **Pluck** | Started by the right button only while Wrist's `/wrist/pluck` says `can`. The brain sends Pluck at the pluck amplitude, direction and speed, until `/wrist/pluck` leaves `can`, and on while it is `busy`; a press `/wrist/pluck` still answers `can` to after a second is dropped, because its Pluck never reached Wrist (a desk holding the universe), and the right button lights again: Wrist tugs once, dwells and returns slowly to where it began. When it says `cannot`, the brain sends Hold and the Pluck has finished; the ring-down is in the accelerometer's trace. It cannot be aborted: the right button is dark while it runs. | 3: resonance by free decay, no hands |
| **Run** | The brain sends Run at the run amplitude for a few minutes, then finishes. Wrist homes, starts the model swinging if it is still, locks to the sway and drives in phase with its velocity, learning the drive's phase as it goes. | 2 and 3: the structure swings controllably at resonance, found and held by Wrist |

Artist sessions and commissioning use QLC+'s takeover from any state, as on every show; the
Director does not know and does not need a state for it. Wrist's channels on faders are how
it is driven at any amplitude, and in Manual, which the programme never sends.

### 2.3 The lamps

| situation | left lamp | right lamp |
|---|---|---|
| armed, experiment *k* of 2 | *k* flashes of 100 ms at full, then 1.5 s dark, repeating | armed Pluck: green while `/wrist/pluck` is `can` and the button has not been pressed, dark while it is `busy` or `cannot` or stale; armed Run: off |
| Pluck | steady 100% | off |
| Run | the measured sine | the driven sine |
| finished | two 1 s fades up and down, then the armed pattern | off |
| aborted | three 100 ms flashes, then the armed pattern | off |

**In Run, the two lamps plot the two waveforms side by side.** The right lamp shows the
motor's phase, (1 + sin φ)/2, from `/wrist/run/phase` in each state bundle as it arrives,
whenever `/wrist/mode` is `run`. The left, once the accelerometer is in, the measured sway,
(1 + sway/a)/2, from the waveform bundle as it arrives. While Wrist's OSC is stale, both lamps
are at steady 100%.

### 2.4 The e-stop, signal loss, and what the drive does

The e-stop is the rig's show-stop, as the README has it, and the show runs on underneath it.
What is specific to this show is what Wrist does — set by this show's `.qxf` and Wrist's
firmware, used here:

- **Pressed:** the mask puts Wrist's Mode in its no-function band, **Blackout**. Wrist cannot
  tell that from a Blackout the show asks for: it releases its coils at once, as if the
  motor's power had been removed.
- **Released:** the live frame returns, and Wrist takes up whatever it asks. Nothing in the fixture
  stops motion resuming on its own. The show runs on under the stop and reads the buttons, so
  an experiment aborted with the right button while the stop is pressed makes the release a
  Blackout, and nothing moves.
- **Universe lost:** no frame for E1.31's timeout, or the brain ending its stream as it stops, and Wrist
  releases its coils at once, as for a Blackout in the frame.
- **Boot with the stop pressed:** nothing goes out, Wrist stays in Blackout, as every fixture
  does.

## 3. The fixtures

The workspace patches two fixtures: `Face` as the code has it, and `Wrist`.

### 3.1 Wrist

The sway drive. What it is and why is `DESIGN.md` §3.10; its firmware, with its pins and
constants, is `wrist/`; its parts are in `OWNED-DEVICES.md`. The brain sets its mode and what
the mode reads, and reads its OSC for the face. How this show sets it up:

**Patch.** `fixtures/wrist.qxf`, patched as `Wrist` in `clockwork.qxw`; its channels and bands
are the definition's, and its address on the access point is in `clockwork.dot`. Frequency is
16 bits because 8 would not do: a Q of 30 at 2 Hz is a resonance 0.07 Hz wide, finer than 8
bits step across the band.

**Motor.** The closed-loop stepper, set and supplied as `OWNED-DEVICES.md` has it. Its pulses
per revolution are set low, so the step generator's ceiling turns the shaft fast enough for a
sharp pluck.

**Supply.** If the e-stop button has a spare contact block, a relay on the motor's
feed from that contact, its contacts rated for at least 50 V DC, makes "limp when pressed" a
hardware fact as well as a DMX one; without one, Blackout over DMX is what the PoC has, and the full-scale rig
gets the relay regardless.

**Board.** Wrist's board, as `OWNED-DEVICES.md` has it, on the pins `wrist/wrist.ino` names.
The home switch goes on GPIO13 (D7), normally open to ground with the internal pull-up, and
the ADXL345 on GPIO5 and GPIO4 (D1 and D2, SCL and SDA), I²C at 400 kHz; the boot-strapping
pins, GPIO0, GPIO2 and GPIO15, take neither. The sensors run on the board's 3.3 V; the motor
has its own supply, with no ground tie: the optos are the only link. The ADXL345's cable runs
apart from the motor's.

**Wire.** The guy wire wrapped a few turns round the bare motor shaft, a capstan with its tail
tied off, then a short cable, a hook, rubber bands in parallel, a hook and the wire to the
chosen node. The shaft is taken to be 8 mm, 25 mm of wire a revolution; measure it. The bands
are the series spring that turns the shaft's position into force, and their preload keeps the
wire taut, so there is no backlash. Unpowered, the shaft holds only by its detent, taken to be
8–15 N of wire tension; measure it. A preload above that unwinds the wraps in Blackout and
lets the guy go slack, so either a backup guy, fixed and slightly longer, holds the structure
without the motor's, or the preload stays below the detent hold. The motor on its load cell
sits on a wooden bridge spanning the model's base, clamped to the table, the wire leaving the
shaft along the cell's axis so the cell reads its tension; nothing is fixed to the sculpture
but the wire and the accelerometer.

**Load cell.** Calibrated once per rig, in Blackout: unhook the wire and average
`/wrist/load/raw` from the recording, the tare; hang a known mass `m` and average again, so
the scale is `m·g/(raw − tare)` newtons per count. Both are coded into the firmware.

### 3.2 The accelerometer

The accelerometer (`OWNED-DEVICES.md`) Velcroed to the model, on a cable to Wrist's board run
apart from the motor's. Wrist picks the sway axis at each Pluck, so its orientation does not
matter. The three placements tried in E0 are the top node, a mid-height node, and a strut.

### 3.3 Memory and the dashboard

`memory-pi` as the README has it, with one provisioned dashboard for this show,
`pi/memory-pi/grafana/headless-hug-poc.json`: Wrist's state and waveform bundles, its
transitions, pluck results and protections as annotations, and `brain/show` and `brain/run` as
annotations. Wrist speaks OSC only, no MQTT: it sends every bundle twice, to the brain for the
lamps and to `memory-pi`, whose OSC bridge ingests it alongside MQTT, so the trace runs on
while the brain is down. The Mac is on the access point for the sessions
and reaches Grafana over the SSH jump as today; QLC+'s output unicasts to the brain over
WireGuard, as `clockwork.dot` has it.

## 4. The experiments, in order

Each with what it needs working, what is read off the dashboard, and what decides it. The
numbers are recorded by `brain/run`.

**E0 — Sensing, by hand.** *Needs:* Wrist's board and the accelerometer, its telemetry in
Memory, the dashboard. *Do:* with Wrist in Blackout, pluck the model a
centimetre by hand; three times per placement, three placements. *Read:* off the raw trace,
f₁ from ten periods counted and Q from the cycles to half amplitude; second-mode and
cross-axis content per placement. Also by hand: the static pull test, a luggage scale at the
top, grams per millimetre of deflection. *Decides:* whether the model has a usable first mode
(Q < 3 means a heavier top or a tighter net before anything else), the placement, and the
reference numbers for everything after.

**E1 — Wrist from a fader.** *Needs:* Wrist's motor side, the load cell calibrated, QLC+ on
the access point. *Do:* under takeover, Hold, then Pluck at a small Amplitude and watch it
tug and return; Run, Amplitude up and down, then Blackout; Run again and stop the brain,
whose stream ends, and watch the coils release at once (stopping QLC+ only hands the universe
back to the brain); e-stop while running and watch it release at once; release the stop
and watch it resume. *Read:* the motor's position against the wire, the load cell against
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
