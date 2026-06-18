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
