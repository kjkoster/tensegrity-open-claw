#!/usr/bin/env python3
"""The recorder: everything published on the broker, kept in Greptime.

It subscribes to `#` and writes each live message into the `rig` database, so a show can be
read back afterwards. Greptime has no MQTT listener of its own, and its documented route is
EMQX's rule engine, which would mean replacing Mosquitto for a few messages a second.

One table per topic's first level — `stem`, `memory`, `brain`, `health`, `sys` — with the rest
of the topic as the `topic` tag. The payload goes into `value` when it is a number, `true` and
`false` as 1 and 0 so a flag draws as a line, and into `text` otherwise. The time is when the
recorder received it.

Not recorded:

- Retained replays. The broker replays every retained topic on each connect; recording them
  would write each state again every time. A live publish arrives without the retain flag.
- Empty payloads, which on this rig delete a retained topic rather than say anything.
- Anything before the clock has synchronised once, which would be filed under the stale time
  the Pi booted with.

Kept forever: at a few messages a second it is a few MB a day, and the SSD holds 500 GB.

Reports on `health/recorder`, and under `recorder/`: `greptime` — `ok`, or why it cannot write
— `clock` and `written`.
"""

import math
import os
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import rig_mqtt

SERVICE = "recorder"
GREPTIME = "http://127.0.0.1:4000"
DATABASE = "rig"
# Short enough that a power cut costs only seconds, long enough that each write carries many
# messages rather than one.
FLUSH_S = 5
# What is held while Greptime cannot be reached: hours of the rig at a few messages a second,
# in a few tens of MB. Beyond it the oldest go, rather than the Pi's memory.
BACKLOG_MAX = 200_000
# timesyncd creates this once it has synchronised the clock.
SYNCHRONIZED = "/run/systemd/timesync/synchronized"


def log(message):
    print(f"{SERVICE}: {message}", file=sys.stderr, flush=True)


# ── Line protocol ────────────────────────────────────────────────────────────


def escape_key(text):
    """For a table or tag name, and a tag value: commas, spaces and equals signs are syntax."""
    return text.replace("\\", "\\\\").replace(",", "\\,").replace(" ", "\\ ").replace("=", "\\=")


def escape_string(text):
    """For a quoted string field. A newline would end the line, so it is written as `\\n`."""
    return text.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n")


def field(payload):
    if payload in ("true", "false"):
        return f"value={1.0 if payload == 'true' else 0.0}"
    try:
        number = float(payload)
    except ValueError:
        number = None
    # `nan` and `inf` parse as floats, but are words someone published, not readings.
    if number is not None and math.isfinite(number):
        return f"value={number!r}"
    return f'text="{escape_string(payload)}"'


def line(topic, payload, at_ms):
    """One message as a line of InfluxDB line protocol, or None for one with no table name."""
    table, _, rest = topic.partition("/")
    if not table:
        return None
    tag = f",topic={escape_key(rest)}" if rest else ""
    return f"{escape_key(table)}{tag} {field(payload)} {at_ms}"


# ── Greptime ─────────────────────────────────────────────────────────────────


class Unwritable(Exception):
    """Greptime answered, and refused the data: retrying the same batch would never succeed."""


def post(path, query, body, content_type):
    request = urllib.request.Request(
        f"{GREPTIME}{path}?{urllib.parse.urlencode(query)}",
        data=body.encode("utf-8"),
        headers={"Content-Type": content_type},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=10) as response:
            response.read()
    except urllib.error.HTTPError as error:
        detail = error.read().decode("utf-8", "replace").strip()
        if 400 <= error.code < 500:
            raise Unwritable(f"HTTP {error.code}: {detail}") from error
        raise


def create_database():
    """Without a time-to-live: the recordings are kept for good."""
    post("/v1/sql", {}, urllib.parse.urlencode({"sql": f"CREATE DATABASE IF NOT EXISTS {DATABASE}"}),
         "application/x-www-form-urlencoded")


def write(lines):
    post("/v1/influxdb/write", {"db": DATABASE, "precision": "ms"}, "\n".join(lines), "text/plain")


# ── Recorder ─────────────────────────────────────────────────────────────────


class Recorder:
    def __init__(self):
        self.lock = threading.Lock()
        # Lines received since the last flush, filled by paho's network thread.
        self.pending = []
        self.synchronized = False

    def on_message(self, message):
        if message.retain or not self.synchronized:
            return
        payload = message.payload.decode("utf-8", "replace")
        if payload == "":
            return
        recorded = line(message.topic, payload, int(time.time() * 1000))
        if recorded is None:
            return
        with self.lock:
            self.pending.append(recorded)

    def take(self):
        with self.lock:
            taken, self.pending = self.pending, []
        return taken


def main():
    telemetry = rig_mqtt.Telemetry.connect(SERVICE)
    recorder = Recorder()
    telemetry.subscribe("#", recorder.on_message)

    backlog = []
    written = 0
    created = False
    status = None
    telemetry.publish("clock", "waiting", retain=True)

    while True:
        time.sleep(FLUSH_S)

        if not recorder.synchronized and os.path.exists(SYNCHRONIZED):
            recorder.synchronized = True
            telemetry.publish("clock", "synchronized", retain=True)
            log("clock synchronized, recording")

        backlog.extend(recorder.take())
        if len(backlog) > BACKLOG_MAX:
            dropped = len(backlog) - BACKLOG_MAX
            del backlog[:dropped]
            log(f"Greptime unreachable too long: dropped the oldest {dropped} messages")

        try:
            if not created:
                # Apart from the write below: a database Greptime will not create is no reason
                # to drop the messages waiting for it.
                try:
                    create_database()
                    created = True
                except Unwritable as error:
                    raise OSError(f"cannot create database {DATABASE}: {error}") from error
            if backlog:
                write(backlog)
                written += len(backlog)
                backlog = []
            now = "ok"
        except Unwritable as error:
            # Greptime will refuse this batch however often it is sent, and holding it would
            # block everything behind it.
            log(f"Greptime refused {len(backlog)} messages, dropped: {error}")
            backlog = []
            now = str(error)
        # URLError first: it is an OSError too, and the one that means Greptime did not answer.
        except urllib.error.URLError as error:
            now = f"unreachable: {error.reason}"
        except OSError as error:
            now = str(error)

        if now != status:
            status = now
            telemetry.publish("greptime", status, retain=True)
            log(f"Greptime: {status}")
        telemetry.publish("written", written, retain=True)


if __name__ == "__main__":
    main()
