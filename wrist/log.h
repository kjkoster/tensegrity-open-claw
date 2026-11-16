// Wrist's serial log: one line per event, and a status line every STATUS_PERIOD_MS on the link
// and the loop. The counts it keeps are sent over OSC as well. Included after generated.h. In a header rather than in wrist.ino, because the Arduino build puts its own
// prototypes for the sketch's functions before the sketch's first function, and these, first
// there, would put them before the types they name.
//
// The serial port has no transmit buffer past the UART's 128-byte FIFO, and a write that does
// not fit what is left of it holds loop() up until it does, about 0.13 ms a byte at 74880 baud.
// So say() only queues a line, and drainLog() hands the UART, once a pass, as much as its FIFO
// has room for: no line holds loop() up. The log is still events and the status lines, never
// anything once a pass, so the queue keeps up. A crash loses what was queued and not yet out;
// the boot ROM's crash dump goes to the UART directly, and is not lost.

#pragma once

#include <ESP8266WiFi.h>
#include <stdarg.h>

const uint32_t STATUS_PERIOD_MS = 10000;
// A pass of loop() longer than this is logged as it happens, besides the status line's longest.
const uint32_t STALL_LOG_US = 20000;

// Lines waiting for the UART: a ring, room for the status lines and a burst of events. A line
// that does not fit what is left is dropped whole, and the drops are logged once there is room.
char logQueue[2048];
size_t logHead = 0;     // where the next line goes
size_t logUsed = 0;
uint32_t logDropped = 0;

void queueLine(const char *text, size_t length) {
  if (length > sizeof logQueue - logUsed) {
    logDropped++;
    return;
  }
  for (size_t i = 0; i < length; i++) {
    logQueue[(logHead + i) % sizeof logQueue] = text[i];
  }
  logHead = (logHead + length) % sizeof logQueue;
  logUsed += length;
}

// Every line starts with the time since boot, in seconds, so the lines can be lined up with each
// other and with the recording. The time is when it was said, not when it went out.
void say(const char *format, ...) __attribute__((format(printf, 1, 2)));
void say(const char *format, ...) {
  char text[160];
  va_list args;
  va_start(args, format);
  vsnprintf(text, sizeof text, format, args);
  va_end(args);
  char line[180];
  const uint32_t ms = millis();
  const int length = snprintf(line, sizeof line, "wrist %lu.%03lu: %s\n",
                              static_cast<unsigned long>(ms / 1000),
                              static_cast<unsigned long>(ms % 1000), text);
  queueLine(line, min(static_cast<size_t>(length), sizeof line - 1));
}

// Once a pass of loop(): as much of the queue as the UART's FIFO takes without waiting.
void drainLog() {
  size_t room = Serial.availableForWrite();
  while (room > 0 && logUsed > 0) {
    const size_t tail = (logHead + sizeof logQueue - logUsed) % sizeof logQueue;
    const size_t run = min(min(logUsed, sizeof logQueue - tail), room);
    Serial.write(reinterpret_cast<const uint8_t *>(logQueue + tail), run);
    logUsed -= run;
    room -= run;
  }
  if (logDropped > 0 && logUsed == 0) {
    const uint32_t dropped = logDropped;
    logDropped = 0;
    say("%lu log lines dropped, the queue full", static_cast<unsigned long>(dropped));
  }
}

// The longest gap between two passes of loop() since the last status line, in µs.
uint32_t longestTickLogged = 0;

// What came in on the sACN port since boot, for the log and for /dmx/wrist/sacn_…: every
// datagram, every frame taken, and the frames the sender's sequence numbers say never arrived.
uint32_t sacnPackets = 0;
uint32_t sacnFrames = 0;
uint32_t sacnSkipped = 0;
// Datagrams since the last frame taken, so a loss tells silence from packets that were not ours.
uint32_t packetsSinceFrame = 0;
// The last frame's sequence number, or -1 with no frame since the stream was last lost.
int16_t lastSequence = -1;

constexpr size_t RECIPIENTS = sizeof net::OSC_RECIPIENTS / sizeof net::OSC_RECIPIENTS[0];
// OSC packets since boot: those handed to the WiFi stack, and per recipient those it refused,
// with no buffer free or the link down. A refused packet is gone; one taken can still be lost
// in the air, which only its recipient can see.
uint32_t oscSent = 0;
uint32_t oscRefused[RECIPIENTS] = {};
// The refusals in a row to each recipient, so a run is logged once as it starts and once as it
// ends, rather than ten times a second.
uint32_t oscRefusing[RECIPIENTS] = {};

void noteOscSend(size_t recipient, bool taken) {
  const char *name = net::OSC_RECIPIENTS[recipient].name;
  oscSent++;
  if (taken) {
    if (oscRefusing[recipient] > 0) {
      say("OSC to %s taken again, after %lu refused", name,
          static_cast<unsigned long>(oscRefusing[recipient]));
      oscRefusing[recipient] = 0;
    }
    return;
  }
  oscRefused[recipient]++;
  if (oscRefusing[recipient]++ == 0) {
    say("OSC to %s refused by the WiFi stack", name);
  }
}

using DisconnectReason = osc::WifiWristDisconnectReason;

// The WiFi stack's reasons for a disconnect, the ones a station sees, from the SDK's numbers to
// /wifi/wrist/disconnect_reason's values.
DisconnectReason disconnectReason(uint8_t reason) {
  switch (reason) {
    case 1: return DisconnectReason::unspecified;
    case 2: return DisconnectReason::auth_expired;
    case 3: return DisconnectReason::deauth_leaving;
    case 4: return DisconnectReason::inactivity;
    case 5: return DisconnectReason::ap_busy;
    case 6: return DisconnectReason::not_authenticated;
    case 7: return DisconnectReason::not_associated;
    case 8: return DisconnectReason::left;
    case 15: return DisconnectReason::four_way_handshake_timeout;
    case 16: return DisconnectReason::group_key_timeout;
    case 23: return DisconnectReason::auth_failed_8021x;
    case 200: return DisconnectReason::beacon_timeout;
    case 201: return DisconnectReason::no_ap_found;
    case 202: return DisconnectReason::auth_failed;
    case 203: return DisconnectReason::assoc_failed;
    case 204: return DisconnectReason::handshake_timeout;
    default: return DisconnectReason::other;
  }
}

// Disconnects since boot: one per failed attempt while it rejoins, too. And the last one's reason.
uint32_t disconnects = 0;
DisconnectReason lastDisconnect = DisconnectReason::none;

// Held for as long as they are to be called: the WiFi stack drops a handler whose handle goes.
WiFiEventHandler onConnected;
WiFiEventHandler onDisconnected;

void watchWifi() {
  onConnected = WiFi.onStationModeConnected([](const WiFiEventStationModeConnected &e) {
    say("WiFi associated with %02x:%02x:%02x:%02x:%02x:%02x on channel %u", e.bssid[0],
        e.bssid[1], e.bssid[2], e.bssid[3], e.bssid[4], e.bssid[5], e.channel);
  });
  onDisconnected = WiFi.onStationModeDisconnected([](const WiFiEventStationModeDisconnected &e) {
    disconnects++;
    lastDisconnect = disconnectReason(e.reason);
    say("WiFi disconnected: %s (%u), %lu since boot",
        osc::WifiWristDisconnectReasonNames[static_cast<uint8_t>(lastDisconnect)], e.reason,
        static_cast<unsigned long>(disconnects));
  });
}

// One line on how the link and the loop have been since the last.
void logStatus() {
  static uint32_t packetsBefore = 0;
  static uint32_t framesBefore = 0;
  static uint32_t skippedBefore = 0;
  static uint32_t sentBefore = 0;
  static uint32_t refusedBefore = 0;
  uint32_t refused = 0;
  for (size_t i = 0; i < RECIPIENTS; i++) {
    refused += oscRefused[i];
  }
  char wifi[40];
  if (WiFi.status() == WL_CONNECTED) {
    snprintf(wifi, sizeof wifi, "up %d dBm ch %d", WiFi.RSSI(), WiFi.channel());
  } else {
    snprintf(wifi, sizeof wifi, "down (status %d)", static_cast<int>(WiFi.status()));
  }
  say("status: WiFi %s; sACN %lu packets, %lu frames, %lu skipped; OSC %lu sent, %lu refused; "
      "loop longest %lu us; heap %lu free, %u%% fragmented",
      wifi, static_cast<unsigned long>(sacnPackets - packetsBefore),
      static_cast<unsigned long>(sacnFrames - framesBefore),
      static_cast<unsigned long>(sacnSkipped - skippedBefore),
      static_cast<unsigned long>(oscSent - sentBefore),
      static_cast<unsigned long>(refused - refusedBefore),
      static_cast<unsigned long>(longestTickLogged),
      static_cast<unsigned long>(ESP.getFreeHeap()), ESP.getHeapFragmentation());
  packetsBefore = sacnPackets;
  framesBefore = sacnFrames;
  skippedBefore = sacnSkipped;
  sentBefore = oscSent;
  refusedBefore = refused;
  longestTickLogged = 0;
}
