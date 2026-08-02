#!/usr/bin/env python3
"""The vision sidecar: camera in, landmarks out, annotated preview for humans.

Separate from `brain` on purpose. The show side must survive this process crashing, stalling
on a decode, or being restarted mid-tune, and a rig that aims moving heads at children cannot
have vision sharing an address space with the DMX loop.

Three outputs, three audiences. Landmarks go to `brain` over UDP, because a late pose frame is
worthless and dropping it beats delivering it in order. The annotated preview goes to a
browser over HTTP, because that is what a human uses to aim a camera and calibrate a crop.
Health goes to MQTT, where the rest of the rig's telemetry already lives.

Only OpenCV and numpy are required. The TFLite runtime and the MQTT client are both optional:
without them the daemon still runs, still draws, and still feeds `brain`, which is what lets
the runtime question stay open while the plumbing gets finished.
"""

import json
import os
import socket
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import cv2
import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import rig_mqtt

# ── Configuration ────────────────────────────────────────────────────────────
# Environment rather than constants, because the camera URL carries credentials and must not
# live in the repository, and because the crop is retuned on site against the live preview.
# Everything else has a working default so the daemon starts with one variable set.

# No default. A daemon that quietly falls back to a local video device when nobody told it
# which camera to watch spends its life retrying something that was never going to work, and
# says so in a log line that reads like a camera fault rather than a missing configuration.
CAMERA = os.environ.get("EYEBALL_CAMERA")

# sysexits.h EX_CONFIG. Named in the unit's RestartPreventExitStatus, so a misconfigured daemon
# stops with one legible error rather than restarting into the same one every five seconds.
EXIT_CONFIG = 78
# x,y,w,h as fractions of the frame: the stool box. The estimator sees only this region, which
# is what keeps other children out of the pose entirely rather than filtered out afterwards.
CROP = tuple(float(v) for v in os.environ.get("EYEBALL_CROP", "0,0,1,1").split(","))

BRAIN_HOST = os.environ.get("EYEBALL_BRAIN_HOST", "127.0.0.1")
BRAIN_PORT = int(os.environ.get("EYEBALL_BRAIN_PORT", "9001"))

HTTP_PORT = int(os.environ.get("EYEBALL_HTTP_PORT", "8080"))
PREVIEW_QUALITY = int(os.environ.get("EYEBALL_PREVIEW_QUALITY", "70"))
# The preview is downscaled before it is drawn on and encoded. It is watched by a person, on a
# laptop, over a link that may be 4G — none of which wants sensor resolution — and every pixel
# above this is one the Pi draws and JPEG-encodes for nothing.
PREVIEW_MAX_WIDTH = int(os.environ.get("EYEBALL_PREVIEW_WIDTH", "640"))
# How long after the last request the daemon keeps drawing. Nothing is annotated or encoded
# unless somebody is looking: the preview is the single most expensive thing here per frame, and
# for almost all of the show's life nobody has the page open.
PREVIEW_IDLE_S = 10.0
# How long a single-frame request waits for a frame newer than itself, so a page opened after a
# quiet spell does not serve whatever was last drawn whenever somebody last looked.
PREVIEW_WAIT_S = 1.0

SERVICE = "eyeball"
TELEMETRY_INTERVAL_S = 5.0

# The camera's own parameter tree, over VAPIX. Split by how often the answers change: the model
# and its capabilities are written once, while the settings that drift — day/night having
# flipped, an exposure someone changed in the web UI — are re-read. Both retained, because both
# are asked about after the fact.
CAMERA_STATIC_GROUPS = ("Brand", "Properties")
CAMERA_LIVE_GROUPS = ("ImageSource", "Image", "Light")
# Slow, and on its own thread. This is an HTTP round trip to a 2016 MIPS camera that is also
# encoding video, and nothing here is worth a frame. Thirty seconds is far faster than any of
# these answers actually move — the IR-cut filter flips once at dusk — and only what changed
# is published, so the steady-state cost on the bus is nothing at all.
CAMERA_POLL_S = float(os.environ.get("EYEBALL_CAMERA_POLL_S", "30"))
CAMERA_HTTP_TIMEOUT_S = 5.0
CAMERA_CONNECT_TIMEOUT_S = 2.0

MODEL = os.environ.get("EYEBALL_MODEL", "/usr/local/share/eyeball/movenet_lightning_int8.tflite")
# One core is reserved for the DMX loop, so the model runtime gets the other three. Asking for
# four would put inference on the core whose jitter shows up as visible stutter in slow moves.
MODEL_THREADS = 3

# Reconnect backoff for a camera that is unplugged, rebooting, or not yet on the link.
RETRY_MAX_S = 30


def log(message):
    print(f"eyeball: {message}", file=sys.stderr, flush=True)


# ── Estimators ───────────────────────────────────────────────────────────────
# An estimator takes the cropped BGR frame and returns a dict of name → (x, y, confidence)
# normalised within that crop, or None when it found nobody. The key set is the estimator's
# own; the receiver logs what it is given rather than insisting on a fixed skeleton, because
# the estimator is expected to be replaced.

MOVENET_KEYPOINTS = [
    "nose", "left_eye", "right_eye", "left_ear", "right_ear",
    "left_shoulder", "right_shoulder", "left_elbow", "right_elbow",
    "left_wrist", "right_wrist", "left_hip", "right_hip",
    "left_knee", "right_knee", "left_ankle", "right_ankle",
]

MOVENET_BONES = [
    ("left_shoulder", "right_shoulder"), ("left_shoulder", "left_elbow"),
    ("left_elbow", "left_wrist"), ("right_shoulder", "right_elbow"),
    ("right_elbow", "right_wrist"), ("left_shoulder", "left_hip"),
    ("right_shoulder", "right_hip"), ("left_hip", "right_hip"),
    ("left_hip", "left_knee"), ("left_knee", "left_ankle"),
    ("right_hip", "right_knee"), ("right_knee", "right_ankle"),
]

MOVENET_MIN_CONFIDENCE = 0.3


class MoveNet:
    """Single-pose TFLite inference. Fast, and the only estimator that labels limbs."""

    name = "movenet"
    bones = MOVENET_BONES

    def __init__(self, interpreter):
        self.interpreter = interpreter
        self.input = interpreter.get_input_details()[0]
        self.output = interpreter.get_output_details()[0]
        _, self.height, self.width, _ = self.input["shape"]

    @staticmethod
    def load():
        """Returns a MoveNet, or None if either the runtime or the model is missing."""
        interpreter_class = None
        for module, attribute in (
            ("ai_edge_litert.interpreter", "Interpreter"),
            ("tflite_runtime.interpreter", "Interpreter"),
            ("tensorflow.lite", "Interpreter"),
        ):
            try:
                interpreter_class = getattr(__import__(module, fromlist=[attribute]), attribute)
                break
            except (ImportError, AttributeError):
                continue
        # Both misses are spelled out rather than noted. The skeleton is what the preview is
        # for — it is the picture that explains the rig to somebody being shown it — so falling
        # back to the silhouette is a degraded mode worth a paragraph, not a shrug.
        if interpreter_class is None:
            log("no TFLite runtime — falling back to the silhouette estimator, no skeleton")
            log("  to get one:  sudo /opt/eyeball/venv/bin/pip install ai-edge-litert")
            log("  or add ai-edge-litert to eyeball/requirements.txt and deploy")
            return None
        if not os.path.exists(MODEL):
            log(f"no model at {MODEL} — falling back to the silhouette estimator, no skeleton")
            log("  MoveNet SinglePose Lightning, int8 tflite, put at the path above")
            log("  EYEBALL_MODEL moves it elsewhere")
            return None

        interpreter = interpreter_class(model_path=MODEL, num_threads=MODEL_THREADS)
        interpreter.allocate_tensors()
        return MoveNet(interpreter)

    def __call__(self, frame):
        resized = cv2.resize(frame, (self.width, self.height), interpolation=cv2.INTER_AREA)
        tensor = cv2.cvtColor(resized, cv2.COLOR_BGR2RGB)
        tensor = np.expand_dims(tensor, axis=0).astype(self.input["dtype"])

        self.interpreter.set_tensor(self.input["index"], tensor)
        self.interpreter.invoke()
        # MoveNet returns (1, 1, 17, 3) as y, x, confidence — y before x.
        raw = self.interpreter.get_tensor(self.output["index"])[0][0]

        keypoints = {
            name: (float(x), float(y), float(confidence))
            for name, (y, x, confidence) in zip(MOVENET_KEYPOINTS, raw)
        }
        best = max(confidence for _, _, confidence in keypoints.values())
        return keypoints if best >= MOVENET_MIN_CONFIDENCE else None


class Silhouette:
    """Background subtraction and contour extremities. No model, no runtime, no wheel.

    It reports where a body is and where its extremities are, and it cannot say which arm is
    which — that is the price of having no model. It exists so the daemon always has a working
    estimator, and because the night show's IR path points this way regardless.
    """

    name = "silhouette"
    bones = [("centroid", "head"), ("centroid", "left_tip"), ("centroid", "right_tip")]

    # A long history and a high variance threshold: the mage stands nearly still for whole
    # turns, and a fast-adapting model would learn them into the background and lose them.
    HISTORY = 500
    VAR_THRESHOLD = 32
    MIN_AREA_FRACTION = 0.01

    def __init__(self):
        self.subtractor = cv2.createBackgroundSubtractorMOG2(
            history=self.HISTORY, varThreshold=self.VAR_THRESHOLD, detectShadows=False
        )
        self.kernel = cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (5, 5))

    def __call__(self, frame):
        height, width = frame.shape[:2]
        mask = self.subtractor.apply(frame)
        mask = cv2.morphologyEx(mask, cv2.MORPH_OPEN, self.kernel)
        mask = cv2.morphologyEx(mask, cv2.MORPH_CLOSE, self.kernel, iterations=2)

        contours, _ = cv2.findContours(mask, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)
        if not contours:
            return None
        blob = max(contours, key=cv2.contourArea)
        if cv2.contourArea(blob) < self.MIN_AREA_FRACTION * width * height:
            return None

        moments = cv2.moments(blob)
        if moments["m00"] == 0:
            return None
        centre = np.array([moments["m10"] / moments["m00"], moments["m01"] / moments["m00"]])

        points = blob.reshape(-1, 2).astype(float)
        # Extremities, not fingertips: the farthest hull point on each side of the centroid is
        # a hand when an arm is out and a shoulder when it is not, which is the honest limit of
        # what a silhouette knows.
        head = points[np.argmin(points[:, 1])]
        left = points[points[:, 0] < centre[0]]
        right = points[points[:, 0] >= centre[0]]

        def farthest(candidates, fallback):
            if len(candidates) == 0:
                return fallback
            return candidates[np.argmax(np.linalg.norm(candidates - centre, axis=1))]

        found = {
            "centroid": centre,
            "head": head,
            "left_tip": farthest(left, centre),
            "right_tip": farthest(right, centre),
        }
        return {
            name: (float(point[0] / width), float(point[1] / height), 1.0)
            for name, point in found.items()
        }


# ── Latest-frame slot ────────────────────────────────────────────────────────


class Latest:
    """One slot, last writer wins. The reader takes whatever is newest and never blocks."""

    def __init__(self, value=None):
        self.lock = threading.Lock()
        self.value = value

    def put(self, value):
        with self.lock:
            self.value = value

    def get(self):
        with self.lock:
            return self.value


# ── Capture ──────────────────────────────────────────────────────────────────


def open_camera():
    """Opens the camera, low-latency, retrying until it answers."""
    # RTSP over TCP: UDP loses slices on a busy link and OpenCV renders the tearing rather
    # than dropping the frame, which reads as a vision bug rather than a network one.
    os.environ.setdefault("OPENCV_FFMPEG_CAPTURE_OPTIONS", "rtsp_transport;tcp")

    source = int(CAMERA) if CAMERA.isdigit() else CAMERA
    backoff_s = 1
    while True:
        capture = cv2.VideoCapture(source)
        if capture.isOpened():
            # Default buffering costs hundreds of milliseconds, and the mechanical lag of a
            # moving head has already spent the latency budget.
            capture.set(cv2.CAP_PROP_BUFFERSIZE, 1)
            log(f"camera open: {CAMERA}")
            return capture
        capture.release()
        log(f"camera did not open, retrying in {backoff_s}s")
        time.sleep(backoff_s)
        backoff_s = min(backoff_s * 2, RETRY_MAX_S)


def capture_thread(frames, stats):
    """Reads as fast as the camera delivers and keeps only the newest frame.

    Decode runs here rather than in the estimator loop so that inference falling behind costs
    frames rather than latency: the estimator always picks up what the camera sent last, not
    the head of a queue that grew while it was busy.
    """
    while True:
        camera = open_camera()
        while True:
            ok, frame = camera.read()
            if not ok:
                log("camera read failed, reopening")
                break
            frames.put(frame)
            stats["captured"] += 1
        camera.release()


# ── Annotation ───────────────────────────────────────────────────────────────

COLOUR_BONE = (80, 255, 255)
COLOUR_POINT = (60, 180, 255)
COLOUR_CROP = (0, 220, 0)
COLOUR_TEXT = (255, 255, 255)
# Everything is drawn twice, this colour underneath and one size wider. A skeleton in a single
# colour disappears wherever the image happens to match it, and this preview is shown to
# children against whatever a field looks like that afternoon — the outline is what makes it
# read on a bright shirt and on dark grass alike.
COLOUR_OUTLINE = (16, 16, 16)

# One definition each, because the status text is measured with one call and drawn with
# another: a font or scale that differed between them would size the backing box wrongly, and
# the mismatch would look like the text had moved rather than like the box was wrong.
FONT = cv2.FONT_HERSHEY_SIMPLEX
FONT_SCALE = 0.5
FONT_THICKNESS = 1
TEXT_LINE_HEIGHT = 18
TEXT_PADDING = 4


def downscale(frame):
    """Shrinks to the preview width, or returns the frame untouched if it is already small."""
    height, width = frame.shape[:2]
    if width <= PREVIEW_MAX_WIDTH:
        return frame
    scale = PREVIEW_MAX_WIDTH / width
    return cv2.resize(
        frame, (PREVIEW_MAX_WIDTH, max(1, int(height * scale))), interpolation=cv2.INTER_AREA
    )


def annotate(frame, stool, keypoints, estimator, status):
    """Draws the crop, the skeleton and the running numbers onto a copy of the frame.

    The skeleton is the point of this picture, not a debugging overlay on it — it is what makes
    the rig legible to somebody being shown how it works. So it is drawn to be seen from across
    a field on a laptop screen: outlined, thick enough to survive JPEG, and joints on top of
    bones so a limb reads as jointed rather than as a bent stick.
    """
    canvas = frame.copy()
    x, y, w, h = stool
    cv2.rectangle(canvas, (x, y), (x + w, y + h), COLOUR_CROP, 1)

    if keypoints:
        # Keypoints are normalised within the crop, so the crop origin puts them back into the
        # picture — the preview shows where the mage is in the room, not in the tensor.
        placed = {
            name: (int(x + px * w), int(y + py * h))
            for name, (px, py, confidence) in keypoints.items()
            if confidence >= MOVENET_MIN_CONFIDENCE
        }
        bones = [
            (placed[first], placed[second])
            for first, second in estimator.bones
            if first in placed and second in placed
        ]
        # Every outline first, then every fill: drawing each bone's outline immediately before
        # its own fill would let the next bone's outline cut a dark notch through the last
        # bone's body wherever two limbs cross.
        for start, end in bones:
            cv2.line(canvas, start, end, COLOUR_OUTLINE, 6, cv2.LINE_AA)
        for start, end in bones:
            cv2.line(canvas, start, end, COLOUR_BONE, 3, cv2.LINE_AA)
        for point in placed.values():
            cv2.circle(canvas, point, 6, COLOUR_OUTLINE, -1, cv2.LINE_AA)
        for point in placed.values():
            cv2.circle(canvas, point, 4, COLOUR_POINT, -1, cv2.LINE_AA)

    # A filled box behind the text rather than an outline around it. Drawing the same string
    # twice at two thicknesses does not produce concentric glyphs — the thick pass sits beside
    # the thin one rather than under it — and a box needs no such alignment to hold.
    for line, text in enumerate(status):
        origin = (8, 20 + line * TEXT_LINE_HEIGHT)
        (width, height), baseline = cv2.getTextSize(text, FONT, FONT_SCALE, FONT_THICKNESS)
        cv2.rectangle(
            canvas,
            (origin[0] - TEXT_PADDING, origin[1] - height - TEXT_PADDING),
            (origin[0] + width + TEXT_PADDING, origin[1] + baseline),
            COLOUR_OUTLINE,
            -1,
        )
        cv2.putText(canvas, text, origin, FONT, FONT_SCALE,
                    COLOUR_TEXT, FONT_THICKNESS, cv2.LINE_AA)
    return canvas


def encode(frame):
    ok, buffer = cv2.imencode(".jpg", frame, [cv2.IMWRITE_JPEG_QUALITY, PREVIEW_QUALITY])
    return buffer.tobytes() if ok else None


# ── HTTP preview ─────────────────────────────────────────────────────────────

PAGE = b"""<!doctype html>
<title>eyeball</title>
<style>body{background:#111;color:#ccc;font:14px system-ui;margin:0;padding:1rem}
img{max-width:100%;display:block;border:1px solid #333}a{color:#6cf}</style>
<h1>eyeball</h1>
<img src="/annotated.mjpg">
<p><a href="/raw.jpg">raw frame</a> &middot;
<a href="/annotated.jpg">annotated frame</a> &middot;
<a href="/pose.json">pose</a></p>
"""


class Preview(BaseHTTPRequestHandler):
    """The human end. Single frames for aiming, a stream for watching, JSON for reading."""

    raw = None
    annotated = None
    pose = None
    # When somebody last asked for a picture. The frame loop reads it to decide whether drawing
    # and encoding are worth doing at all.
    last_request = 0.0

    protocol_version = "HTTP/1.1"

    def log_message(self, format, *args):
        """Silenced: a browser holding an MJPEG stream open would otherwise fill the journal."""

    @classmethod
    def touch(cls):
        cls.last_request = time.monotonic()

    @classmethod
    def watching(cls):
        return time.monotonic() - cls.last_request < PREVIEW_IDLE_S

    def _send(self, body, content_type):
        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def _image(self, slot):
        """Serves one frame, waiting briefly for one drawn since this request arrived.

        Without the wait, a page opened after a quiet spell would show whatever was last drawn
        whenever somebody last looked — which could be hours old and would look like a frozen
        camera rather than a preview that had been switched off.
        """
        before = slot.get()
        self.touch()
        deadline = time.monotonic() + PREVIEW_WAIT_S
        while time.monotonic() < deadline:
            current = slot.get()
            if current is not None and current is not before:
                return current
            time.sleep(0.02)
        return slot.get() or b""

    def _stream(self, slot):
        boundary = b"--eyeballframe"
        self.send_response(200)
        self.send_header("Content-Type", "multipart/x-mixed-replace; boundary=eyeballframe")
        self.send_header("Cache-Control", "no-store")
        # No length is knowable for a stream that ends when the browser leaves, and HTTP/1.1
        # keep-alive without one is a connection the client cannot find the end of.
        self.send_header("Connection", "close")
        self.end_headers()
        last = None
        try:
            while True:
                # Every pass, not only on a new frame: this is what keeps the frame loop drawing
                # for as long as a browser holds the stream open, and lets it stop within
                # PREVIEW_IDLE_S of the browser going away.
                self.touch()
                frame = slot.get()
                if frame is None or frame is last:
                    time.sleep(0.02)
                    continue
                last = frame
                self.wfile.write(
                    boundary + b"\r\nContent-Type: image/jpeg\r\nContent-Length: "
                    + str(len(frame)).encode() + b"\r\n\r\n" + frame + b"\r\n"
                )
        except (BrokenPipeError, ConnectionResetError):
            pass

    def do_GET(self):
        path = self.path.split("?")[0]
        if path == "/":
            self._send(PAGE, "text/html; charset=utf-8")
        elif path == "/raw.jpg":
            self._send(self._image(self.raw), "image/jpeg")
        elif path == "/annotated.jpg":
            self._send(self._image(self.annotated), "image/jpeg")
        elif path == "/annotated.mjpg":
            self._stream(self.annotated)
        elif path == "/pose.json":
            self._send(self.pose.get() or b"{}", "application/json")
        else:
            self.send_error(404)


# ── Telemetry ────────────────────────────────────────────────────────────────


# Connection, health topic and will all come from the shared module rather than from here.
# A liveness convention that two daemons implement slightly differently is worse than none:
# the disagreement surfaces only in the telemetry you went looking at during a failure.


def vapix_opener(camera):
    """An HTTP opener carrying the camera's credentials, if the stream URL has any.

    The credentials come out of the stream URL rather than from variables of their own: they
    are the same account, and a second copy in the environment is a second thing to rotate and
    a second thing to get wrong.

    Without credentials it still returns an opener rather than nothing. Anonymous viewer access
    is a setting some cameras have on, and an attempt that comes back 401 is a logged answer,
    where declining to ask is a silence indistinguishable from everything working.
    """
    if not camera.username:
        return urllib.request.build_opener()
    passwords = urllib.request.HTTPPasswordMgrWithDefaultRealm()
    passwords.add_password(
        None, f"http://{camera.hostname}/", camera.username, camera.password or ""
    )
    # Both handlers: Axis defaults to digest, but can be configured to accept basic, and which
    # one is in force is a setting on the camera rather than a property of the API.
    return urllib.request.build_opener(
        urllib.request.HTTPDigestAuthHandler(passwords),
        urllib.request.HTTPBasicAuthHandler(passwords),
    )


def vapix_parameters(opener, host, groups):
    """Reads VAPIX parameter groups into `{dotted.key: value}`.

    The response is `root.Brand.ProdNbr=P3367-VE` per line — already a hierarchy, which is why
    this needs no parsing beyond a split and why the result maps onto MQTT by swapping dots for
    slashes. A group that fails is skipped rather than aborting the rest: an older firmware not
    having `Light` should cost that one subtree, not the whole inventory.
    """
    found = {}
    for group in groups:
        url = f"http://{host}/axis-cgi/param.cgi?action=list&group={group}"
        try:
            with opener.open(url, timeout=CAMERA_HTTP_TIMEOUT_S) as response:
                body = response.read().decode("utf-8", "replace")
        except (urllib.error.URLError, OSError) as e:
            log(f"camera parameters: {group} unavailable ({e})")
            continue
        for line in body.splitlines():
            key, separator, value = line.partition("=")
            if separator and key.startswith("root."):
                found[key[len("root."):]] = value
    return found


def link_state(camera):
    """Whether the camera answers a TCP connect on its stream port, and how fast.

    A connect rather than an ICMP ping: it needs no privileges and no subprocess, and it asks a
    better question — a camera whose network stack is up while its streaming server has died
    still answers a ping. The socket is closed immediately; nothing here speaks the protocol.

    This is the daemon's own observation rather than the camera's, which is why it lands under
    `camera/link/` and not beside the parameter tree. Three questions, three answers, coarse to
    fine: `link/reachable` says the box is there, the parameter tree says how it is configured,
    and `status/fps` says frames are actually arriving.
    """
    port = camera.port or (554 if camera.scheme == "rtsp" else 80)
    started = time.monotonic()
    try:
        socket.create_connection(
            (camera.hostname, port), timeout=CAMERA_CONNECT_TIMEOUT_S
        ).close()
    except OSError:
        return {"address": camera.hostname, "port": port, "reachable": False, "latency_ms": None}
    return {
        "address": camera.hostname,
        "port": port,
        "reachable": True,
        "latency_ms": round((time.monotonic() - started) * 1000, 1),
    }


def publish_parameters(telemetry, parameters, published):
    """Publishes the parameters that changed, and remembers them.

    Only the changes, because almost none of this moves: republishing a hundred identical
    retained values every poll churns the broker's persistence for nothing, and — worse — makes
    every topic look like it just updated, so the one that genuinely did is invisible. A
    retained value the broker already holds needs no refreshing, and a broker that restarts gets
    the whole set replayed by the reconnect path rather than by this loop.
    """
    for key, value in parameters.items():
        if published.get(key) == value:
            continue
        published[key] = value
        telemetry.publish(f"camera/{key.replace('.', '/')}", value, retain=True)


def camera_thread(telemetry, camera):
    """Publishes what the camera says about itself, on its own thread.

    Its own, because these are HTTP round trips to a 2016 MIPS camera that is simultaneously
    encoding the video this daemon is reading, and the frame loop must never wait on that.
    """
    if not camera.hostname:
        log("camera is not a network address, no parameter tree to read")
        return
    opener = vapix_opener(camera)

    published = {}
    announced = False
    was_reachable = None

    while True:
        # Published every poll rather than only on change: latency is a live measurement, and a
        # link that is degrading says so in the numbers before it says so by failing.
        link = link_state(camera)
        telemetry.publish("camera/link", link, retain=True)
        if link["reachable"] != was_reachable:
            was_reachable = link["reachable"]
            answers = "answers" if was_reachable else "does not answer"
            log(f"camera {link['address']}:{link['port']} {answers}")

        # Retried until it answers rather than attempted once: a camera still booting, or one
        # plugged in after the daemon, should still end up describing itself.
        if not announced:
            static = vapix_parameters(opener, camera.hostname, CAMERA_STATIC_GROUPS)
            if static:
                publish_parameters(telemetry, static, published)
                announced = True
                log(f"camera is {static.get('Brand.ProdFullName', 'an unidentified model')}")

        live = vapix_parameters(opener, camera.hostname, CAMERA_LIVE_GROUPS)
        publish_parameters(telemetry, live, published)
        time.sleep(CAMERA_POLL_S)


# ── Main loop ────────────────────────────────────────────────────────────────


def crop_box(frame):
    """The stool box in pixels, clamped to the frame so a bad config cannot slice to nothing."""
    height, width = frame.shape[:2]
    fx, fy, fw, fh = CROP
    x = max(0, min(int(fx * width), width - 1))
    y = max(0, min(int(fy * height), height - 1))
    w = max(1, min(int(fw * width), width - x))
    h = max(1, min(int(fh * height), height - y))
    return x, y, w, h


def require_camera():
    """Refuses to start without a camera, and says how to give it one.

    The whole of the configuration is this one variable — the stream URL carries the address
    and the credentials that everything else derives from — so its absence is the single most
    likely reason for a daemon that runs and sees nothing. It is worth a paragraph in the
    journal rather than an exit code somebody has to go and look up.
    """
    if CAMERA:
        return
    log("EYEBALL_CAMERA is not set — refusing to start.")
    log("")
    log("The daemon needs the camera's stream URL. It is also where the address and the")
    log("credentials come from, so nothing works without it. Put it in /etc/default/eyeball:")
    log("")
    log("  EYEBALL_CAMERA=rtsp://user:password@192.168.0.90/axis-media/media.amp"
        "?videocodec=h264&resolution=640x360&fps=10")
    log("")
    log("MJPEG over HTTP is the alternative, and may decode more cheaply on this Pi:")
    log("")
    log("  EYEBALL_CAMERA=http://user:password@192.168.0.90/axis-cgi/mjpg/video.cgi"
        "?resolution=640x360&fps=10")
    log("")
    log("That file holds a password, so it wants mode 0600. A local device for desk testing")
    log("is written as a bare index, EYEBALL_CAMERA=0, which disables the parameter tree.")
    log("")
    log("Then: sudo systemctl restart eyeball")
    sys.exit(EXIT_CONFIG)


def main():
    require_camera()
    estimator = MoveNet.load() or Silhouette()
    # The camera first, and without its password. Its absence is the single most likely reason
    # for a daemon that runs but sees nothing, and `EnvironmentFile=-` means a missing
    # configuration file starts the daemon on defaults rather than failing where it would show.
    log(f"camera: {CAMERA.split('@')[-1]}")
    log(f"estimator: {estimator.name}")
    log(f"crop: {CROP}")

    frames = Latest()
    stats = {"captured": 0}
    threading.Thread(target=capture_thread, args=(frames, stats), daemon=True).start()

    Preview.raw = Latest()
    Preview.annotated = Latest()
    Preview.pose = Latest()
    server = ThreadingHTTPServer(("0.0.0.0", HTTP_PORT), Preview)
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, daemon=True).start()
    log(f"preview on http://0.0.0.0:{HTTP_PORT}/")

    telemetry = rig_mqtt.Telemetry.connect(SERVICE)
    # The camera is published without its credentials: the URL carries a password and the
    # telemetry tree is the one part of the rig anyone on the AP can read.
    telemetry.publish("identity", {
        "estimator": estimator.name,
        "camera": CAMERA.split("@")[-1],
        "crop": dict(zip(("x", "y", "w", "h"), CROP)),
        "started_at": time.time(),
    }, retain=True)

    threading.Thread(
        target=camera_thread,
        args=(telemetry, urllib.parse.urlparse(CAMERA)),
        daemon=True,
    ).start()

    to_brain = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)

    sequence = 0
    last_frame = None
    fps = 0.0
    estimate_ms = 0.0
    last_tick = time.monotonic()
    last_telemetry = 0.0
    # Where the camera's own delivery rate is measured, against which the processing rate above
    # is read: the two together say whether frames are being decoded and thrown away.
    last_captured = 0
    last_captured_at = time.monotonic()

    while True:
        frame = frames.get()
        if frame is None or frame is last_frame:
            time.sleep(0.005)
            continue
        last_frame = frame

        box = crop_box(frame)
        x, y, w, h = box
        # Timed on its own, because the alternative is arguing about whether the cost is the
        # estimator or the decode when one number separates them.
        started = time.monotonic()
        keypoints = estimator(frame[y:y + h, x:x + w])
        estimate_ms = 0.9 * estimate_ms + 0.1 * (time.monotonic() - started) * 1000

        sequence += 1
        now = time.monotonic()
        # Smoothed rather than instantaneous: the interesting question is whether the pose rate
        # holds, and a single slow frame answers nothing.
        fps = 0.9 * fps + 0.1 / max(now - last_tick, 1e-6)
        last_tick = now

        sighting = {
            "seq": sequence,
            "sent_at": time.time(),
            "source": estimator.name,
            "fps": round(fps, 2),
            "present": keypoints is not None,
            "keypoints": {
                name: [round(px, 4), round(py, 4), round(confidence, 3)]
                for name, (px, py, confidence) in (keypoints or {}).items()
            },
        }
        payload = json.dumps(sighting).encode()
        to_brain.sendto(payload, (BRAIN_HOST, BRAIN_PORT))
        Preview.pose.put(payload)

        # Drawing and encoding only while somebody is looking. This is by far the most expensive
        # thing in the loop — a full-frame copy and two JPEG encodes — and for almost all of the
        # rig's life nobody has the page open. The landmark stream and the telemetry above are
        # unaffected: the show never depended on the preview being produced.
        if Preview.watching():
            small = downscale(frame)
            # The crop box is in full-frame pixels and the preview may be smaller, so it is
            # scaled to match. Keypoints need no such treatment: they are normalised within the
            # crop and land correctly wherever the crop itself is drawn.
            ratio = small.shape[1] / frame.shape[1]
            scaled = tuple(int(value * ratio) for value in box)
            status = [
                f"{estimator.name}  {fps:4.1f} Hz  seq {sequence}",
                "mage: yes" if keypoints else "mage: no",
            ]
            Preview.raw.put(encode(small))
            Preview.annotated.put(
                encode(annotate(small, scaled, keypoints, estimator, status))
            )

        if time.time() - last_telemetry >= TELEMETRY_INTERVAL_S:
            last_telemetry = time.time()
            now = time.monotonic()
            captured_fps = (stats["captured"] - last_captured) / max(now - last_captured_at, 1e-6)
            last_captured, last_captured_at = stats["captured"], now
            telemetry.publish("status", {
                "fps": round(fps, 2),
                # What the camera is actually delivering. Below the requested rate means the
                # camera or the link is the limit; above the processing rate means frames are
                # being decoded and dropped, and the request should come down to meet it.
                "captured_fps": round(captured_fps, 2),
                # What the camera is actually sending, which is the only way to know whether it
                # honoured the resolution the URL asked for.
                "width": frame.shape[1],
                "height": frame.shape[0],
                # Splits the cost: a small number here with a busy process means the expense is
                # decode, and a large one means it is the estimator.
                "estimate_ms": round(estimate_ms, 1),
                "seq": sequence,
                "captured": stats["captured"],
                "present": keypoints is not None,
                "estimator": estimator.name,
            }, retain=True)


if __name__ == "__main__":
    main()
