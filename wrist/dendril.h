// Dendril, the sensing: the ADXL345 accelerometer on the structure, on a cable over I²C, and the
// load cell under the motor through its HX711. Compiled into Wrist's firmware, it reports as a
// device of its own, fixtures/dendril.oscquery.json, so it can move to a board of its own.
// Included after generated.h and log.h; in a header for log.h's reason.
//
// The accelerometer samples on its own clock into its FIFO, and is read in batches, so a
// sample's time of reading says little of when it was taken. Each sample is numbered as it comes
// out of the FIFO, and the chip's clock is fitted against the board's (ChipClock), so each
// sample's time is worked out from its number.

#pragma once

#include <Wire.h>

namespace dendril {

// ── The sway ─────────────────────────────────────────────────────────────────────────────────
//
// The sway's frequency, by counting its cycles: the same on each of the accelerometer's axes and
// on the tension, so the two sensors check each other. A slow running mean is taken off, which
// takes gravity and the wire's standing tension with it; what is left crosses zero upwards once
// a cycle. A crossing counts only after the signal has been below the hysteresis and before it
// goes above it, so noise near zero adds none; its time is interpolated between the samples
// either side. The frequency is from the median of the last few periods; with no crossing for a
// while, there is no sway. Counting needs a few samples a cycle: the tension, at about 10
// readings a second, sees a sway to about 3.5 Hz, the accelerometer at 50 to 10 Hz.

// The running mean's time constant: well below the slowest sway, 1/(2π·3 s) = 0.05 Hz.
const float MEAN_S = 3;
// How fast the swing's size follows.
const float LEVEL_S = 2;
// The hysteresis, as a part of the swing.
const float HYSTERESIS = 1.0f / 3;
// The periods taken as a sway: 0.2 to 10 Hz.
const float PERIOD_MIN_S = 0.1f;
const float PERIOD_MAX_S = 5;
const uint8_t PERIODS = 8;

struct Sway {
  // The least hysteresis, in the signal's units: above its noise, so a still structure shows none.
  float floor;

  bool primed = false;
  double lastS = 0;
  float mean = 0;
  float swing = 0;              // the mean of |signal − mean|
  float lastAc = 0;
  // Waiting to be below the hysteresis, then for the upward crossing, then to be above it.
  enum class Wait : uint8_t { low, crossing, high } wait = Wait::low;
  double crossedS = NAN;
  float periods[PERIODS] = {};
  uint8_t count = 0;
  uint8_t next = 0;

  explicit Sway(float floor) : floor(floor) {}

  // A sample `value` taken at `s`, in seconds on the board's clock.
  void take(double s, float value) {
    if (!primed) {
      mean = value;
      lastS = s;
      primed = true;
      return;
    }
    const float dt = s - lastS;
    if (dt <= 0) {
      return;
    }
    mean += (value - mean) * min(1.0f, dt / MEAN_S);
    const float ac = value - mean;
    swing += (fabsf(ac) - swing) * min(1.0f, dt / LEVEL_S);
    const float h = max(floor, HYSTERESIS * amplitude());
    switch (wait) {
      case Wait::low:
        if (ac < -h) {
          wait = Wait::crossing;
        }
        break;
      case Wait::crossing:
        if (ac >= 0) {
          cross(lastS + (s - lastS) * (-lastAc / (ac - lastAc)));
          // With a few samples a cycle, the crossing's own sample may be the peak.
          wait = ac > h ? Wait::low : Wait::high;
        }
        break;
      case Wait::high:
        if (ac > h) {
          wait = Wait::low;
        }
        break;
    }
    lastAc = ac;
    lastS = s;
  }

  void cross(double s) {
    const float period = s - crossedS;
    crossedS = s;
    if (!(period >= PERIOD_MIN_S && period <= PERIOD_MAX_S)) {
      count = 0;
      return;
    }
    periods[next] = period;
    next = (next + 1) % PERIODS;
    if (count < PERIODS) {
      count++;
    }
  }

  // The size of the swing, as a sine's amplitude: its mean absolute value is 2/π of it.
  float amplitude() const { return swing * PI / 2; }

  // The sway's frequency, in Hz, or NaN with no sway: too few periods yet, or none lately. From
  // the median period, so a cycle missed, which counts as one period twice as long, moves it
  // not at all.
  float frequency() const {
    if (count < 3 || !(lastS - crossedS < 2 * PERIOD_MAX_S)) {
      return NAN;
    }
    float sorted[PERIODS];
    for (uint8_t i = 0; i < count; i++) {
      uint8_t j = i;
      for (; j > 0 && sorted[j - 1] > periods[i]; j--) {
        sorted[j] = sorted[j - 1];
      }
      sorted[j] = periods[i];
    }
    const float median = count % 2 ? sorted[count / 2] : (sorted[count / 2 - 1] + sorted[count / 2]) / 2;
    return lastS - crossedS < 2 * median ? 1 / median : NAN;
  }
};

// The accelerometer's noise is about a count; the tension's about 1 mN.
Sway swayX(4), swayY(4), swayZ(4);
Sway swayTension(0.004f);

// ── The accelerometer ────────────────────────────────────────────────────────────────────────

// I²C on D1 and D2, at 100 kHz: a cable of a metre or two carries it where 400 kHz's edges are
// too fast for the breakout's pull-ups.
const uint8_t PIN_SDA = 4;      // D2
const uint8_t PIN_SCL = 5;      // D1
const uint32_t I2C_HZ = 100000;

// The ADXL345, SDO tied to GND.
const uint8_t ADXL_ADDRESS = 0x53;
const uint8_t REG_DEVID = 0x00;
const uint8_t DEVID = 0xE5;
const uint8_t REG_BW_RATE = 0x2C;
const uint8_t REG_POWER_CTL = 0x2D;
const uint8_t REG_DATA_FORMAT = 0x31;
const uint8_t REG_DATAX0 = 0x32;
const uint8_t REG_FIFO_CTL = 0x38;
const uint8_t REG_FIFO_STATUS = 0x39;

// 50 samples a second: ten a cycle of a 5 Hz sway. The chip's filter follows the rate, its
// bandwidth half of it, so every sample is sent and none is thinned out here.
const uint8_t BW_RATE_50_HZ = 0x09;
const double NOMINAL_PERIOD_US = 20000;
// ±2 g at full resolution, right-justified: 10 bits, about 3.9 mg a count.
const uint8_t DATA_FORMAT_FULL_RES_2G = 0x08;
// Stream: the FIFO keeps the newest 32 samples, dropping the oldest when it is full.
const uint8_t FIFO_BYPASS = 0x00;
const uint8_t FIFO_STREAM = 0x80;
const uint8_t POWER_STANDBY = 0x00;
const uint8_t POWER_MEASURE = 0x08;
// The FIFO's 32 and the output registers' one.
const uint8_t FIFO_MAX = 33;
// At or past this many waiting, the FIFO may have dropped samples.
const uint8_t FIFO_FULL = 32;

// How often the FIFO is read, on average: about two samples a read. Each read is dithered across
// the whole of this, so the reads fall at every phase of the chip's samples and ChipClock's
// estimate is not biased by a read that keeps step with them.
const uint32_t POLL_US = 40000;
// With no chip answering, how long before it is looked for again.
const uint32_t RETRY_MS = 5000;

// The chip's clock against the board's. A sample is never taken after the read that finds it,
// so each read (n, r), n the newest sample it found, bounds that sample's time from above, and
// the chip's clock is a line on or under every read. A pass held up by the WiFi only reads
// later, which loosens its bound and moves no estimate. Over a window of reads, the line is the
// lower convex hull's edge above the window's mean sample, the usual fit of clock skew from
// one-sided delays; it gives the period and where the line lies, and the next window's samples
// are timed from it. Between fits, a read that comes before the line says a sample was taken
// moves the line back to it at once.
//
// Simulated with reads dithered as here, a chip clock 5% off nominal and stalls of up to 300 ms
// in one pass in a hundred: about 0.25 ms of scatter, under 2 ms at worst. Until the first
// window is in, about 20 s after boot, the samples are timed from the nominal period.
const uint16_t WINDOW = 500;    // reads

struct ChipClock {
  bool started = false;
  // The line: sample anchorN was taken at anchorUs, on the board's clock since boot.
  int32_t anchorN = 0;
  double anchorUs = 0;
  double periodUs = NOMINAL_PERIOD_US;
  // The window's reads, from its first: samples on, and µs on, 20 s of them in 32 bits.
  int32_t baseN = 0;
  double baseUs = 0;
  struct Read {
    int32_t n;
    uint32_t us;
  } reads[WINDOW];
  uint16_t count = 0;

  // When sample `n` was taken.
  double timeOf(int32_t n) const { return anchorUs + (n - anchorN) * periodUs; }

  // Starts again from a read at `readUs` whose newest sample is `n`, keeping the period.
  void restart(int32_t n, double readUs) {
    anchorN = n;
    anchorUs = readUs - periodUs / 2;
    count = 0;
    started = true;
  }

  // Takes a read at `readUs` whose newest sample is `n`, past the last read's.
  void update(int32_t n, double readUs) {
    if (!started) {
      restart(n, readUs);
    }
    if (readUs < timeOf(n)) {
      anchorUs -= timeOf(n) - readUs;
    }
    if (count == 0) {
      baseN = n;
      baseUs = readUs;
    }
    reads[count++] = {n - baseN, static_cast<uint32_t>(readUs - baseUs)};
    if (count == WINDOW) {
      fit();
      count = 0;
    }
  }

  // The lower hull of the window's reads, in place, and the edge above their mean sample.
  void fit() {
    double sum = 0;
    for (uint16_t i = 0; i < count; i++) {
      sum += reads[i].n;
    }
    const double mean = sum / count;
    uint16_t h = 0;
    for (uint16_t i = 0; i < count; i++) {
      const Read p = reads[i];
      // Pops the last vertex while it does not lie strictly under the chord to the new read.
      while (h >= 2) {
        const Read a = reads[h - 2];
        const Read b = reads[h - 1];
        const int64_t cross = static_cast<int64_t>(b.n - a.n) * (static_cast<int64_t>(p.us) - a.us) -
                              (static_cast<int64_t>(b.us) - a.us) * static_cast<int64_t>(p.n - a.n);
        if (cross > 0) {
          break;
        }
        h--;
      }
      reads[h++] = p;
    }
    for (uint16_t i = 0; i + 1 < h; i++) {
      const Read a = reads[i];
      const Read b = reads[i + 1];
      if (a.n <= mean && mean <= b.n) {
        periodUs = (static_cast<double>(b.us) - a.us) / (b.n - a.n);
        anchorN = baseN + a.n;
        anchorUs = baseUs + a.us;
        return;
      }
    }
  }
};

ChipClock chipClock;
// Whether the chip answered as an ADXL345 and is set up.
bool present = false;
uint32_t lookedAt = 0;
uint32_t nextPollUs = 0;
// The next sample's number: counted on from the chip's first sample since boot, so a gap in the
// numbers is samples lost.
int32_t nextN = 0;
// Whether the next read follows the chip being set up again, after the clock started.
bool resync = false;

// For the log's status line, since the last one: samples read and lost, the most found waiting
// in the FIFO at one read, and the last sample.
uint32_t samplesRead = 0;
uint32_t samplesLost = 0;
uint8_t mostWaiting = 0;
int16_t lastX = 0, lastY = 0, lastZ = 0;
uint32_t tensionReads = 0;

bool writeRegister(uint8_t reg, uint8_t value) {
  Wire.beginTransmission(ADXL_ADDRESS);
  Wire.write(reg);
  Wire.write(value);
  return Wire.endTransmission() == 0;
}

bool readRegisters(uint8_t reg, uint8_t *into, uint8_t count) {
  Wire.beginTransmission(ADXL_ADDRESS);
  Wire.write(reg);
  if (Wire.endTransmission(false) != 0) {
    return false;
  }
  if (Wire.requestFrom(ADXL_ADDRESS, count) != count) {
    return false;
  }
  for (uint8_t i = 0; i < count; i++) {
    into[i] = Wire.read();
  }
  return true;
}

// Looks for the chip and sets it measuring, the FIFO emptied first. Logs what it finds once
// per change, not every retry.
void setUp() {
  static bool missingLogged = false;
  lookedAt = millis();
  uint8_t id = 0;
  if (!readRegisters(REG_DEVID, &id, 1) || id != DEVID) {
    if (!missingLogged) {
      say("Dendril: no ADXL345 at 0x%02x (read 0x%02x); looking again every %lu s",
          ADXL_ADDRESS, id, static_cast<unsigned long>(RETRY_MS / 1000));
      missingLogged = true;
    }
    return;
  }
  const bool set = writeRegister(REG_POWER_CTL, POWER_STANDBY) &&
                   writeRegister(REG_BW_RATE, BW_RATE_50_HZ) &&
                   writeRegister(REG_DATA_FORMAT, DATA_FORMAT_FULL_RES_2G) &&
                   writeRegister(REG_FIFO_CTL, FIFO_BYPASS) &&
                   writeRegister(REG_FIFO_CTL, FIFO_STREAM) &&
                   writeRegister(REG_POWER_CTL, POWER_MEASURE);
  if (!set) {
    say("Dendril: the ADXL345 did not take its settings");
    return;
  }
  missingLogged = false;
  present = true;
  resync = chipClock.started;
  nextPollUs = micros();
  say("Dendril: ADXL345 measuring");
}

void begin() {
  Wire.begin(PIN_SDA, PIN_SCL);
  Wire.setClock(I2C_HZ);
  setUp();
}

void lost() {
  present = false;
  say("Dendril: the ADXL345 stopped answering");
}

// Once per pass of loop(). When a read is due, empties the FIFO into `out`, oldest first, each
// sample numbered and timed, and returns how many; otherwise 0.
uint8_t pollAccel(osc::DendrilAccel *out) {
  if (!present) {
    if (millis() - lookedAt >= RETRY_MS) {
      setUp();
    }
    return 0;
  }
  if (static_cast<int32_t>(micros() - nextPollUs) < 0) {
    return 0;
  }
  nextPollUs = micros() + POLL_US / 2 + ESP.random() % POLL_US;

  uint8_t status;
  if (!readRegisters(REG_FIFO_STATUS, &status, 1)) {
    lost();
    return 0;
  }
  const double readUs = static_cast<double>(micros64());
  const uint8_t entries = status & 0x3F;
  const uint8_t waiting = entries > FIFO_MAX ? FIFO_MAX : entries;
  if (waiting == 0) {
    return 0;
  }

  // A full FIFO may have dropped samples, and a chip set up again restarted its own: the
  // numbers skip as many as the time says were taken meanwhile.
  if (chipClock.started && (waiting >= FIFO_FULL || resync)) {
    const double expected = chipClock.timeOf(nextN + waiting - 1);
    const long skipped = lround((readUs - chipClock.periodUs / 2 - expected) / chipClock.periodUs);
    if (skipped > 0) {
      nextN += skipped;
      samplesLost += skipped;
      say("Dendril: %ld samples lost%s", skipped, resync ? " while the ADXL345 was away" : "");
    }
  }

  for (uint8_t i = 0; i < waiting; i++) {
    uint8_t b[6];
    if (!readRegisters(REG_DATAX0, b, sizeof b)) {
      lost();
      return 0;
    }
    out[i].dendril_accel_x = static_cast<int16_t>(b[1] << 8 | b[0]);
    out[i].dendril_accel_y = static_cast<int16_t>(b[3] << 8 | b[2]);
    out[i].dendril_accel_z = static_cast<int16_t>(b[5] << 8 | b[4]);
    out[i].dendril_accel_sample_n = nextN + i;
  }
  samplesRead += waiting;
  if (waiting > mostWaiting) {
    mostWaiting = waiting;
  }
  lastX = out[waiting - 1].dendril_accel_x;
  lastY = out[waiting - 1].dendril_accel_y;
  lastZ = out[waiting - 1].dendril_accel_z;
  const int32_t newest = nextN + waiting - 1;
  if (resync) {
    chipClock.restart(newest, readUs);
    resync = false;
  } else {
    chipClock.update(newest, readUs);
  }
  nextN += waiting;
  // In ms since boot, an int32 for 24 days: the firmware is not left running that long.
  for (uint8_t i = 0; i < waiting; i++) {
    const double us = chipClock.timeOf(out[i].dendril_accel_sample_n);
    out[i].dendril_accel_sampled_ms = static_cast<int32_t>(llround(us / 1000));
    swayX.take(us / 1e6, out[i].dendril_accel_x);
    swayY.take(us / 1e6, out[i].dendril_accel_y);
    swayZ.take(us / 1e6, out[i].dendril_accel_z);
  }
  return waiting;
}

// ── The load cell ────────────────────────────────────────────────────────────────────────────

const uint8_t PIN_HX_DT = 16;   // D0
// SCK must idle low: GPIO15 low is a boot condition, and held high it powers the HX711 down.
const uint8_t PIN_HX_SCK = 15;  // D8

// The load cell's calibration: tension = (raw − tare) · scale. Measured 2026-11-15, mounted with
// the motor and no wire, against 737, 1070 and 1370 g weighed on a kitchen scale and set on
// the motor, g = 9.81; the three agree to 0.1%. Weight on the motor lowers the counts; the
// wire pulls the other way, up, so the scale is positive and tension reads positive. Taken to
// be symmetric: the pull up itself was not calibrated.
const int32_t LOAD_TARE = -553801;      // raw counts
const float LOAD_SCALE = 2.3109e-5f;    // N per count

// The latest reading, and whether there has been one.
int32_t loadRaw = 0;
bool loadRead = false;

void beginLoadCell() {
  pinMode(PIN_HX_DT, INPUT);
  digitalWrite(PIN_HX_SCK, LOW);
  pinMode(PIN_HX_SCK, OUTPUT);
}

// Returns false while the HX711 has no new reading. The chip says a reading is ready by DT
// going low, and is read within the pass that finds it, so the time it is read is when it was
// ready, to within a pass.
bool readLoadCell(osc::DendrilTension &out) {
  if (digitalRead(PIN_HX_DT) == HIGH) {
    return false;
  }
  const uint32_t readMs = millis();
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
  loadRaw = static_cast<int32_t>(raw << 8) >> 8;
  loadRead = true;
  tensionReads++;
  swayTension.take(readMs / 1000.0, (loadRaw - LOAD_TARE) * LOAD_SCALE);
  out.dendril_tension_raw = loadRaw;
  out.dendril_tension_sampled_ms = static_cast<int32_t>(readMs);
  return true;
}

// The wire's tension, or NaN before the first reading.
float tension() { return loadRead ? (loadRaw - LOAD_TARE) * LOAD_SCALE : NAN; }

// ── The log ──────────────────────────────────────────────────────────────────────────────────

// One line on how Dendril has been since the last, beside the log's status line: the
// accelerometer's samples, the chip's period as fitted, the last sample's axes in counts, and
// the tension's readings.
void logStatus() {
  if (present) {
    say("dendril: %lu samples, %lu lost, FIFO up to %u; period %.1f us; last x %d y %d z %d; "
        "tension %lu readings, %.2f N",
        static_cast<unsigned long>(samplesRead), static_cast<unsigned long>(samplesLost),
        mostWaiting, chipClock.periodUs, lastX, lastY, lastZ,
        static_cast<unsigned long>(tensionReads), tension());
  } else {
    say("dendril: no ADXL345; tension %lu readings, %.2f N",
        static_cast<unsigned long>(tensionReads), tension());
  }
  samplesRead = 0;
  samplesLost = 0;
  mostWaiting = 0;
  tensionReads = 0;

  // The sway as each sensor sees it: the accelerometer's axis that swings most, in counts, and
  // the tension, in newtons; "none" where it is still.
  const Sway *axes[] = {&swayX, &swayY, &swayZ};
  const char names[] = {'x', 'y', 'z'};
  int best = -1;
  for (int i = 0; i < 3; i++) {
    if (!isnan(axes[i]->frequency()) &&
        (best < 0 || axes[i]->amplitude() > axes[best]->amplitude())) {
      best = i;
    }
  }
  char accel[48] = "none";
  if (best >= 0) {
    snprintf(accel, sizeof accel, "%.3f Hz on %c, %.1f counts", axes[best]->frequency(),
             names[best], axes[best]->amplitude());
  }
  char tensionSway[40] = "none";
  if (!isnan(swayTension.frequency())) {
    snprintf(tensionSway, sizeof tensionSway, "%.3f Hz, %.3f N", swayTension.frequency(),
             swayTension.amplitude());
  }
  say("sway: accelerometer %s; tension %s", accel, tensionSway);
}

}  // namespace dendril
