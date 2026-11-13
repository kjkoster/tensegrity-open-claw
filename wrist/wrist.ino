// Wrist's firmware, so far: the board on the WiFi, every OSC address in wrist.oscquery.json sent
// the way the file says, and the motor in Blackout, Hold, Pluck or Manual as the brain's sACN
// frame sets, per CONTROL_LOOP.md. It follows the frame and decides nothing but Blackout on a
// lost universe. The accelerometer and Run come later.
//
// The network, the patch and the OSC encoders are in generated.h, which build.sh writes from
// clockwork.dot, clockwork.qxw, wrist.qxf and wrist.oscquery.json before every build.

#include <ESP8266WiFi.h>
#include <WiFiUdp.h>
#include <math.h>

#include "generated.h"
#include "log.h"

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

// The motion, CONTROL_LOOP.md §10. The driver is set to 400 steps a revolution, SW1–SW4; every
// figure here is in its steps. The top speed is the step generator's ceiling, half its 25 kHz
// tick: 31 revolutions a second, past the 15 where the motor's torque falls off, on trial. No
// ramp: every change of speed is taken at once. Plucks at full Amplitude and Speed, and Manual
// reversed at full Speed, ran on the wire without an alarm, 2026-11-13.
const float V_MAX = 12500.0f;           // steps/s, Speed at the top of its band

// Pluck, CONTROL_LOOP.md §4. The largest tug, Amplitude at full: four revolutions, about 100 mm
// of wire on the 8 mm shaft. The largest the band set takes is found by test.
const int32_t A_MAX = 1600;             // steps
const uint32_t PLUCK_DWELL_MS = 500;    // held at the tug's end
const float PLUCK_RETURN_S = 2.5f;      // back to where it began
// The DIR level that turns the shaft clockwise, facing it, with SW5 as set. By test, 2026-11-09.
const uint8_t DIR_CW = HIGH;
// The driver wants enable settled this long before the first pulse.
const uint32_t ENABLE_SETTLE_MS = 100;

// E1.31's network-data-loss timeout: a universe silent this long is lost.
const uint32_t SACN_TIMEOUT_MS = 2500;

// 1 Hz: half a second lit, half dark.
const uint32_t BLINK_HALF_PERIOD_MS = 500;

// Room for the largest bundle with a margin; an encoder that would overrun it sends nothing. The
// largest is the connect bundle, at most 580 bytes with its longest strings.
uint8_t packet[1024];

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
//
// Every step is counted into `position`, positive clockwise. A move to a position arms
// `stopAt`, and the tick makes no step once `position` is there, whatever `increment` says:
// a pass of loop() held up by the WiFi cannot carry the shaft past the end of a move.
namespace stepper {

constexpr uint32_t TICK_HZ = 25000;
constexpr float MAX_RATE = TICK_HZ / 2;   // steps/s
// One step a second as an increment: a carry is 2^32.
constexpr float PER_STEP_PER_S = 4294967296.0f / TICK_HZ;

volatile uint32_t increment = 0;
volatile bool pulseLow = false;
uint32_t phase = 0;
// Steps made, positive clockwise; read by loop(), written only here and by setDirection.
volatile int32_t position = 0;
volatile int8_t stepSign = 1;
volatile bool stopArmed = false;
volatile int32_t stopAt = 0;

void IRAM_ATTR tick() {
  if (pulseLow) {
    GPOS = 1 << PIN_PUL;
    pulseLow = false;
  }
  if (stopArmed && position == stopAt) {
    return;
  }
  const uint32_t before = phase;
  phase += increment;
  if (phase < before) {
    GPOC = 1 << PIN_PUL;
    pulseLow = true;
    position += stepSign;
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
  stepSign = clockwise ? 1 : -1;
  delayMicroseconds(10);
}

// Arms the stop for a move to `target`, or, with no move, disarms it.
void stopOn(int32_t target) {
  stopAt = target;
  stopArmed = true;
}

void noStop() { stopArmed = false; }

}  // namespace stepper

// ── The motor ────────────────────────────────────────────────────────────────────────────────

// The modes are /wrist/mode's values, as wrist.oscquery.json lists them, so the mode is sent as
// it is.
using Mode = osc::WristMode;
Mode mode = Mode::blackout;
uint32_t enabledAt = 0;
// The shaft's speed now, in steps/s, positive clockwise.
float speed = 0;

const char *modeName(Mode m) { return osc::WristModeNames[static_cast<uint8_t>(m)]; }

// One pluck, from the tick Pluck begins: Out to the tug's end, Dwell there, Back to where it
// began, then Rest there until Mode leaves Pluck.
enum class PluckPhase { out, dwell, back, rest };
PluckPhase pluckPhase = PluckPhase::rest;
// Read from the frame once, as Pluck begins.
int32_t pluckSteps = 0;
bool pluckClockwise = true;
float pluckSpeed = 0;           // steps/s
// Where the shaft stood when the tug began, and where the tug ends.
int32_t pluckStart = 0;
int32_t pluckEnd = 0;
uint32_t dwellFrom = 0;

void enableMotor() {
  digitalWrite(PIN_ENA, LOW);
  pinMode(PIN_ENA, OUTPUT);
}

// Stops the pulses and frees the shaft at once, whatever it was doing.
void releaseMotor() {
  stepper::setRate(0);
  stepper::noStop();
  speed = 0;
  pinMode(PIN_ENA, INPUT);
}

void toBlackout(const char *why) {
  releaseMotor();
  if (mode != Mode::blackout) {
    say("Blackout, released: %s", why);
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
      say("an sACN frame of %u slots stops short of Wrist's, %u–%u",
          static_cast<unsigned>(slots), patch::DMX_ADDRESS,
          patch::DMX_ADDRESS + patch::CHANNELS - 1);
      shortWarned = true;
    }
    return;
  }
  memcpy(levels, p + 126 + first, patch::CHANNELS);
  // Counted, not acted on: a frame that skips ahead is still the newest.
  const uint8_t sequence = p[111];
  if (lastSequence >= 0) {
    const uint8_t ahead = sequence - static_cast<uint8_t>(lastSequence);
    if (ahead > 1 && ahead < 128) {
      sacnSkipped += ahead - 1;
    }
  }
  lastSequence = sequence;
  sacnFrames++;
  packetsSinceFrame = 0;
  if (!haveFrame) {
    if (frameAt == 0) {
      say("sACN frames arriving");
    } else {
      say("sACN frames arriving again, after %lu ms without",
          static_cast<unsigned long>(millis() - frameAt));
    }
  }
  haveFrame = true;
  frameAt = millis();
}

void receiveSacn() {
  while (sacnUdp.parsePacket() > 0) {
    sacnPackets++;
    packetsSinceFrame++;
    const int len = sacnUdp.read(sacnBuffer, sizeof sacnBuffer);
    if (len > 0) {
      takeSacn(sacnBuffer, len);
    }
  }
  if (haveFrame && millis() - frameAt > SACN_TIMEOUT_MS) {
    haveFrame = false;
    lastSequence = -1;
    // Whether anything came in meanwhile, and the link as it is now, tell a sender that stopped
    // from packets that were not this universe's from a link that went.
    say("sACN lost: %lu packets since the last frame, WiFi %s, %d dBm",
        static_cast<unsigned long>(packetsSinceFrame),
        WiFi.status() == WL_CONNECTED ? "up" : "down", WiFi.RSSI());
    toBlackout("sACN lost");
  }
}

// The mode the frame asks for; with no live frame, Blackout.
Mode askedMode() {
  if (!haveFrame) {
    return Mode::blackout;
  }
  const uint8_t level = levels[patch::mode::OFFSET];
  if (patch::mode::HOLD.holds(level)) {
    return Mode::hold;
  }
  if (patch::mode::PLUCK.holds(level)) {
    return Mode::pluck;
  }
  if (patch::mode::MANUAL.holds(level)) {
    return Mode::manual;
  }
  return Mode::blackout;
}

// The step rate a Speed level means, in steps/s, not negative. Speed's lowest level in its
// moving band is the slowest step rate, its highest V_MAX.
float rateOf(uint8_t level) {
  if (!patch::speed::SLOW_TO_THE_MAXIMUM.holds(level)) {
    return 0;
  }
  const patch::Band band = patch::speed::SLOW_TO_THE_MAXIMUM;
  return V_MAX * (level - band.min + 1) / (band.max - band.min + 1);
}

bool clockwiseOf(uint8_t level) { return patch::direction::CLOCKWISE.holds(level); }

// The one motion stage: every mode asks for a speed, or for a position to stop on, and this sets
// it at once. `wanted` is in steps/s, positive clockwise. With `target`, the shaft is to come to
// rest there, at `wanted`'s speed until the step generator stops itself on it. Once per pass of
// loop().
void move(float wanted, const int32_t *target = nullptr) {
  if (target != nullptr) {
    stepper::stopOn(*target);
    const int32_t remaining = *target - stepper::position;
    wanted = remaining == 0 ? 0 : copysignf(fabsf(wanted), remaining);
  } else {
    stepper::noStop();
  }
  // The driver wants enable settled before the first pulse.
  if (millis() - enabledAt < ENABLE_SETTLE_MS) {
    wanted = 0;
  }
  // DIR is written only with the pulses stopped: a start, or a change of direction, stops them,
  // sets DIR, and goes on the other way in the same pass.
  if (wanted != 0 && (speed == 0 || (wanted > 0) != (speed > 0))) {
    stepper::setRate(0);
    stepper::setDirection(wanted > 0);
  }
  speed = wanted;
  stepper::setRate(fabsf(speed));
}

// Pluck's channels are read once, here, and ignored for the rest of the pluck.
void beginPluck() {
  pluckSteps = lroundf(A_MAX * levels[patch::amplitude::OFFSET] / 255.0f);
  pluckClockwise = clockwiseOf(levels[patch::direction::OFFSET]);
  pluckSpeed = rateOf(levels[patch::speed::OFFSET]);
  // Turning, the shaft stops here, and the tug starts from where it stands.
  move(0);
  if (pluckSteps == 0 || pluckSpeed == 0) {
    pluckPhase = PluckPhase::rest;
    say("Pluck, no tug");
    return;
  }
  pluckStart = stepper::position;
  pluckEnd = pluckStart + (pluckClockwise ? pluckSteps : -pluckSteps);
  pluckPhase = PluckPhase::out;
  say("Pluck, %ld steps %s at %.0f steps/s", static_cast<long>(pluckSteps),
      pluckClockwise ? "clockwise" : "counter-clockwise", pluckSpeed);
}

void stepPluck() {
  switch (pluckPhase) {
    case PluckPhase::out:
      move(pluckSpeed, &pluckEnd);
      if (speed == 0 && stepper::position == pluckEnd) {
        dwellFrom = millis();
        pluckPhase = PluckPhase::dwell;
      }
      break;
    case PluckPhase::dwell:
      move(0);
      if (millis() - dwellFrom >= PLUCK_DWELL_MS) {
        pluckPhase = PluckPhase::back;
      }
      break;
    case PluckPhase::back:
      move(pluckSteps / PLUCK_RETURN_S, &pluckStart);
      if (speed == 0 && stepper::position == pluckStart) {
        pluckPhase = PluckPhase::rest;
        say("Pluck, back where it began");
      }
      break;
    case PluckPhase::rest:
      move(0);
      break;
  }
}

// Whether a pluck can be started, for /wrist/pluck.
osc::WristPluck pluckState() {
  if (mode == Mode::hold) {
    return osc::WristPluck::can;
  }
  if (mode == Mode::pluck && pluckPhase != PluckPhase::rest) {
    return osc::WristPluck::busy;
  }
  return osc::WristPluck::cannot;
}

// Once per pass of loop(). The frame's mode is the mode, from any mode to any other, taking over
// from wherever the shaft is.
void updateMotor() {
  const Mode asked = askedMode();
  if (asked == Mode::blackout) {
    if (mode != Mode::blackout) {
      toBlackout(haveFrame ? "Mode is Blackout" : "no live frame");
    }
    return;
  }
  if (asked != mode) {
    if (mode == Mode::blackout) {
      enableMotor();
      enabledAt = millis();
    }
    say("%s → %s", modeName(mode), modeName(asked));
    mode = asked;
    if (mode == Mode::pluck) {
      beginPluck();
    }
  }
  switch (mode) {
    case Mode::hold:
      move(0);
      break;
    case Mode::pluck:
      stepPluck();
      break;
    case Mode::manual: {
      const float rate = rateOf(levels[patch::speed::OFFSET]);
      move(clockwiseOf(levels[patch::direction::OFFSET]) ? rate : -rate);
      break;
    }
    default:
      break;
  }
}

// ── OSC ──────────────────────────────────────────────────────────────────────────────────────

void sendToRecipients(size_t length) {
  if (length == 0) {
    say("an OSC packet did not fit its buffer, and was not sent");
    return;
  }
  for (size_t i = 0; i < RECIPIENTS; i++) {
    const net::Recipient &r = net::OSC_RECIPIENTS[i];
    const bool begun =
        udp.beginPacket(IPAddress(r.address[0], r.address[1], r.address[2], r.address[3]), r.port);
    bool taken = false;
    if (begun) {
      udp.write(packet, length);
      taken = udp.endPacket();
    }
    noteOscSend(i, taken);
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
  const String bssid = WiFi.BSSIDstr();
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
  v.sys_wrist_wifi_channel = WiFi.channel();
  v.sys_wrist_wifi_bssid = bssid.c_str();
  v.sys_wrist_wifi_disconnects = static_cast<int32_t>(disconnects);
  v.sys_wrist_wifi_disconnect_reason = lastDisconnect;
  sendToRecipients(osc::encodeOnConnect(v, packet, sizeof packet));
  say("connected as %s, %s, %d dBm", ip.c_str(), mac.c_str(), WiFi.RSSI());
}

void sendState() {
  osc::At10hz v;
  v.wrist_mode = mode;
  v.wrist_pluck = pluckState();
  v.wrist_alarm = digitalRead(PIN_ALM) == LOW;
  v.wrist_load_force = loadRead ? (loadRaw - LOAD_TARE) * LOAD_SCALE : NAN;
  v.wrist_health_tick_dt = static_cast<int32_t>(longestTick);
  // No frame yet is as old as a frame can be.
  v.dmx_wrist_sacn_age = frameAt == 0 ? INT32_MAX : static_cast<int32_t>(millis() - frameAt);
  v.dmx_wrist_sacn_packets = static_cast<int32_t>(sacnPackets);
  v.dmx_wrist_sacn_frames = static_cast<int32_t>(sacnFrames);
  v.dmx_wrist_sacn_skipped = static_cast<int32_t>(sacnSkipped);
  v.sys_wrist_uptime = static_cast<int32_t>(millis() / 1000);
  v.sys_wrist_heap_free = static_cast<int32_t>(ESP.getFreeHeap());
  v.sys_wrist_wifi_rssi = WiFi.RSSI();
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
  watchWifi();
  WiFi.mode(WIFI_STA);
  WiFi.setSleepMode(WIFI_NONE_SLEEP);
  WiFi.setAutoReconnect(true);
  WiFi.config(IPAddress(net::ADDRESS), IPAddress(net::GATEWAY), IPAddress(net::NETMASK));
  WiFi.begin(net::WIFI_SSID, net::WIFI_PSK, net::WIFI_CHANNEL);
  sacnUdp.begin(net::SACN_PORT);

  Serial.println();
  say("start (%s), joining \"%s\" as %u.%u.%u.%u, sACN universe %u on port %u, DMX %u–%u",
      ESP.getResetReason().c_str(), net::WIFI_SSID, net::ADDRESS[0], net::ADDRESS[1],
      net::ADDRESS[2], net::ADDRESS[3], patch::SACN_UNIVERSE, net::SACN_PORT,
      patch::DMX_ADDRESS, patch::DMX_ADDRESS + patch::CHANNELS - 1);
}

void loop() {
  static uint32_t lastTick = micros();
  static bool connected = false;
  static uint32_t stateDue = 0;
  static uint32_t statusDue = STATUS_PERIOD_MS;

  const uint32_t now = micros();
  const uint32_t tick = now - lastTick;
  lastTick = now;
  if (tick > longestTick) {
    longestTick = tick;
  }
  if (tick > longestTickLogged) {
    longestTickLogged = tick;
  }
  if (tick > STALL_LOG_US) {
    say("loop stalled %lu ms", static_cast<unsigned long>(tick / 1000));
  }
  if (static_cast<int32_t>(millis() - statusDue) >= 0) {
    logStatus();
    statusDue = millis() + STATUS_PERIOD_MS;
  }

  // Lit while the motor is enabled, blinking at 1 Hz while it is released.
  digitalWrite(PIN_LED, mode != Mode::blackout || (millis() / BLINK_HALF_PERIOD_MS) % 2 == 0
                            ? LOW
                            : HIGH);

  // The motor first, and whether or not the WiFi is up: with the WiFi gone the frames stop,
  // and the timeout is what releases the shaft. Through a pass held up by the WiFi stack the
  // step generator runs on at the speed it has, and still stops itself on a move's target.
  receiveSacn();
  updateMotor();

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
    say("WiFi lost");
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
