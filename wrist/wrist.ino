// Wrist's firmware, the start of it: the board on the WiFi, the LED blinking, and every OSC
// address in wrist.oscquery.json sent the way the file says. The motor stays released and the
// mode is always Blackout; the sACN receiver, the accelerometer and the modes come later, per
// CONTROL_LOOP.md.
//
// The network, the patch and the OSC encoders are in generated.h, which build.sh writes from
// clockwork.dot, clockwork.qxw and wrist.oscquery.json before every build.

#include <ESP8266WiFi.h>
#include <WiFiUdp.h>
#include <math.h>

#include "generated.h"

// The optos share a common anode on 3.3 V, so a pin driven low lights its opto.
const uint8_t PIN_PUL = 14;     // D5
const uint8_t PIN_DIR = 12;     // D6
// Driven low, the motor is enabled; released to an input, it is limp. Left released here.
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

// 1 Hz: half a second lit, half dark.
const uint32_t BLINK_HALF_PERIOD_MS = 500;

// Room for the largest bundle with a margin; an encoder that would overrun it sends nothing.
uint8_t packet[512];

WiFiUDP udp;

// The latest load cell reading, and whether there has been one.
int32_t loadRaw = 0;
bool loadRead = false;

// The longest gap between two passes of loop() since the last state bundle, in µs.
uint32_t longestTick = 0;

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
  v.wrist_mode = osc::WristMode::blackout;
  v.wrist_alarm = digitalRead(PIN_ALM) == LOW;
  v.wrist_load_force = loadRead ? (loadRaw - LOAD_TARE) * LOAD_SCALE : NAN;
  v.wrist_health_tick_dt = static_cast<int32_t>(longestTick);
  // No sACN receiver yet, so no frame has ever arrived: the age is as large as it can be.
  v.dmx_wrist_sacn_age = INT32_MAX;
  sendToRecipients(osc::encodeAt10hz(v, packet, sizeof packet));
  longestTick = 0;
}

// Returns false while the HX711 has no new reading.
bool readLoadCell(int32_t &value) {
  if (digitalRead(PIN_HX_DT) == HIGH) {
    return false;
  }
  uint32_t raw = 0;
  // SCK held high for more than 60 µs powers the chip down, so no interrupt may stretch a clock.
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

void setup() {
  // Transmit only: RX is the alarm input. At 74880, the boot ROM's own rate, so its reset cause
  // and any crash dump read in the same log as the firmware's lines.
  Serial.begin(74880, SERIAL_8N1, SERIAL_TX_ONLY);

  // Idle levels are written before the pins become outputs, so no opto flashes on at start.
  digitalWrite(PIN_PUL, HIGH);
  pinMode(PIN_PUL, OUTPUT);
  digitalWrite(PIN_DIR, HIGH);
  pinMode(PIN_DIR, OUTPUT);
  pinMode(PIN_ENA, INPUT);
  digitalWrite(PIN_LED, HIGH);
  pinMode(PIN_LED, OUTPUT);
  pinMode(PIN_ALM, INPUT_PULLUP);
  pinMode(PIN_HX_DT, INPUT);
  digitalWrite(PIN_HX_SCK, LOW);
  pinMode(PIN_HX_SCK, OUTPUT);

  // The address is fixed, from clockwork.dot, and modem sleep is off, so a packet to Wrist is
  // never held for a beacon interval. Not persisted: the settings come from the build, not flash.
  WiFi.persistent(false);
  WiFi.mode(WIFI_STA);
  WiFi.setSleepMode(WIFI_NONE_SLEEP);
  WiFi.setAutoReconnect(true);
  WiFi.config(IPAddress(net::ADDRESS), IPAddress(net::GATEWAY), IPAddress(net::NETMASK));
  WiFi.begin(net::WIFI_SSID, net::WIFI_PSK, net::WIFI_CHANNEL);

  Serial.println();
  Serial.printf("wrist: start, joining \"%s\" as %u.%u.%u.%u\n", net::WIFI_SSID, net::ADDRESS[0],
                net::ADDRESS[1], net::ADDRESS[2], net::ADDRESS[3]);
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

  digitalWrite(PIN_LED, (millis() / BLINK_HALF_PERIOD_MS) % 2 == 0 ? LOW : HIGH);

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
