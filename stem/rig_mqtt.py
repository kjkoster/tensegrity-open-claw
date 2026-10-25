"""The rig's MQTT conventions: one connection, one health topic, one will.

Lives with Stem because Stem owns the broker, and is imported by every Python daemon on the
rig rather than reimplemented in each — a liveness convention that two processes implement
slightly differently is worse than none, because the disagreement only shows up in the
telemetry you went looking at during a failure.

## health/<service>

Every client publishes its own liveness, retained, as one of three words:

    connected     attached now
    disconnected  left deliberately — a stop, a restart, a deploy
    gone          the broker stopped hearing from it

The three are distinguishable only because the first two are published by the client and the
third is published by the *broker*, from the will it was handed at connect time. That is the
whole trick: a process cannot announce its own crash, so it hands the announcement over in
advance and the broker makes it when the connection dies without a goodbye. `disconnected`
and `gone` are therefore a real distinction and not a cosmetic one — the first is somebody
running `systemctl stop`, the second is a segfault, an OOM kill, or a Pi that lost power.

Retained on all three, deliberately. The question a health topic answers is asked by whoever
subscribes *after* the interesting moment, and an unretained tree is empty exactly then.

No leading slash: `health/stem`, not `/health/stem`. A leading slash is legal MQTT but
creates an anonymous empty first level, and `health/#` then matches nothing.

## sys/<owner> and dmx/<owner>

Facts the whole rig is read by go under a rig-wide root with the owner beneath it, retained:
what a process runs on — board, cores, memory — under `sys/`, and where it sits on the
universe under `dmx/`. `sys/#` and `dmx/#` then read the whole rig in one subscription each,
the way `health/#` reads its liveness.

## Everything else

Telemetry publishes under the service's own name — `stem/temperature_c`, `brain/show` — so the
tree says which process asserted a thing, and a dead process's topics stop moving instead of
being quietly taken over.

One field per topic, never a JSON document. A payload that has to be parsed before it can be
read is a payload no browser, `mosquitto_sub` or dashboard can chart, and the structure it
carries is structure MQTT already has: `stem/temperature_c` is the whole path and the
whole value. Nested values expand into nested topics, so a dict becomes a subtree and nothing
downstream has to know it was ever a dict.
"""

import os
import signal
import sys

HOST = os.environ.get("RIG_MQTT_HOST", "127.0.0.1")
PORT = int(os.environ.get("RIG_MQTT_PORT", "1883"))
# The same value as `stem::KEEPALIVE_S` on the Rust clients.
KEEPALIVE_S = 10
# The longest wait between reconnect attempts, matching the brain's. paho's own default is two
# minutes, which is two minutes of a broker being back and Stem still reading as gone.
RETRY_MAX_S = 30

CONNECTED = "connected"
DISCONNECTED = "disconnected"
GONE = "gone"


def health_topic(service):
    return f"health/{service}"


def sys_topic(service):
    """Where what `service` runs on is published: `sys/stem/board`."""
    return f"sys/{service}"


def flatten(topic, value):
    """Expands one value into `(topic, payload)` pairs, one per scalar field.

    Dicts and lists become subtrees, so structure lives in the topic path where a subscriber
    can filter on it, rather than inside a payload it would have to parse first.

    A `None` publishes an empty payload, which is MQTT's way of *deleting* a retained topic
    rather than storing the word "None" in it. A reading the machine cannot supply — no
    `vcgencmd`, no thermal zone — should leave no value behind, because a stale one is worse
    than an absent one.
    """
    if isinstance(value, dict):
        for key, inner in value.items():
            yield from flatten(f"{topic}/{key}", inner)
    elif isinstance(value, (list, tuple)):
        for index, inner in enumerate(value):
            yield from flatten(f"{topic}/{index}", inner)
    elif value is None:
        yield topic, ""
    elif isinstance(value, bool):
        # Before the general case, or `str(True)` capitalises it into something no consumer
        # parses back to a boolean.
        yield topic, "true" if value else "false"
    else:
        yield topic, str(value)


class Telemetry:
    """A client, its service name, and the health contract above.

    Always usable, even with no broker and no paho installed: publishing then does nothing.
    Telemetry is observability, and a daemon that refuses to run because nobody is listening
    has turned its reporting into a dependency of the thing it reports on.
    """

    def __init__(self, service, client=None):
        self.service = service
        self.client = client
        self.closed = False
        # Every retained topic this service has published, replayed on each connect.
        #
        # Retained state is a promise that a late subscriber sees the live picture, and there
        # are three ways to break it with one client: publish before the CONNACK lands and a
        # QoS 0 message is dropped rather than queued; restart the broker and its retained set
        # goes with it; reconnect after a network blip and nothing re-asserts. Replaying from
        # the connect callback answers all three, and it is the same mechanism `connected`
        # already needed.
        self.retained = {}

    @classmethod
    def connect(cls, service, host=HOST, port=PORT):
        try:
            import paho.mqtt.client as mqtt
        except ImportError:
            print(f"{service}: no paho-mqtt installed, telemetry disabled", file=sys.stderr, flush=True)
            return cls(service)

        # requirements.txt pins paho 2.x, so the current callback API and nothing else.
        client = mqtt.Client(mqtt.CallbackAPIVersion.VERSION2, client_id=service)

        telemetry = cls(service, client)
        # Before connect, not after: the will has to reach the broker in the CONNECT packet,
        # and a will set afterwards protects only the next connection.
        client.will_set(health_topic(service), GONE, retain=True)
        client.on_connect = telemetry.on_connect
        client.on_disconnect = telemetry.on_disconnect
        client.reconnect_delay_set(min_delay=1, max_delay=RETRY_MAX_S)

        # Asynchronous, so a broker that is not up yet is a wait rather than a verdict. The
        # synchronous form raises on a refused connection, and a daemon that gave up there
        # would stay silent for its whole run over a few seconds of startup ordering — which
        # is exactly the failure telemetry is supposed to report on.
        client.connect_async(host, port, keepalive=KEEPALIVE_S)
        client.loop_start()
        print(f"{service}: telemetry to {host}:{port}", file=sys.stderr, flush=True)

        # systemd stops daemons with SIGTERM, whose default disposition ends the process
        # without a word — so every ordinary restart would read as `gone`. Catching it is what
        # makes `disconnected` mean anything at all.
        telemetry.arm_shutdown()
        return telemetry

    def on_connect(self, client, _userdata, _flags, reason, _properties):
        # paho calls this for a refusal too. Announcing `connected` on a connection the broker
        # turned down would be a lie the broker itself then never corrects.
        if reason.is_failure:
            print(f"{self.service}: broker refused the connection: {reason}",
                  file=sys.stderr, flush=True)
            return
        print(f"{self.service}: connected", file=sys.stderr, flush=True)
        client.publish(health_topic(self.service), CONNECTED, retain=True)
        for topic, payload in self.retained.items():
            client.publish(topic, payload, retain=True)

    def on_disconnect(self, _client, _userdata, _flags, reason, _properties):
        # Success is our own goodbye, which `close` already speaks for.
        if reason.is_failure:
            print(f"{self.service}: lost the broker: {reason}, reconnecting",
                  file=sys.stderr, flush=True)

    def arm_shutdown(self):
        # Signals only, and deliberately no `atexit`. An uncaught exception also ends the
        # interpreter through atexit, and saying `disconnected` there would report a crash as
        # a deliberate stop — the one confusion this contract exists to prevent. A crash must
        # leave the connection to die unannounced, so the broker's `gone` is what is heard.
        for received in (signal.SIGTERM, signal.SIGINT):
            signal.signal(received, self.on_signal)

    def on_signal(self, *_):
        self.close()
        # `os._exit`, not `sys.exit`. Raising SystemExit here starts an orderly interpreter
        # shutdown while daemon threads are parked inside C calls — a blocking frame decode, an
        # HTTP handler, a socket read — and tearing those down mid-call aborts the process with
        # `FATAL: exception not rethrown`, which systemd then records as a failed unit for what
        # was an ordinary `systemctl stop`.
        #
        # Nothing is lost by leaving abruptly: the goodbye above has already been published and
        # waited for, and every log line is written unbuffered.
        os._exit(0)

    def publish(self, topic, value, retain=False):
        """Publishes under the service's own prefix, one topic per scalar field."""
        self._publish(f"{self.service}/{topic}", value, retain)

    def publish_sys(self, value):
        """Publishes what this process runs on, retained, under `sys/<service>/`."""
        self._publish(sys_topic(self.service), value, retain=True)

    def _publish(self, prefix, value, retain):
        if self.client is None:
            return
        for full, payload in flatten(prefix, value):
            if retain:
                self.retained[full] = payload
            self.client.publish(full, payload, retain=retain)

    def close(self):
        """Says goodbye deliberately, so the will is never invoked for an orderly stop."""
        if self.closed or self.client is None:
            return
        self.closed = True
        message = self.client.publish(health_topic(self.service), DISCONNECTED, retain=True)
        # Waited on rather than fired and forgotten: the disconnect below races the publish,
        # and losing that race is exactly the case this whole contract exists to distinguish.
        # The timeout keyword postdates paho 1.5, and a goodbye is not worth failing an exit
        # over on an older one.
        try:
            message.wait_for_publish(timeout=2)
        except (TypeError, ValueError, RuntimeError):
            pass
        self.client.disconnect()
        self.client.loop_stop()
