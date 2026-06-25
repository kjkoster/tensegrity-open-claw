# Ponytail

Fibre-optic light fixture retrofitted with an ESP32-S3 (Seeed Studio XIAO). The MCU
joins the Pi's WiFi, subscribes to its sACN universe, and drives the fixture. Per-board
configuration (DMX address, BLE target) is keyed by the WiFi station MAC; see
`src/config.rs`.

Two personalities share the same sACN front end (`sacn.rs` → the `DMX_VALUE` signal):

- **PWM** (`led_fixture.rs`) — drives the RGBW LED array directly over LEDC PWM. The
  current, known-good build.
- **BLE bridge** (`ble.rs`) — keeps the fixture's original Telink controller and
  bridges DMX → BLE write commands, the only path that reaches the gobo motor. Dormant
  until WiFi + BLE coexistence is proven on the XIAO and the fixture's BLE MAC / GATT UUID
  are captured.

### Interlocked white

The Telink gobo fixture is *modal*: its hardware cannot light the RGB emitters and the
white LED at the same time. RGB and white are mutually exclusive modes of the same
underlying command, so ponytail exposes them as an **interlocked RGBW**:

- The **White** channel takes precedence. While White > 0 the fixture is in white mode and
  the Red/Green/Blue channels are ignored. Drop White to 0 to return to RGB.
- Color ↔ white is a **hard cut**, not a crossfade — the fixture snaps between modes. A cue
  that needs a smooth color-to-white transition must pass through black or fake white in
  RGB.
- The **Dimmer** (Intensity) is applied in software (it scales the values sent), so it works
  identically in both modes. Dimmer at 0 powers the LED off entirely.
- The **gobo** axis is a single rotation channel (0 = motor off, 1–255 → speed). The fixture
  ignores gobo *selection*, so there is no select channel. Powering the LED off also stops
  the gobo motor.

### Manual override from QLC+ (sACN priority)

The sACN decoder (`sacn.rs`) does E1.31 source arbitration, so a lighting console such as
QLC+ can take live manual control of the fixture without stopping or coordinating with the
brain. The brain and the console send the same universe; the fixture obeys the
**highest-priority live source** and falls back automatically when it goes quiet.

- The brain sends at the default **priority 100**. Configure QLC+ to send the same universe
  at a **higher priority (e.g. 200)** and it takes over within a frame; the brain is ignored
  while the console is live. The project ships a preconfigured QLC+ workspace,
  [`open-claw.qxw`](../open-claw.qxw), with the universe, output, and priority already set up.
- Sources are tracked per **CID** (the sender's 16-byte ID). A source is dropped after the
  E1.31 **2.5 s network-data-loss timeout**, or immediately if it sets the **stream-terminated**
  flag on a clean stop — so control reverts to the brain when the console stops sending or
  drops off WiFi.
- Because the arbitration runs in the fixture, the override works **even if the brain has hung
  or crashed** — which is exactly when manual control is most wanted.

This is independent of the existing 5 s socket-rebind timeout, which is the "nobody is talking
at all" safety net.
