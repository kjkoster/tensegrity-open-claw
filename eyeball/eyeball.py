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
import math
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

# The camera is three facts: where it is and who to be. Everything else about it — which
# stream, at what size, at what rate, with the sensor locked how — is this daemon's decision to
# make and its job to enforce, not something to be spelled out in a URL somebody maintains by
# hand. A stream URL in a configuration file is a place for those decisions to drift from the
# code that depends on them.
#
# No default for the host. A daemon that quietly falls back to a local video device when nobody
# told it which camera to watch spends its life retrying something that was never going to work,
# and says so in a log line that reads like a camera fault rather than a missing configuration.
CAMERA_HOST = os.environ.get("EYEBALL_CAMERA_HOST")
CAMERA_USER = os.environ.get("EYEBALL_CAMERA_USER", "root")
CAMERA_PASSWORD = os.environ.get("EYEBALL_CAMERA_PASSWORD", "")

# What to ask the camera to send. `mjpeg` and `rtsp` are the same picture by different means —
# independent JPEGs against H.264 — and which is cheaper on this Pi is a measurement, so it is
# a setting rather than a decision baked into a URL.
CAMERA_TRANSPORT = os.environ.get("EYEBALL_CAMERA_TRANSPORT", "rtsp")
CAMERA_RESOLUTION = os.environ.get("EYEBALL_CAMERA_RESOLUTION", "640x360")
CAMERA_FPS = os.environ.get("EYEBALL_CAMERA_FPS", "10")
# Which of the camera's view areas to stream. It reports eight, each a separately croppable
# window onto the same sensor, addressed as `camera=N` on every stream URL and every PTZ call.
CAMERA_VIEW = os.environ.get("EYEBALL_CAMERA_VIEW", "1")

# What the daemon insists the camera is set to lives in `/etc/default/eyeball`, not here.
#
# Every setting is a thing the show requires and the camera will otherwise decide for itself,
# badly, from a scene the show is actively changing: an auto IR-cut filter oscillating between
# day and night as beams sweep across the mage, an auto white balance chasing a wash that has no
# green in it, an auto exposure re-metering every time a head moves.
#
# None of it is in this file, because a value here and a value on the Pi is two places to change
# one setting and one of them will lose. The values are firmware-specific, they are retuned on
# site against a live picture, and the only machine that can say whether one took is the camera
# itself — all of which describes configuration rather than code. `eyeball/eyeball.default` in
# the repository is the known-good list, installed by the deploy only when the Pi has no file of
# its own, so a fresh card comes up configured and a tuned one is never overwritten.
#
# The whole list is written on every start and read back afterwards, because a camera that
# silently clamps or ignores a setting looks exactly like one that accepted it. Nothing re-writes
# between starts: the deploy restarts this daemon whether or not it changed, so a start is the
# moment the file's contents reach the camera.

# sysexits.h EX_CONFIG. Named in the unit's RestartPreventExitStatus, so a misconfigured daemon
# stops with one legible error rather than restarting into the same one every five seconds.
EXIT_CONFIG = 78
# x,y,w,h as fractions of the frame: the stool box. The estimator sees only this region, which
# is what keeps other children out of the pose entirely rather than filtered out afterwards.
CROP = tuple(float(v) for v in os.environ.get("EYEBALL_CROP", "0,0,1,1").split(","))

# Which colour plane the estimator sees. `colour` is the whole image; naming one channel hands
# it that plane as grey instead.
#
# This is a lighting trick, not an image-processing one. The rig's own beams are narrow-band —
# a red LED puts almost nothing into the sensor's green channel, a blue one likewise — so if
# the show owns red and blue while the mage is lit by something with green in it, the green
# plane is very nearly the vision light alone, and the show's colour changes stop moving the
# picture the model is asked to read. Attenuation rather than immunity: the dye filters on a
# Bayer sensor overlap, so a bright red beam still leaks some green.
CHANNELS = {"blue": 0, "green": 1, "red": 2}
CHANNEL = CHANNELS.get(os.environ.get("EYEBALL_CHANNEL", "colour"))

# Local contrast equalisation before inference. Off by default and worth having under infrared,
# where a scene lit by one lamp at one point arrives flat and dim at its edges — the mage is
# there in the pixels, spread over forty grey levels instead of two hundred. CLAHE is local
# rather than global, so it lifts the subject without blowing out whatever the illuminator is
# closest to.
ENHANCE = os.environ.get("EYEBALL_ENHANCE", "none")
CLAHE = cv2.createCLAHE(clipLimit=2.0, tileGridSize=(8, 8))

# How long the preview holds the worst each landmark has been.
#
# The instantaneous confidence is the wrong instrument for the question the arm mappings need
# answered. An elbow that lets go for two frames of a fast sweep is back before an eye watching
# a 9 Hz preview can register that it went, and the person best placed to notice is the one
# waving their own arm about and therefore not reading the screen. A floor held for a couple of
# seconds turns a blink into something that is still there when they look.
CONFIDENCE_FLOOR_S = float(os.environ.get("EYEBALL_CONFIDENCE_FLOOR_S", "2.0"))

BRAIN_HOST = os.environ.get("EYEBALL_BRAIN_HOST", "127.0.0.1")
BRAIN_PORT = int(os.environ.get("EYEBALL_BRAIN_PORT", "9001"))

HTTP_PORT = int(os.environ.get("EYEBALL_HTTP_PORT", "8080"))
PREVIEW_QUALITY = int(os.environ.get("EYEBALL_PREVIEW_QUALITY", "70"))
# The preview is downscaled before it is drawn on and encoded. It is watched by a person, on a
# laptop, over a link that may be 4G — none of which wants sensor resolution — and both the
# drawing and the encode are charged to the pose rate, which is the show's own responsiveness.
# The ceiling is deliberately below the widths worth capturing at, so that raising the camera's
# resolution to feed the estimator never quietly raises what the preview costs alongside it.
PREVIEW_MAX_WIDTH = int(os.environ.get("EYEBALL_PREVIEW_WIDTH", "480"))
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

# Two model files, because three runtimes want two formats between them. Whichever exists is
# the one that gets used; neither existing is what leaves the daemon on the silhouette.
MODEL_TFLITE = os.environ.get("EYEBALL_MODEL_TFLITE", "/usr/local/share/eyeball/movenet.tflite")
MODEL_ONNX = os.environ.get("EYEBALL_MODEL_ONNX", "/usr/local/share/eyeball/movenet.onnx")
# MoveNet SinglePose Lightning is 192×192; Thunder is 256. Only needed for the OpenCV backend,
# which cannot be asked what shape its input is.
MODEL_INPUT = int(os.environ.get("EYEBALL_MODEL_INPUT", "192"))
# One core is reserved for the DMX loop, so the model runtime gets the other three. Asking for
# four would put inference on the core whose jitter shows up as visible stutter in slow moves.
#
# This also caps OpenCV itself, which otherwise spreads resize, colour conversion and the
# background subtractor across every core it can find — including the one the frame loop is
# supposed to have to itself.
#
# Tunable rather than fixed, because the reason for three is a judgement about DMX jitter
# rather than a measurement, and the fourth core is worth trying against a slow head move.
MODEL_THREADS = int(os.environ.get("EYEBALL_MODEL_THREADS", "3"))

# How the crop is shrunk to the model's input.
#
# Bilinear, which reverses the original choice along with the reasoning behind it. INTER_AREA is
# the right answer on a *large* downscale, and this stopped being one when the crop became the
# box the mage stands in: 270×270 into a 192×192 input is a factor of 1.4, where area-averaging
# reads barely more than two source pixels per axis and still pays for the general resampler.
# What it bought was anti-aliasing, which is the artefact a pose model is least troubled by, and
# what it cost was most of the 13–15 ms this stage takes out of a ~100 ms frame — the rest of
# that gap being the padding above, which a square crop now skips entirely.
#
# `EYEBALL_RESIZE=area` goes back, and is the setting to reach for if the crop is ever widened
# far enough to make the downscale steep again.
RESIZE = cv2.INTER_AREA if os.environ.get("EYEBALL_RESIZE") == "area" else cv2.INTER_LINEAR

# Which estimator to run. `movenet` is the intended one and the only one that draws a skeleton,
# so a missing model is a refusal to start rather than a quiet demotion — the silhouette is a
# deliberate choice, available by naming it, and never something the daemon slides into on its
# own. A show that silently lost its skeleton would look like a working rig.
ESTIMATOR = os.environ.get("EYEBALL_ESTIMATOR", "movenet")

# Reconnect backoff for a camera that is unplugged, rebooting, or not yet on the link.
RETRY_MAX_S = 30


def log(message):
    print(f"eyeball: {message}", file=sys.stderr, flush=True)


def parse_settings(text):
    """`a.b=c,d.e=f` into a dict, ignoring anything without an `=`."""
    settings = {}
    for item in text.split(","):
        key, separator, value = item.partition("=")
        if separator:
            settings[key.strip()] = value.strip()
    return settings


# Empty is a real answer rather than a missing one: `EYEBALL_CAMERA_HOST=0` opens a local video
# device for desk testing, and a webcam has no parameter tree to write to.
CAMERA_SETTINGS = parse_settings(os.environ.get("EYEBALL_CAMERA_SETTINGS", ""))


def stream_url():
    """The stream to open, assembled from the three facts and this daemon's own choices.

    Assembled rather than configured, so that changing the transport is a word in a file and
    not a URL somebody has to know the VAPIX spelling of.
    """
    if CAMERA_HOST.isdigit():
        return CAMERA_HOST
    credentials = f"{CAMERA_USER}:{CAMERA_PASSWORD}@" if CAMERA_USER else ""
    size = f"camera={CAMERA_VIEW}&resolution={CAMERA_RESOLUTION}&fps={CAMERA_FPS}"
    if CAMERA_TRANSPORT == "mjpeg":
        return f"http://{credentials}{CAMERA_HOST}/axis-cgi/mjpg/video.cgi?{size}"
    return f"rtsp://{credentials}{CAMERA_HOST}/axis-media/media.amp?videocodec=h264&{size}"


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
    # No bones across the face. The five landmarks up there are not a skeleton — joining them
    # draws a small angular cage where a head should be — so they are used to place an oval
    # instead, and only the nose is left as a point, because which way it sits inside the oval
    # is how you see where the mage is looking.
    ("left_shoulder", "right_shoulder"), ("left_shoulder", "left_elbow"),
    ("left_elbow", "left_wrist"), ("right_shoulder", "right_elbow"),
    ("right_elbow", "right_wrist"), ("left_shoulder", "left_hip"),
    ("right_shoulder", "right_hip"), ("left_hip", "right_hip"),
    ("left_hip", "left_knee"), ("left_knee", "left_ankle"),
    ("right_hip", "right_knee"), ("right_knee", "right_ankle"),
]

# Tunable, because infrared is out of distribution for a model trained on daylight photographs
# and the first thing that gives way is confidence rather than position. A skeleton that is
# roughly right at 0.15 is evidence the approach works; nothing at all at 0.3 is not evidence
# that it does not.
MOVENET_MIN_CONFIDENCE = float(os.environ.get("EYEBALL_MIN_CONFIDENCE", "0.3"))


def movenet_tensor(frame, size, dtype):
    """RGB, batched, cast. The frame arrives already square and already the right size."""
    if frame.shape[0] != size or frame.shape[1] != size:
        frame = cv2.resize(frame, (size, size), interpolation=cv2.INTER_AREA)
    rgb = cv2.cvtColor(frame, cv2.COLOR_BGR2RGB)
    return np.expand_dims(rgb, axis=0).astype(dtype)


class TfliteBackend:
    """The intended one: int8 weights on an XNNPACK runtime, which is what the pose-rate
    estimate in the design was made against."""

    name = "tflite"

    def __init__(self, interpreter):
        self.interpreter = interpreter
        self.input = interpreter.get_input_details()[0]
        self.output = interpreter.get_output_details()[0]
        self.size = self.input["shape"][1]
        self.dtype = self.input["dtype"]

    @staticmethod
    def load():
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
        if interpreter_class is None or not os.path.exists(MODEL_TFLITE):
            return None
        interpreter = interpreter_class(model_path=MODEL_TFLITE, num_threads=MODEL_THREADS)
        interpreter.allocate_tensors()
        return TfliteBackend(interpreter)

    def infer(self, frame):
        self.interpreter.set_tensor(
            self.input["index"], movenet_tensor(frame, self.size, self.input["dtype"])
        )
        self.interpreter.invoke()
        return self.interpreter.get_tensor(self.output["index"])[0][0]


class OnnxBackend:
    """ONNX Runtime. Slower than int8 TFLite, but its wheels track new Python versions far
    more promptly, which on this Pi has been the deciding property rather than speed."""

    name = "onnxruntime"

    NUMPY_TYPES = {
        "tensor(float)": np.float32,
        "tensor(int32)": np.int32,
        "tensor(uint8)": np.uint8,
    }

    def __init__(self, session):
        self.session = session
        spec = session.get_inputs()[0]
        self.input_name = spec.name
        self.dtype = self.NUMPY_TYPES.get(spec.type, np.float32)
        # The shape is [1, size, size, 3], but an exported model may leave dimensions symbolic,
        # in which case the configured size is the only answer available.
        self.size = spec.shape[1] if isinstance(spec.shape[1], int) else MODEL_INPUT

    @staticmethod
    def load():
        try:
            import onnxruntime
        except ImportError:
            return None
        if not os.path.exists(MODEL_ONNX):
            return None
        options = onnxruntime.SessionOptions()
        options.intra_op_num_threads = MODEL_THREADS
        return OnnxBackend(onnxruntime.InferenceSession(MODEL_ONNX, options))

    def infer(self, frame):
        tensor = movenet_tensor(frame, self.size, self.dtype)
        return self.session.run(None, {self.input_name: tensor})[0][0][0]


class OpenCvBackend:
    """OpenCV's own DNN module, reading the same ONNX file.

    The slowest of the three — it has no useful int8 path, so this is float arithmetic — and
    the only one that cannot fail on packaging, because OpenCV is already here. It exists so
    that the skeleton is never blocked on a wheel that does not exist for this interpreter.
    """

    name = "cv2.dnn"

    def __init__(self, net):
        self.net = net
        self.size = MODEL_INPUT
        self.dtype = np.float32

    @staticmethod
    def load():
        if not os.path.exists(MODEL_ONNX):
            return None
        # Unlike the other two, this can fail at parse time on an operator OpenCV does not
        # implement, and that is a property of how the model was exported rather than of the
        # file being wrong. It is the last one tried, so failing here costs only the skeleton.
        try:
            return OpenCvBackend(cv2.dnn.readNetFromONNX(MODEL_ONNX))
        except cv2.error as e:
            log(f"OpenCV cannot read {MODEL_ONNX}: {e}")
            return None

    def infer(self, frame):
        # blobFromImage produces NCHW; MoveNet wants NHWC.
        blob = cv2.dnn.blobFromImage(
            frame, 1.0, (self.size, self.size), swapRB=True, crop=False
        ).transpose(0, 2, 3, 1)
        self.net.setInput(blob)
        return self.net.forward()[0][0]


class MoveNet:
    """Single-pose inference. The only estimator that labels limbs, and so the only one that
    draws a skeleton — which is what the preview is for."""

    name = "movenet"
    bones = MOVENET_BONES

    BACKENDS = (TfliteBackend, OnnxBackend, OpenCvBackend)

    def __init__(self, backend):
        self.backend = backend
        # The model's own cost, apart from the preprocessing charged to it. The two together are
        # what the loop pays; only one of them is helped by threads, and only the other is helped
        # by a cheaper resize, so they are worth telling apart before tuning either.
        self.infer_ms = 0.0

    @staticmethod
    def load():
        """The first backend that has both a runtime and a model. None if that is none of them.

        Three of them because the packaging is the risk, not the arithmetic: this Pi runs a
        Python version that the pose runtimes have been slow to ship wheels for, and the whole
        point of the last one is that it needs no wheel at all.
        """
        for candidate in MoveNet.BACKENDS:
            backend = candidate.load()
            if backend is not None:
                # The input dtype is reported because it is the difference between the model the
                # design costed and a model that merely loaded. A float tensor here says the file
                # on disk is not the quantised one, whatever the runtime underneath it is, and
                # that is worth several times more than anything else in this loop can be tuned.
                log(
                    f"pose model on {backend.name}, {backend.size}px "
                    f"{np.dtype(backend.dtype).name} input, {MODEL_THREADS} threads"
                )
                return MoveNet(backend)
        return None

    def __call__(self, frame):
        # Fitted to square, never stretched to it.
        #
        # The model takes a square and the crop is whatever shape the stool wants, so something
        # has to give. Stretching hands the model a person of the wrong proportions — a third of
        # the frame's width at its full height arrives 1.7 times too tall — and a pose model's
        # whole prior is what human proportions look like.
        #
        # Shrunk first and padded second, in that order. Padding a 1280×720 crop to 1280×1280
        # before shrinking it means allocating and filling five megabytes per frame to throw
        # nearly all of it away at the resize — which cost this loop most of its frame rate. The
        # arithmetic is identical either way; only the size of the intermediate differs.
        size = self.backend.size
        height, width = frame.shape[:2]
        scale = size / max(width, height)
        fitted_w = max(1, min(size, round(width * scale)))
        fitted_h = max(1, min(size, round(height * scale)))
        pad_x, pad_y = (size - fitted_w) // 2, (size - fitted_h) // 2

        square = cv2.resize(frame, (fitted_w, fitted_h), interpolation=RESIZE)
        # Skipped outright when the crop is already square, which is the case worth having: the
        # crop is calibrated by hand against the mage and squaring it there is free, where padding
        # here allocates and fills a whole tensor to write nothing into the margins.
        if fitted_w != size or fitted_h != size:
            square = cv2.copyMakeBorder(
                square, pad_y, size - fitted_h - pad_y, pad_x, size - fitted_w - pad_x,
                cv2.BORDER_CONSTANT, value=(0, 0, 0),
            )

        # MoveNet returns (1, 1, 17, 3) as y, x, confidence — y before x — normalised against
        # the padded square, so the padding comes back out here and every consumer downstream
        # keeps seeing coordinates normalised within the crop.
        started = time.monotonic()
        raw = self.backend.infer(square)
        self.infer_ms = 0.9 * self.infer_ms + 0.1 * (time.monotonic() - started) * 1000
        keypoints = {
            name: (
                float((x * size - pad_x) / fitted_w),
                float((y * size - pad_y) / fitted_h),
                float(confidence),
            )
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


class Floors:
    """The worst each landmark has been over the last few seconds.

    A mean across the skeleton is a picture-quality number and it stays healthy through exactly
    the failure the arm mappings care about: the elbow sits between two landmarks the model
    finds easily, so a shoulder at 0.9 and a wrist at 0.8 will carry an elbow at 0.1 without the
    average moving enough to notice.

    A gap in the sightings is left as a gap rather than pushed in as a zero. Somebody stepping
    out of frame is not a landmark coming loose, and a floor that read zero for two seconds
    after every exit would say it was.
    """

    def __init__(self):
        self.history = {}

    def __call__(self, keypoints, now):
        for name, (_, _, confidence) in (keypoints or {}).items():
            self.history.setdefault(name, []).append((now, confidence))

        cutoff = now - CONFIDENCE_FLOOR_S
        floors = {}
        for name, samples in self.history.items():
            fresh = [sample for sample in samples if sample[0] >= cutoff]
            self.history[name] = fresh
            if fresh:
                floors[name] = min(confidence for _, confidence in fresh)
        return floors


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


class Watched(Latest):
    """A slot that also knows when somebody last asked for what is in it.

    On the slot rather than on the request handler, because not everything the handler serves is
    a picture that costs anything to produce: the page polls the pose for liveness every two
    seconds, and a flag shared with that poll would mean an open tab forever paying to draw a
    preview it is not showing.

    No lock: the timestamp is one float written by request threads and read by the frame loop,
    where a read landing either side of a write is a decision made a frame early or a frame late.
    """

    def __init__(self, value=None):
        super().__init__(value)
        self.last_request = 0.0

    def touch(self):
        self.last_request = time.monotonic()

    def watching(self):
        return time.monotonic() - self.last_request < PREVIEW_IDLE_S


# ── Capture ──────────────────────────────────────────────────────────────────


def open_camera():
    """Opens the camera, low-latency, retrying until it answers."""
    # RTSP over TCP: UDP loses slices on a busy link and OpenCV renders the tearing rather
    # than dropping the frame, which reads as a vision bug rather than a network one.
    os.environ.setdefault("OPENCV_FFMPEG_CAPTURE_OPTIONS", "rtsp_transport;tcp")

    url = stream_url()
    source = int(url) if url.isdigit() else url
    backoff_s = 1
    while True:
        capture = cv2.VideoCapture(source)
        if capture.isOpened():
            # Default buffering costs hundreds of milliseconds, and the mechanical lag of a
            # moving head has already spent the latency budget.
            capture.set(cv2.CAP_PROP_BUFFERSIZE, 1)
            log(f"camera open: {url.split('@')[-1]}")
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

# Left, right, and neither, rather than one colour for everything.
#
# Which arm is which is the distinction the show is built on — the selection sectors are
# per-arm, and calibrating them means watching one arm at a time — so it is the distinction the
# picture should carry. Amber against cyan rather than the obvious red against green: it is the
# pair that survives colour blindness, and both hold up against grass, skin and JPEG.
#
# Bones spanning the middle, and everything on the centre line, are white. The torso then reads
# as the frame it is, and the limbs as the things that move.
# Orange rather than amber, to stay clear of the crop's yellow. A large yellow rectangle and a
# yellowish arm inside it are one glance away from being read as the same thing.
COLOUR_LEFT = (0, 130, 255)
COLOUR_RIGHT = (255, 200, 0)
COLOUR_CENTRE = (240, 240, 240)
# Hard yellow, and nothing else in the picture is.
COLOUR_CROP = (0, 255, 255)
COLOUR_TEXT = (255, 255, 255)
# Everything is drawn twice, this colour underneath and one size wider. A skeleton in a single
# colour disappears wherever the image happens to match it, and this preview is shown to
# children against whatever a field looks like that afternoon — the outline is what makes it
# read on a bright shirt and on dark grass alike.
COLOUR_OUTLINE = (16, 16, 16)

# One definition each, because the status text is measured with one call and drawn with
# another: a font or scale that differed between them would size the backing box wrongly, and
# the mismatch would look like the text had moved rather than like the box was wrong.
# The face landmarks, which the oval replaces. The nose survives as a drawn point; the eyes and
# ears do their work by defining the oval and are not drawn themselves.
FACE_POINTS = ("left_eye", "right_eye", "left_ear", "right_ear")

# A head against the distance between the ears. Biauricular breadth is very nearly the widest
# part of a skull, so the width is barely more than that span; height runs about a third again,
# chin to crown. Both are anthropometry rather than taste, and neither is worth tuning.
HEAD_WIDTH_FROM_EARS = 1.05
HEAD_ASPECT = 1.35
# Interpupillary distance against head width, for the profile case where an ear is hidden.
HEAD_WIDTH_FROM_EYES = 2.4
# The ears sit below the middle of a head, so an oval centred on them rides low. Shifted up by
# this much of its own height — "up" taken from the shoulders, which is the only direction in
# the picture that reliably knows which way a body is standing.
HEAD_RISE = 0.18

FONT = cv2.FONT_HERSHEY_SIMPLEX
FONT_SCALE = 0.5
FONT_THICKNESS = 1
TEXT_LINE_HEIGHT = 18
TEXT_PADDING = 4
# How far a column sits from the edge it is aligned against.
TEXT_MARGIN = 8


# What the skeleton is drawn at, by whether the show reads it.
#
# The arms carry the whole of the control — the upper segment aims a pair of heads and the lower
# one swings them — so they keep their side's colour and the full weight. The torso, the legs
# and the head are there to say that the shape in the picture is a person and which way it is
# facing, which is a job that is done just as well thin and white, and done worse at a weight
# that competes with the two limbs somebody is actually watching.
ARM_WIDTH, ARM_OUTLINE = 3, 6
# Half, as near as an integer line width gets to it.
QUIET_WIDTH, QUIET_OUTLINE = 2, 4

# Which landmarks belong to an arm. `tip` is the silhouette's word for the same thing.
ARM_LANDMARKS = ("shoulder", "elbow", "wrist", "tip")
# Which of them make a bone one of an arm's segments. The shoulder is left out here and kept
# above, because the span between the two shoulders is a torso bone with a shoulder at each end.
ARM_SEGMENT_ENDS = ("elbow", "wrist", "tip")


class Stroke:
    """How one bone is drawn: its colour, and the two widths of the pass that lays it down."""

    def __init__(self, colour, width, outline):
        self.colour = colour
        self.width = width
        self.outline = outline


def side_colour(name):
    """Which side a landmark is on, by the naming every estimator here already uses.

    Derived from the name rather than tabulated per estimator, so the silhouette's `left_tip`
    and `right_tip` colour themselves correctly without knowing this scheme exists, and so do
    whatever landmarks come next.
    """
    if name.startswith("left_"):
        return COLOUR_LEFT
    if name.startswith("right_"):
        return COLOUR_RIGHT
    return COLOUR_CENTRE


def landmark_colour(name):
    """A joint takes its side's colour only if it is part of an arm. Everything else is white."""
    if name.endswith(ARM_LANDMARKS):
        return side_colour(name)
    return COLOUR_CENTRE


def bone_stroke(first, second):
    """An arm segment in its side's colour at full weight; every other bone thin and white."""
    if first.endswith(ARM_SEGMENT_ENDS) or second.endswith(ARM_SEGMENT_ENDS):
        start, end = side_colour(first), side_colour(second)
        colour = start if start == end else COLOUR_CENTRE
        return Stroke(colour, ARM_WIDTH, ARM_OUTLINE)
    return Stroke(COLOUR_CENTRE, QUIET_WIDTH, QUIET_OUTLINE)


ARM_CHAIN = ("shoulder", "elbow", "wrist")


def confidence_mean(keypoints):
    """How sure the model is overall, as the number worth watching while tuning the picture.

    The mean across every landmark tracks image quality — it is what rises when the crop
    tightens, the exposure stops blurring, or the illuminator reaches. What it does not track is
    whether any particular joint is usable, which is what the two arm lines are for.
    """
    if not keypoints:
        return "conf —"
    values = [confidence for _, _, confidence in keypoints.values()]
    return f"conf {sum(values) / len(values):.2f}"


def arm_lines(keypoints, floors, side):
    """One arm's chain — shoulder, elbow, wrist — as it is now over the worst it has just been.

    This arm is what the show is built on: its upper segment aims a pair of heads and its lower
    one swings them, so the whole of the pointing rests on three landmarks of which the middle
    one is by far the least certain. Two numbers each: the first says whether the joint is
    there, the second whether it stayed there while somebody moved.

    A line per joint, and no side written on any of them. Which arm these belong to is said by
    which edge of the picture they are drawn against, which is a thing the eye reads without
    being asked to.
    """
    lines = []
    for joint in ARM_CHAIN:
        name = f"{side}_{joint}"
        found = keypoints.get(name) if keypoints else None
        now = f"{found[2]:.2f}" if found else "—"
        floor = f"{floors[name]:.2f}" if name in floors else "—"
        lines.append(f"{joint[:3]} {now}/{floor}")
    return lines


class Readout:
    """What the preview prints, and which edge of the picture each part goes against.

    Two columns rather than a block in the corner, because the numbers were crossing the mage.
    Splitting them to the margins leaves the middle of the frame — which is the only part of it
    anybody is actually looking at — clear.

    Which arm goes on which side is not a layout choice. This camera faces the mage, so the
    picture is mirrored: their left arm appears on the right of the frame. Printing an arm's
    numbers on the side the limb is drawn means nobody has to hold the mirror in their head
    while they are also the one waving the arm about.
    """

    def __init__(self, header, image_left, image_right):
        self.header = header
        self.image_left = image_left
        self.image_right = image_right


def head_oval(placed):
    """The head as a rotated ellipse, or None when too little of the face was found.

    Built from the ears where both are visible, because the span between them is the head's
    width and the line between them is its roll. In profile one ear disappears, so the eyes are
    the fallback — a worse estimate of width, but the only one left that still carries an angle.

    Returns what `cv2.ellipse` takes: centre, half-axes along and across that angle, degrees.
    """
    left_ear, right_ear = placed.get("left_ear"), placed.get("right_ear")
    left_eye, right_eye = placed.get("left_eye"), placed.get("right_eye")

    if left_ear and right_ear:
        across, span = (left_ear, right_ear), HEAD_WIDTH_FROM_EARS
    elif left_eye and right_eye:
        across, span = (left_eye, right_eye), HEAD_WIDTH_FROM_EYES
    else:
        return None

    (x1, y1), (x2, y2) = across
    separation = math.hypot(x2 - x1, y2 - y1)
    if separation < 2:
        return None

    width = separation * span
    height = width * HEAD_ASPECT
    angle = math.degrees(math.atan2(y2 - y1, x2 - x1))
    centre = ((x1 + x2) / 2.0, (y1 + y2) / 2.0)

    # Lifted off the ear line toward the crown. Which way that is cannot be read from the face —
    # a head tilted back has its nose above its ears — so it comes from the shoulders, the one
    # pair of landmarks that always knows which end of the body is up.
    shoulders = [placed.get("left_shoulder"), placed.get("right_shoulder")]
    if all(shoulders):
        (sx1, sy1), (sx2, sy2) = shoulders
        up = (centre[0] - (sx1 + sx2) / 2.0, centre[1] - (sy1 + sy2) / 2.0)
        reach = math.hypot(*up)
        if reach > 1:
            rise = height * HEAD_RISE
            centre = (centre[0] + up[0] / reach * rise, centre[1] + up[1] / reach * rise)

    return (
        (int(round(centre[0])), int(round(centre[1]))),
        (max(2, int(round(width / 2))), max(2, int(round(height / 2)))),
        angle,
    )


def downscale(frame):
    """Shrinks to the preview width, or returns the frame untouched if it is already small."""
    height, width = frame.shape[:2]
    if width <= PREVIEW_MAX_WIDTH:
        return frame
    scale = PREVIEW_MAX_WIDTH / width
    return cv2.resize(
        frame, (PREVIEW_MAX_WIDTH, max(1, int(height * scale))), interpolation=cv2.INTER_AREA
    )


def annotate(frame, stool, keypoints, estimator, readout):
    """Draws the crop, the skeleton and the running numbers onto a copy of the frame.

    The skeleton is the point of this picture, not a debugging overlay on it — it is what makes
    the rig legible to somebody being shown how it works. So it is drawn to be seen from across
    a field on a laptop screen: outlined, thick enough to survive JPEG, and joints on top of
    bones so a limb reads as jointed rather than as a bent stick.
    """
    x, y, w, h = stool
    height, width = frame.shape[:2]

    # Grey outside the crop, colour inside it. The estimator only ever sees the box, so the
    # picture says which part of the field is being looked at and which is merely present —
    # legible at a glance, and from across a field, in a way a thin rectangle is not. Grey
    # rather than dark, because the surroundings are still what the camera is aimed by.
    canvas = cv2.cvtColor(cv2.cvtColor(frame, cv2.COLOR_BGR2GRAY), cv2.COLOR_GRAY2BGR)
    canvas[y:y + h, x:x + w] = frame[y:y + h, x:x + w]

    # Pulled a pixel inside its own bounds, because a crop of the whole frame otherwise draws
    # its rectangle on the border where it is clipped away and reads as no rectangle at all.
    cv2.rectangle(
        canvas,
        (min(x + 1, width - 2), min(y + 1, height - 2)),
        (max(x + w - 2, 1), max(y + h - 2, 1)),
        COLOUR_CROP,
        2,
    )

    if keypoints:
        # Keypoints are normalised within the crop, so the crop origin puts them back into the
        # picture — the preview shows where the mage is in the room, not in the tensor.
        placed = {
            name: (int(x + px * w), int(y + py * h))
            for name, (px, py, confidence) in keypoints.items()
            if confidence >= MOVENET_MIN_CONFIDENCE
        }
        bones = [
            (placed[first], placed[second], bone_stroke(first, second))
            for first, second in estimator.bones
            if first in placed and second in placed
        ]
        # The eyes and ears place the head and are not drawn; the nose stays, because where it
        # sits inside the oval is how you read which way the mage is facing.
        head = head_oval(placed)
        drawn = {
            name: point for name, point in placed.items()
            if name not in FACE_POINTS or (head is None and name != "nose")
        }

        # Every outline first, then every fill: drawing each bone's outline immediately before
        # its own fill would let the next bone's outline cut a dark notch through the last
        # bone's body wherever two limbs cross.
        for start, end, stroke in bones:
            cv2.line(canvas, start, end, COLOUR_OUTLINE, stroke.outline, cv2.LINE_AA)
        if head:
            centre, axes, angle = head
            cv2.ellipse(canvas, centre, axes, angle, 0, 360,
                        COLOUR_OUTLINE, QUIET_OUTLINE, cv2.LINE_AA)
        for start, end, stroke in bones:
            cv2.line(canvas, start, end, stroke.colour, stroke.width, cv2.LINE_AA)
        if head:
            cv2.ellipse(canvas, centre, axes, angle, 0, 360,
                        COLOUR_CENTRE, QUIET_WIDTH, cv2.LINE_AA)
        for point in drawn.values():
            cv2.circle(canvas, point, 6, COLOUR_OUTLINE, -1, cv2.LINE_AA)
        for name, point in drawn.items():
            cv2.circle(canvas, point, 4, landmark_colour(name), -1, cv2.LINE_AA)

    def label(text, row, align_right):
        # Measured rather than estimated, because the right-hand column is positioned from its
        # own width and a guess there walks off the edge as soon as a number gains a digit.
        (text_w, text_h), baseline = cv2.getTextSize(text, FONT, FONT_SCALE, FONT_THICKNESS)
        left = width - TEXT_MARGIN - text_w if align_right else TEXT_MARGIN
        base = 20 + row * TEXT_LINE_HEIGHT
        cv2.rectangle(
            canvas,
            (left - TEXT_PADDING, base - text_h - TEXT_PADDING),
            (left + text_w + TEXT_PADDING, base + baseline),
            COLOUR_OUTLINE,
            -1,
        )
        cv2.putText(canvas, text, (left, base), FONT, FONT_SCALE,
                    COLOUR_TEXT, FONT_THICKNESS, cv2.LINE_AA)

    for row, text in enumerate(readout.header):
        label(text, row, False)
    # Both arms start on the same row, under the header, so the two chains read as a pair and
    # a joint can be compared across sides by looking straight across the picture.
    for row, text in enumerate(readout.image_left):
        label(text, len(readout.header) + row, False)
    for row, text in enumerate(readout.image_right):
        label(text, len(readout.header) + row, True)
    return canvas


def encode(frame):
    ok, buffer = cv2.imencode(".jpg", frame, [cv2.IMWRITE_JPEG_QUALITY, PREVIEW_QUALITY])
    if not ok:
        # A `None` here would be published into the preview slot and served to a browser as a
        # zero-length JPEG, which reads as a broken camera rather than as a failed encode.
        raise RuntimeError(f"cannot JPEG-encode a {frame.shape} frame")
    return buffer.tobytes()


# ── HTTP preview ─────────────────────────────────────────────────────────────

# The page reconnects itself.
#
# An MJPEG stream is one long response, so it dies with the connection and a browser will not
# retry it — after a daemon restart the tab holds a broken image until somebody reloads, which
# during a tuning session is every thirty seconds. So the page polls a cheap endpoint, and on
# seeing the daemon come back it re-requests the stream with a fresh query string, because a
# browser will otherwise serve the dead one from cache.
#
# The picture takes the whole viewport that the text around it does not, because full-screening
# this tab is how the rig gets shown to the kids and a small image pinned to a corner of a large
# dark page is a poor thing to gather round. A column flex with the stage as the only growing row
# is what makes that hold at any window shape; the `min-` zeroes on it stop a flex item refusing
# to shrink below its content and pushing the footer off-screen.
#
# The image is sized rather than merely capped, because the frame arrives far smaller than the
# screen it is shown on and a maximum only ever shrinks. `object-fit` is then what keeps a
# stretched box from stretching the picture inside it: the element fills the stage, the frame
# sits centred within it at its own proportions, and the border belongs to the stage for that
# reason — it is the edge of the viewing area rather than of the picture.
PAGE = b"""<!doctype html>
<title>eyeball</title>
<style>
html,body{height:100%}
body{background:#111;color:#ccc;font:14px system-ui;margin:0;padding:1rem;
     box-sizing:border-box;display:flex;flex-direction:column;gap:.6rem}
h1{margin:0;font-size:1.1rem}
#stage{flex:1;min-height:0;min-width:0;border:1px solid #333}
#view{display:block;width:100%;height:100%;object-fit:contain}
a{color:#6cf}
p{margin:0}
#state{color:#7a7}
#state.down{color:#fc6}
</style>
<h1>eyeball</h1>
<div id="stage"><img id="view" src="/annotated.mjpg"></div>
<p id="state">live</p>
<p><a href="/annotated.jpg">annotated frame</a> &middot;
<a href="/pose.json">pose</a></p>
<script>
const view = document.getElementById('view');
const state = document.getElementById('state');
let live = true;

function down(message) {
  if (!live) return;
  live = false;
  state.textContent = message;
  state.className = 'down';
}

function up() {
  live = true;
  state.textContent = 'live';
  state.className = '';
  view.src = '/annotated.mjpg?t=' + Date.now();
}

view.onerror = () => down('stream lost, retrying');

setInterval(() => {
  fetch('/pose.json', {cache: 'no-store'})
    .then(response => { if (!response.ok) throw new Error(response.status); })
    .then(() => { if (!live) up(); })
    .catch(() => down('daemon down, retrying'));
}, 2000);
</script>
"""


class Preview(BaseHTTPRequestHandler):
    """The human end. Single frames for aiming, a stream for watching, JSON for reading."""

    annotated = None
    pose = None

    protocol_version = "HTTP/1.1"

    def log_message(self, format, *args):
        """Silenced: a browser holding an MJPEG stream open would otherwise fill the journal."""

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
        slot.touch()
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
                slot.touch()
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


def vapix_ptz(opener, host, view):
    """The view area's digital pan, tilt and zoom, as the camera currently has them.

    A second reader, because this does not live in the parameter tree: `root.PTZ` does not
    exist on this camera and asking for it is an error, while `Properties.PTZ.DigitalPTZ` says
    yes. The position is runtime state behind `ptz.cgi` rather than a stored setting, so §5's
    rule — nothing written that is not read back — needs this endpoint polled alongside the
    parameter groups before anything commands a crop through it.

    The reply is `pan=0.0` per line, the same shape as the parameter tree, so it flattens the
    same way. An older firmware without the endpoint costs one logged line and nothing else.
    """
    url = f"http://{host}/axis-cgi/com/ptz.cgi?query=position&camera={view}"
    try:
        with opener.open(url, timeout=CAMERA_HTTP_TIMEOUT_S) as response:
            body = response.read().decode("utf-8", "replace")
    except (urllib.error.URLError, OSError) as e:
        log(f"camera ptz position unavailable ({e})")
        return {}
    found = {}
    for line in body.splitlines():
        key, separator, value = line.partition("=")
        if separator:
            found[f"ptz.{view}.{key.strip()}"] = value.strip()
    return found


def vapix_update(opener, host, settings):
    """Writes parameters. Returns the camera's reply, which is `OK` when it took them."""
    query = "&".join(
        f"{urllib.parse.quote(key)}={urllib.parse.quote(value)}"
        for key, value in settings.items()
    )
    url = f"http://{host}/axis-cgi/param.cgi?action=update&{query}"
    with opener.open(url, timeout=CAMERA_HTTP_TIMEOUT_S) as response:
        return response.read().decode("utf-8", "replace").strip()


def configure_camera(telemetry, opener, host, published):
    """Puts the camera into the state the show needs, then checks that it went there.

    Written *and read back*, never written alone. A camera can clamp a value to its own range,
    ignore a parameter its firmware spells differently, or revert one at the next day/night
    transition — and all three are indistinguishable from success at the moment of writing. The
    read-back is what makes the camera's state a fact rather than something this code believes.

    Both halves reach the broker: `camera/requested/…` is what was asked for, and the ordinary
    parameter tree is what the camera says it is. A disagreement between the two subtrees is
    the whole diagnosis, visible without logging into anything.

    A setting the camera refuses outright shows up here as a mismatch on every start, which is
    the intended outcome: it is a wrong line in `/etc/default/eyeball` and it stays visible until
    somebody takes it out. Nothing retries between starts — the camera is written once, when the
    file's contents are known to be fresh, and the deploy restarts this daemon regardless.

    Raises whatever the HTTP layer raises. Reaching the camera is either possible or it is not,
    and a caller handed a `False` has to remember to look at it — which is the failure that hides,
    because the code that forgets looks exactly like the code that succeeded. The retry policy
    belongs to the caller, which is the only place that knows a camera may still be booting.

    A mismatch is not an error and does not raise. The camera answered, the read-back is honest,
    and the disagreement is a line in a config file for a person to fix.
    """
    if not CAMERA_SETTINGS:
        return

    for key, value in CAMERA_SETTINGS.items():
        telemetry.publish(f"camera/requested/{key.replace('.', '/')}", value, retain=True)

    reply = vapix_update(opener, host, CAMERA_SETTINGS)
    if reply.upper() != "OK":
        # Logged rather than raised, because the read-back below is a better account of what
        # happened than this reply is: it names which settings are wrong instead of saying that
        # some were.
        log(f"camera did not accept every setting: {reply}")

    actual = vapix_parameters(opener, host, CAMERA_LIVE_GROUPS)
    publish_parameters(telemetry, actual, published)
    for key, wanted in CAMERA_SETTINGS.items():
        got = actual.get(key)
        if got == wanted:
            log(f"camera {key} = {got}")
        else:
            log(f"camera {key}: asked for {wanted!r}, reads {got!r}")


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
    configured = False
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

        # After the camera has answered once, and retried until the write goes through: a camera
        # still booting will refuse it, and a daemon that tried once would leave the sensor on
        # auto for the rest of the evening without ever saying so.
        #
        # This is the one place that knows a failure is worth another attempt, which is why it is
        # the one place that catches. `configured` is set only on the far side of the call, so a
        # write that raised leaves it false and comes round again next poll.
        if not configured and link["reachable"]:
            try:
                configure_camera(telemetry, opener, camera.hostname, published)
                configured = True
            except (urllib.error.URLError, OSError) as e:
                log(f"camera configuration failed, retrying in {CAMERA_POLL_S:.0f}s: {e}")

        live = vapix_parameters(opener, camera.hostname, CAMERA_LIVE_GROUPS)
        live.update(vapix_ptz(opener, camera.hostname, CAMERA_VIEW))
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
    if CAMERA_HOST:
        return
    log("EYEBALL_CAMERA_HOST is not set — refusing to start.")
    log("")
    log("The daemon needs three facts about the camera and works the rest out itself.")
    log("Put them in /etc/default/eyeball, mode 0600 because of the password:")
    log("")
    log("  EYEBALL_CAMERA_HOST=192.168.0.90")
    log("  EYEBALL_CAMERA_USER=root")
    log("  EYEBALL_CAMERA_PASSWORD=secret")
    log("")
    log("Optional, with the defaults shown:")
    log("")
    log("  EYEBALL_CAMERA_TRANSPORT=rtsp        rtsp or mjpeg")
    log("  EYEBALL_CAMERA_RESOLUTION=640x360")
    log("  EYEBALL_CAMERA_FPS=10")
    log("  EYEBALL_CHANNEL=colour               colour, red, green or blue")
    log("")
    log("A bare index as the host — EYEBALL_CAMERA_HOST=0 — opens a local video device for")
    log("desk testing, and leaves the camera untouched since there is none to configure.")
    log("")
    log("Then: sudo systemctl restart eyeball")
    sys.exit(EXIT_CONFIG)


def require_estimator():
    """Builds the estimator that was asked for, or refuses to start explaining why it cannot.

    Refusing rather than demoting, because the silhouette draws no skeleton and the skeleton is
    what the preview is for. A daemon that quietly ran without one would look like a working rig
    to everything downstream — the landmark stream would flow, the health topic would say
    connected, and the only symptom would be a picture nobody could read.

    Refusing is safe here in a way it would not be in the brain: the show already degrades to
    plain attentive when this process goes quiet, so a daemon that will not start costs a
    preview and a landmark stream, never the rig.
    """
    if ESTIMATOR == "silhouette":
        log("estimator: silhouette, by request — this draws no skeleton")
        return Silhouette()

    if ESTIMATOR != "movenet":
        log(f"EYEBALL_ESTIMATOR={ESTIMATOR!r} is not an estimator — refusing to start.")
        log("")
        log("  movenet     the pose model, which draws a skeleton")
        log("  silhouette  background subtraction, no skeleton, needs no model")
        sys.exit(EXIT_CONFIG)

    estimator = MoveNet.load()
    if estimator is not None:
        return estimator

    log("no pose model — refusing to start.")
    log("")
    log("The skeleton is what the preview is for, so running without one is not a fallback")
    log("this daemon will choose on its own. It looked for, in order:")
    log("")
    log(f"  {MODEL_TFLITE}   with ai-edge-litert or tflite-runtime")
    log(f"  {MODEL_ONNX}     with onnxruntime")
    log(f"  {MODEL_ONNX}     with OpenCV's own DNN module, which needs nothing installed")
    log("")
    log("The deploy fetches the .tflite and installs ai-edge-litert to read it, so both")
    log("missing at once usually means the model download failed — its error says where to")
    log("get one by hand. The last line above needs no runtime installed and takes a .onnx,")
    log("which is the way out if no wheel exists for this Python.")
    log("")
    log("To run without a skeleton deliberately, which the IR path may yet want:")
    log("")
    log("  EYEBALL_ESTIMATOR=silhouette in /etc/default/eyeball")
    sys.exit(EXIT_CONFIG)


def main():
    require_camera()
    # Before anything touches OpenCV. Left alone it spreads resize, colour conversion and the
    # background subtractor across every core on the box, including the one the DMX loop is
    # meant to have to itself.
    cv2.setNumThreads(MODEL_THREADS)
    estimator = require_estimator()
    # The camera first, and without its password. Its absence is the single most likely reason
    # for a daemon that runs but sees nothing, and `EnvironmentFile=-` means a missing
    # configuration file starts the daemon on defaults rather than failing where it would show.
    log(f"camera: {CAMERA_HOST} — {CAMERA_TRANSPORT} {CAMERA_RESOLUTION} at {CAMERA_FPS} fps")
    # Listed rather than counted, and before the camera is touched. What the daemon is about to
    # force is the thing a journal is read for after a picture comes out wrong, and reading it
    # back off a label would be inferring the answer instead of seeing it.
    if CAMERA_SETTINGS:
        for key, value in CAMERA_SETTINGS.items():
            log(f"camera setting: {key}={value}")
    else:
        log("camera settings: none configured, the camera keeps whatever it decided for itself")
    log(f"channel: {os.environ.get('EYEBALL_CHANNEL', 'colour')}, enhance: {ENHANCE}")
    log(f"confidence: {MOVENET_MIN_CONFIDENCE}")
    log(f"estimator: {estimator.name}")
    log(f"crop: {CROP}")

    frames = Latest()
    stats = {"captured": 0}
    threading.Thread(target=capture_thread, args=(frames, stats), daemon=True).start()

    Preview.annotated = Watched()
    # Not watched: it is served to the page's own two-second liveness poll, which asks whether
    # the daemon is up rather than for a picture. Keeping the drawing alive on the strength of
    # that poll would mean an open tab never stops paying for a preview it is not showing.
    Preview.pose = Latest()
    server = ThreadingHTTPServer(("0.0.0.0", HTTP_PORT), Preview)
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, daemon=True).start()
    log(f"preview on http://0.0.0.0:{HTTP_PORT}/")

    telemetry = rig_mqtt.Telemetry.connect(SERVICE)
    # Never the password: the telemetry tree is the one part of the rig anyone on the AP reads.
    telemetry.publish("identity", {
        "estimator": estimator.name,
        # No mode. What the camera was told is under `camera/requested/`, one topic per setting,
        # beside the parameter tree saying what it actually is — and a label summarising those
        # would be a second answer to the same question, free to disagree with the first.
        "channel": os.environ.get("EYEBALL_CHANNEL", "colour"),
        "enhance": ENHANCE,
        "min_confidence": MOVENET_MIN_CONFIDENCE,
        "camera": CAMERA_HOST,
        "view": CAMERA_VIEW,
        "transport": CAMERA_TRANSPORT,
        "resolution": CAMERA_RESOLUTION,
        "fps": CAMERA_FPS,
        "crop": dict(zip(("x", "y", "w", "h"), CROP)),
        "started_at": time.time(),
    }, retain=True)

    threading.Thread(
        target=camera_thread,
        args=(telemetry, urllib.parse.urlparse(stream_url())),
        daemon=True,
    ).start()

    floors = Floors()
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
        region = frame[y:y + h, x:x + w]
        if CHANNEL is not None:
            # Back to three channels because the models want three. The information is one
            # plane's worth either way; this only stops the show's colours being part of it.
            region = cv2.cvtColor(region[:, :, CHANNEL], cv2.COLOR_GRAY2BGR)
        if ENHANCE == "clahe":
            # An infrared frame is already grey in all three channels, so flattening and
            # re-expanding costs nothing and keeps this one path for both regimes.
            region = cv2.cvtColor(
                CLAHE.apply(cv2.cvtColor(region, cv2.COLOR_BGR2GRAY)), cv2.COLOR_GRAY2BGR
            )
        # Timed on its own, because the alternative is arguing about whether the cost is the
        # estimator or the decode when one number separates them.
        started = time.monotonic()
        keypoints = estimator(region)
        estimate_ms = 0.9 * estimate_ms + 0.1 * (time.monotonic() - started) * 1000

        sequence += 1
        now = time.monotonic()
        # Smoothed rather than instantaneous: the interesting question is whether the pose rate
        # holds, and a single slow frame answers nothing.
        fps = 0.9 * fps + 0.1 / max(now - last_tick, 1e-6)
        last_tick = now

        held = floors(keypoints, now)

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
        # thing in the loop — a frame copy, the annotation, and a JPEG encode — and it is charged
        # straight to the pose rate. The landmark stream and the telemetry above are unaffected:
        # the show never depended on the preview being produced.
        drawing = Preview.annotated.watching()
        if drawing:
            small = downscale(frame)
            # The crop box is in full-frame pixels and the preview may be smaller, so it is
            # scaled to match. Keypoints need no such treatment: they are normalised within the
            # crop and land correctly wherever the crop itself is drawn.
            ratio = small.shape[1] / frame.shape[1]
            scaled = tuple(int(value * ratio) for value in box)
            readout = Readout(
                header=[estimator.name, f"{fps:4.1f} Hz", confidence_mean(keypoints)],
                # Mirrored on purpose: the camera faces the mage, so their right arm is the one
                # drawn on the left of the picture.
                image_left=arm_lines(keypoints, held, "right"),
                image_right=arm_lines(keypoints, held, "left"),
            )
            Preview.annotated.put(
                encode(annotate(small, scaled, keypoints, estimator, readout))
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
                # The model alone. Whatever `estimate_ms` has above this is the shrink to the
                # model's input — a different problem with a different fix.
                "infer_ms": round(getattr(estimator, "infer_ms", 0.0), 1),
                # How much of the model's square input the crop actually covers. The rest is the
                # black the letterbox padded it with — real input resolution spent on nothing.
                # A square crop reads 1.0; a 16:9 frame reads 0.56, and that missing 44% is
                # detail on the mage that was available and thrown away.
                "input_fill": round(min(w, h) / max(w, h), 2),
                # Whether the preview was being drawn while the rest of this was measured. A
                # frame rate taken with a browser open is a different number, and this is what
                # says which one you are looking at.
                "previewing": drawing,
                "seq": sequence,
                "captured": stats["captured"],
                "present": keypoints is not None,
                "estimator": estimator.name,
                # The same floors the preview draws, for the sittings where nobody has a browser
                # pointed at the rig. Six landmarks rather than seventeen: these are the ones the
                # arm mappings are built on, and a status payload that carries the whole skeleton
                # every five seconds buries them.
                "arm_floor": {
                    f"{side}_{joint}": round(held[f"{side}_{joint}"], 2)
                    for side in ("left", "right")
                    for joint in ARM_CHAIN
                    if f"{side}_{joint}" in held
                },
            }, retain=True)


if __name__ == "__main__":
    main()
