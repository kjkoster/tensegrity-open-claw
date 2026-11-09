// OSC 1.0 encoding: the few primitives the generated encoders in generated.h are written in.
//
// Everything is big-endian and padded to four bytes. A write that would run past the buffer
// marks the writer failed instead, and a failed writer reports a length of 0, so a message
// too big for its buffer is dropped whole rather than sent cut short.
#pragma once

#include <stddef.h>
#include <stdint.h>
#include <string.h>

namespace osc {

class Writer {
 public:
  Writer(uint8_t *buf, size_t cap) : buf_(buf), cap_(cap) {}

  // The bytes written, or 0 if any write did not fit.
  size_t length() const { return ok_ ? len_ : 0; }

  // An OSC string: the bytes, a NUL, and NULs up to a multiple of four.
  void string(const char *s) {
    const size_t n = strlen(s);
    bytes(s, n);
    const uint8_t zeros[4] = {0, 0, 0, 0};
    bytes(zeros, 4 - (n % 4));
  }

  void address(const char *path) { string(path); }

  void int32(int32_t v) { uint32(static_cast<uint32_t>(v)); }

  void float32(float v) {
    uint32_t bits;
    memcpy(&bits, &v, sizeof bits);
    uint32(bits);
  }

  // A bundle's header, timed "immediately": Wrist keeps no wall-clock time to stamp it with.
  void beginBundle() {
    string("#bundle");
    uint32(0);
    uint32(1);
  }

  // Reserves an element's size, to be filled in by endElement once the element is written.
  size_t beginElement() {
    const size_t at = len_;
    uint32(0);
    return at;
  }

  void endElement(size_t at) {
    if (!ok_) {
      return;
    }
    const uint32_t size = static_cast<uint32_t>(len_ - at - 4);
    const uint8_t be[4] = {uint8_t(size >> 24), uint8_t(size >> 16), uint8_t(size >> 8), uint8_t(size)};
    memcpy(buf_ + at, be, 4);
  }

 private:
  void uint32(uint32_t v) {
    const uint8_t be[4] = {uint8_t(v >> 24), uint8_t(v >> 16), uint8_t(v >> 8), uint8_t(v)};
    bytes(be, 4);
  }

  void bytes(const void *p, size_t n) {
    if (!ok_ || n > cap_ - len_) {
      ok_ = false;
      return;
    }
    memcpy(buf_ + len_, p, n);
    len_ += n;
  }

  uint8_t *buf_;
  size_t cap_;
  size_t len_ = 0;
  bool ok_ = true;
};

}  // namespace osc
