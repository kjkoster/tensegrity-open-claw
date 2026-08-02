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

No leading slash: `health/eyeball`, not `/health/eyeball`. A leading slash is legal MQTT but
creates an anonymous empty first level, and `health/#` then matches nothing.

## Everything else

Telemetry publishes under the service's own name — `stem/specs`, `eyeball/status` — so the
tree says which process asserted a thing, and a dead process's topics stop moving instead of
being quietly taken over.

One field per topic, never a JSON document. A payload that has to be parsed before it can be
read is a payload no browser, `mosquitto_sub` or dashboard can chart, and the structure it
carries is structure MQTT already has: `stem/stats/temperature_c` is the whole path and the
whole value. Nested values expand into nested topics, so a dict becomes a subtree and nothing
downstream has to know it was ever a dict.
"""

import atexit
import os
import signal
import sys

HOST = os.environ.get("RIG_MQTT_HOST", "127.0.0.1")
PORT = int(os.environ.get("RIG_MQTT_PORT", "1883"))
KEEPALIVE_S = 30

CONNECTED = "connected"
DISCONNECTED = "disconnected"
GONE = "gone"


def health_topic(service):
    return f"health/{service}"


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

        # Debian's paho is 2.x, which requires a callback API version; 1.x has no such enum.
        # Asking for the version 1 callbacks on both keeps one set of signatures here.
        try:
            client = mqtt.Client(mqtt.CallbackAPIVersion.VERSION1, client_id=service)
        except AttributeError:
            client = mqtt.Client(client_id=service)

        telemetry = cls(service, client)
        # Before connect, not after: the will has to reach the broker in the CONNECT packet,
        # and a will set afterwards protects only the next connection.
        client.will_set(health_topic(service), GONE, retain=True)
        client.on_connect = telemetry.on_connect

        # Asynchronous, so a broker that is not up yet is a wait rather than a verdict. The
        # synchronous form raises on a refused connection, and a daemon that gave up there
        # would stay silent for its whole run over a few seconds of startup ordering — which
        # is exactly the failure telemetry is supposed to report on.
        client.connect_async(host, port, keepalive=KEEPALIVE_S)
        client.loop_start()
        print(f"{service}: telemetry to {host}:{port}", file=sys.stderr, flush=True)

        # systemd stops daemons with SIGTERM, whose default disposition ends the process
        # without running atexit — so every ordinary restart would publish nothing and read as
        # `gone`. Catching it is what makes `disconnected` mean anything at all.
        telemetry.arm_shutdown()
        return telemetry

    def on_connect(self, client, *_):
        client.publish(health_topic(self.service), CONNECTED, retain=True)
        for topic, payload in self.retained.items():
            client.publish(topic, payload, retain=True)

    def arm_shutdown(self):
        atexit.register(self.close)
        for received in (signal.SIGTERM, signal.SIGINT):
            signal.signal(received, self.on_signal)

    def on_signal(self, *_):
        self.close()
        sys.exit(0)

    def publish(self, topic, value, retain=False):
        """Publishes under the service's own prefix, one topic per scalar field."""
        if self.client is None:
            return
        for full, payload in flatten(f"{self.service}/{topic}", value):
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
