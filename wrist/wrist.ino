// Wrist's firmware, so far: the board on the WiFi, every OSC address in wrist.oscquery.json sent
// the way the file says, and the motor in Blackout or Manual as the brain's sACN frame sets,
// per CONTROL_LOOP.md. The accelerometer, the other modes and the protections come later.
//
// The network, the patch and the OSC encoders are in generated.h, which build.sh writes from
// clockwork.dot, clockwork.qxw, wrist.qxf and wrist.oscquery.json before every build.

#include <ESP8266WiFi.h>
#include <WiFiUdp.h>
#include <math.h>

#include "generated.h"

// The optos share a common anode on 3.3 V, so a pin driven low lights its opto.
const uint8_t PIN_PUL = 14;     // D5
const uint8_t PIN_DIR = 12;     // D6
// ENA drives an NPN that lights the EN opto, which disables the driver. Released, the pull-up
// turns the NPN on, so the motor is limp through boot, reset and crashes; driven low, enabled.
const uint8_t PIN_ENA = 0;      // D3
// The driver's alarm output rests open; an alarm pulls RX low.
const uint8_t PIN_ALM = 3;      // RX
const uint8_t PIN_HX_DT = 16;   // D0
// SCK must idle low: GPIO15 low is a boot condition, and held high it powers the HX711 down.
const uint8_t PIN_HX_SCK = 15;  // D8
const uint8_t PIN_LED = 2;      // D4, lit when low

// The load cell's calibration, CONTROL_LOOP.md §7. Until it is measured the scale is NaN, so
// /wrist/load/force reads NaN rather than a tension nobody measured.
const int32_t LOAD_TARE = 0;            // raw counts
const float LOAD_SCALE = NAN;           // N per count

// Manual's motion, CONTROL_LOOP.md §10. First guesses, to be set to what the motor does: the
// bench ran 200,000 steps/s² to 12,000 steps/s with the shaft free and no alarm, so these sit
// well inside it. At 800 steps a revolution, 6000 steps/s is 7.5 revolutions a second, about
// 190 mm/s of wire on an 8 mm shaft.
const float V_MAX = 6000.0f;            // steps/s, Speed at the top of its band
const float A_MANUAL = 20000.0f;        // steps/s², every change of speed
// The DIR level that turns the shaft clockwise, facing it, with SW5 as set. By test.
const uint8_t DIR_CW = HIGH;
// The driver wants enable settled this long before the first pulse.
const uint32_t ENABLE_SETTLE_MS = 100;

// E1.31's network-data-loss timeout: a universe silent this long is lost.
const uint32_t SACN_TIMEOUT_MS = 2500;

// 1 Hz: half a second lit, half dark.
const uint32_t BLINK_HALF_PERIOD_MS = 500;

// Room for the largest bundle with a margin; an encoder that would overrun it sends nothing.
uint8_t packet[512];

WiFiUDP udp;
WiFiUDP sacnUdp;

// The latest load cell reading, and whether there has been one.
int32_t loadRaw = 0;
bool loadRead = false;

// The longest gap between two passes of loop() since the last state bundle, in µs.
uint32_t longestTick = 0;

// ── The step generator ───────────────────────────────────────────────────────────────────────
//
// Timer1 as an NMI at 25 kHz, so neither the WiFi nor the HX711's masked readout can delay a
// step. A phase accumulator advances by `increment` every tick; each carry starts a step pulse,
// PUL low, and the next tick ends it. The rate is capped at half the tick rate, so a carry never
// falls on the tick straight after one: every pulse is one tick low and at least one tick high,
// 40 µs each, well over the 2.5 µs the driver asks for.
//
// loop() writes DIR itself, only while `increment` is 0 and no pulse is low, and waits 10 µs
// before it sets `increment`: the accumulator keeps its phase through a stop, so the first carry
// can come on the very next tick, and DIR must be steady 5 µs before it.
namespace stepper {

constexpr uint32_t TICK_HZ = 25000;
constexpr float MAX_RATE = TICK_HZ / 2;   // steps/s
// One step a second as an increment: a carry is 2^32.
constexpr float PER_STEP_PER_S = 4294967296.0f / TICK_HZ;

volatile uint32_t increment = 0;
volatile bool pulseLow = false;
uint32_t phase = 0;

void IRAM_ATTR tick() {
  if (pulseLow) {
    GPOS = 1 << PIN_PUL;
    pulseLow = false;
  }
  const uint32_t before = phase;
  phase += increment;
  if (phase < before) {
    GPOC = 1 << PIN_PUL;
    pulseLow = true;
  }
}

void start() {
  timer1_disable();
  ETS_FRC_TIMER1_INTR_ATTACH(NULL, NULL);
  ETS_FRC_TIMER1_NMI_INTR_ATTACH(tick);
  timer1_enable(TIM_DIV1, TIM_EDGE, TIM_LOOP);
  timer1_write(80000000 / TICK_HZ);
}

// `rate` in steps/s, not negative; the direction is DIR's.
void setRate(float rate) {
  increment = static_cast<uint32_t>(min(rate, MAX_RATE) * PER_STEP_PER_S);
}

// Only while stopped: waits out a pulse still low, so the step it ends goes the way it began.
void setDirection(bool clockwise) {
  while (pulseLow) {
  }
  digitalWrite(PIN_DIR, clockwise ? DIR_CW : !DIR_CW);
  delayMicroseconds(10);
}

}  // namespace stepper

// ── The motor ────────────────────────────────────────────────────────────────────────────────

enum class Mode { blackout, manual };
Mode mode = Mode::blackout;
uint32_t enabledAt = 0;
// The shaft's speed now, in steps/s, positive clockwise.
float speed = 0;

void enableMotor() {
  digitalWrite(PIN_ENA, LOW);
  pinMode(PIN_ENA, OUTPUT);
}

// Stops the pulses and frees the shaft at once, whatever it was doing.
void releaseMotor() {
  stepper::setRate(0);
  speed = 0;
  pinMode(PIN_ENA, INPUT);
}

void toBlackout(const char *why) {
  releaseMotor();
  if (mode != Mode::blackout) {
    Serial.printf("wrist: Blackout, released: %s\n", why);
  }
  mode = Mode::blackout;
}

// ── sACN ─────────────────────────────────────────────────────────────────────────────────────

// The last levels of Wrist's channels, and when they arrived.
uint8_t levels[patch::CHANNELS];
bool haveFrame = false;
uint32_t frameAt = 0;

// Room for a full universe: 126 bytes of headers and 512 slots.
uint8_t sacnBuffer[638];

uint16_t be16(const uint8_t *p) { return (p[0] << 8) | p[1]; }
uint32_t be32(const uint8_t *p) { return (static_cast<uint32_t>(be16(p)) << 16) | be16(p + 2); }

// Takes one E1.31 data packet. Anything that is not a data packet for this universe, with DMX's
// start code, is ignored: it says nothing about Wrist's channels.
void takeSacn(const uint8_t *p, size_t len) {
  static const uint8_t ACN_ID[12] = {'A', 'S', 'C', '-', 'E', '1', '.', '1', '7', 0, 0, 0};
  static bool shortWarned = false;
  if (len < 126 || be16(p) != 0x0010 || memcmp(p + 4, ACN_ID, sizeof ACN_ID) != 0 ||
      be32(p + 18) != 0x00000004 || be32(p + 40) != 0x00000002 || p[117] != 0x02 ||
      be16(p + 113) != patch::SACN_UNIVERSE) {
    return;
  }
  const uint8_t options = p[112];
  // Preview data is for a console's visualiser, never for a fixture.
  if (options & 0x80) {
    return;
  }
  // The sender is going away: stop now rather than wait out the timeout.
  if (options & 0x40) {
    haveFrame = false;
    toBlackout("sACN stream terminated");
    return;
  }
  if (p[125] != 0x00) {
    return;
  }
  const size_t slots = be16(p + 123) - 1;
  const size_t first = patch::DMX_ADDRESS - 1;
  if (slots < first + patch::CHANNELS || len < 126 + first + patch::CHANNELS) {
    if (!shortWarned) {
      Serial.printf("wrist: an sACN frame of %u slots stops short of Wrist's, %u–%u\n",
                    static_cast<unsigned>(slots), patch::DMX_ADDRESS,
                    patch::DMX_ADDRESS + patch::CHANNELS - 1);
      shortWarned = true;
    }
    return;
  }
  memcpy(levels, p + 126 + first, patch::CHANNELS);
  if (!haveFrame) {
    Serial.println("wrist: sACN frames arriving");
  }
  haveFrame = true;
  frameAt = millis();
}

void receiveSacn() {
  while (sacnUdp.parsePacket() > 0) {
    const int len = sacnUdp.read(sacnBuffer, sizeof sacnBuffer);
    if (len > 0) {
      takeSacn(sacnBuffer, len);
    }
  }
  if (haveFrame && millis() - frameAt > SACN_TIMEOUT_MS) {
    haveFrame = false;
    toBlackout("sACN lost");
  }
}

// The speed the frame asks for, in steps/s, positive clockwise. Speed's lowest level in its
// moving band is the slowest step rate, its highest V_MAX.
float askedSpeed() {
  const uint8_t level = levels[patch::speed::OFFSET];
  if (!patch::speed::SLOW_TO_THE_MAXIMUM.holds(level)) {
    return 0;
  }
  const patch::Band band = patch::speed::SLOW_TO_THE_MAXIMUM;
  const float rate = V_MAX * (level - band.min + 1) / (band.max - band.min + 1);
  return patch::direction::CLOCKWISE.holds(levels[patch::direction::OFFSET]) ? rate : -rate;
}

// Once per pass of loop(), `dt` in seconds since the last.
void updateMotor(float dt) {
  const bool manualAsked = haveFrame && patch::mode::MANUAL.holds(levels[patch::mode::OFFSET]);

  if (mode == Mode::blackout) {
    // Manual is entered from Blackout only, and only on a live frame.
    if (!manualAsked) {
      return;
    }
    enableMotor();
    enabledAt = millis();
    speed = 0;
    mode = Mode::manual;
    Serial.println("wrist: Manual, enabled");
    return;
  }

  // Manual. Asked to leave, it ramps to a stop and releases into Blackout once stopped.
  float target = manualAsked ? askedSpeed() : 0;
  if (millis() - enabledAt < ENABLE_SETTLE_MS) {
    target = 0;
  }
  // A change of direction ramps to a stop first, then sets DIR, then ramps up the other way.
  if (speed != 0 && (target > 0) != (speed > 0)) {
    target = 0;
  }
  const float most = A_MANUAL * dt;
  const bool startingFromRest = speed == 0;
  speed = target > speed ? min(target, speed + most) : max(target, speed - most);
  if (startingFromRest && speed != 0) {
    stepper::setDirection(speed > 0);
  }
  stepper::setRate(fabsf(speed));

  if (!manualAsked && speed == 0) {
    toBlackout("Mode left Manual");
  }
}

// ── OSC ──────────────────────────────────────────────────────────────────────────────────────

void sendToRecipients(size_t length) {
  if (length == 0) {
    Serial.println("wrist: an OSC packet did not fit its buffer, and was not sent");
    return;
  }
  for (const net::Recipient &r : net::OSC_RECIPIENTS) {
    udp.beginPacket(IPAddress(r.address[0], r.address[1], r.address[2], r.address[3]), r.port);
    udp.write(packet, length);
    udp.endPacket();
  }
}

// The ESP8266's reset reasons, REASON_DEFAULT_RST to REASON_EXT_SYS_RST, are 0 to 6 in the
// order wrist.oscquery.json lists their names.
osc::SysWristResetReason resetReason() {
  const uint32_t reason = ESP.getResetInfoPtr()->reason;
  return reason <= static_cast<uint32_t>(osc::SysWristResetReason::external_reset)
             ? static_cast<osc::SysWristResetReason>(reason)
             : osc::SysWristResetReason::power_on;
}

void sendOnConnect() {
  const String mac = WiFi.macAddress();
  const String ip = WiFi.localIP().toString();
  osc::OnConnect v;
  v.wrist_load_tare = LOAD_TARE;
  v.wrist_load_scale = LOAD_SCALE;
  v.dmx_wrist_universe = patch::SACN_UNIVERSE;
  v.dmx_wrist_address = patch::DMX_ADDRESS;
  v.dmx_wrist_port = net::SACN_PORT;
  v.sys_wrist_mac = mac.c_str();
  v.sys_wrist_ip = ip.c_str();
  v.sys_wrist_reset_reason = resetReason();
  v.sys_wrist_flash_size = static_cast<int32_t>(ESP.getFlashChipRealSize());
  v.sys_wrist_cpu_freq = ESP.getCpuFreqMHz();
  sendToRecipients(osc::encodeOnConnect(v, packet, sizeof packet));
  Serial.printf("wrist: connected as %s, %s\n", ip.c_str(), mac.c_str());
}

void sendState() {
  osc::At10hz v;
  v.wrist_mode = mode == Mode::manual ? osc::WristMode::manual : osc::WristMode::blackout;
  v.wrist_alarm = digitalRead(PIN_ALM) == LOW;
  v.wrist_load_force = loadRead ? (loadRaw - LOAD_TARE) * LOAD_SCALE : NAN;
  v.wrist_health_tick_dt = static_cast<int32_t>(longestTick);
  // No frame yet is as old as a frame can be.
  v.dmx_wrist_sacn_age = frameAt == 0 ? INT32_MAX : static_cast<int32_t>(millis() - frameAt);
  sendToRecipients(osc::encodeAt10hz(v, packet, sizeof packet));
  longestTick = 0;
}

// ── The load cell ────────────────────────────────────────────────────────────────────────────

// Returns false while the HX711 has no new reading.
bool readLoadCell(int32_t &value) {
  if (digitalRead(PIN_HX_DT) == HIGH) {
    return false;
  }
  uint32_t raw = 0;
  // SCK held high for more than 60 µs powers the chip down, so no interrupt may stretch a clock.
  // The step generator is an NMI, which this does not mask.
  noInterrupts();
  for (int i = 0; i < 24; i++) {
    digitalWrite(PIN_HX_SCK, HIGH);
    delayMicroseconds(1);
    raw = (raw << 1) | digitalRead(PIN_HX_DT);
    digitalWrite(PIN_HX_SCK, LOW);
    delayMicroseconds(1);
  }
  // A 25th clock selects channel A at gain 128 for the next reading.
  digitalWrite(PIN_HX_SCK, HIGH);
  delayMicroseconds(1);
  digitalWrite(PIN_HX_SCK, LOW);
  interrupts();
  // The reading is 24-bit two's complement.
  value = static_cast<int32_t>(raw << 8) >> 8;
  return true;
}

// ── Setup and the loop ───────────────────────────────────────────────────────────────────────

void setup() {
  // Transmit only: RX is the alarm input. At 74880, the boot ROM's own rate, so its reset cause
  // and any crash dump read in the same log as the firmware's lines.
  Serial.begin(74880, SERIAL_8N1, SERIAL_TX_ONLY);

  // Idle levels are written before the pins become outputs, so no opto flashes on at start.
  digitalWrite(PIN_PUL, HIGH);
  pinMode(PIN_PUL, OUTPUT);
  digitalWrite(PIN_DIR, DIR_CW);
  pinMode(PIN_DIR, OUTPUT);
  pinMode(PIN_ENA, INPUT);
  digitalWrite(PIN_LED, HIGH);
  pinMode(PIN_LED, OUTPUT);
  pinMode(PIN_ALM, INPUT_PULLUP);
  pinMode(PIN_HX_DT, INPUT);
  digitalWrite(PIN_HX_SCK, LOW);
  pinMode(PIN_HX_SCK, OUTPUT);

  stepper::start();

  // The address is fixed, from clockwork.dot, and modem sleep is off, so a packet to Wrist is
  // never held for a beacon interval. Not persisted: the settings come from the build, not flash.
  WiFi.persistent(false);
  WiFi.mode(WIFI_STA);
  WiFi.setSleepMode(WIFI_NONE_SLEEP);
  WiFi.setAutoReconnect(true);
  WiFi.config(IPAddress(net::ADDRESS), IPAddress(net::GATEWAY), IPAddress(net::NETMASK));
  WiFi.begin(net::WIFI_SSID, net::WIFI_PSK, net::WIFI_CHANNEL);
  sacnUdp.begin(net::SACN_PORT);

  Serial.println();
  Serial.printf("wrist: start, joining \"%s\" as %u.%u.%u.%u, sACN universe %u on port %u, "
                "DMX %u–%u\n",
                net::WIFI_SSID, net::ADDRESS[0], net::ADDRESS[1], net::ADDRESS[2],
                net::ADDRESS[3], patch::SACN_UNIVERSE, net::SACN_PORT, patch::DMX_ADDRESS,
                patch::DMX_ADDRESS + patch::CHANNELS - 1);
}

void loop() {
  static uint32_t lastTick = micros();
  static bool connected = false;
  static uint32_t stateDue = 0;

  const uint32_t now = micros();
  const uint32_t tick = now - lastTick;
  lastTick = now;
  if (tick > longestTick) {
    longestTick = tick;
  }

  // Lit while the motor is enabled, blinking at 1 Hz while it is released.
  digitalWrite(PIN_LED, mode == Mode::manual || (millis() / BLINK_HALF_PERIOD_MS) % 2 == 0
                            ? LOW
                            : HIGH);

  // The motor first, and whether or not the WiFi is up: with the WiFi gone the frames stop,
  // and the timeout is what releases the shaft. A pass held up by the WiFi stack ramps no
  // faster for it: dt is capped, so a late pass only accelerates less.
  receiveSacn();
  updateMotor(min(tick, 20000u) * 1e-6f);

  // Read whether or not the WiFi is up, so the force is current the moment it is.
  int32_t raw;
  const bool fresh = readLoadCell(raw);
  if (fresh) {
    loadRaw = raw;
    loadRead = true;
  }

  const bool up = WiFi.status() == WL_CONNECTED;
  if (up && !connected) {
    sendOnConnect();
    stateDue = now;
  }
  if (!up && connected) {
    Serial.println("wrist: WiFi lost");
  }
  connected = up;
  if (!connected) {
    return;
  }

  if (fresh) {
    sendToRecipients(osc::encodeWristLoadRaw(raw, packet, sizeof packet));
  }
  // Due times step by the period, so the rate does not drift with the time a pass takes.
  if (static_cast<int32_t>(now - stateDue) >= 0) {
    sendState();
    stateDue += osc::AT_10HZ_PERIOD_US;
    // After a long stall, start again from now rather than send the missed bundles in a burst.
    if (static_cast<int32_t>(now - stateDue) >= 0) {
      stateDue = now + osc::AT_10HZ_PERIOD_US;
    }
  }
}
