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

# What the daemon insists the camera is set to, written at startup and read back.
#
# Every one of these is a thing the design requires and the camera will otherwise decide for
# itself, badly, from a scene the show is actively changing: an auto IR-cut filter oscillating
# between day and night as beams sweep across the mage, an auto white balance chasing a wash
# that has no green in it, an auto exposure re-metering every time a head moves.
#
# The values differ between firmware generations, so they are overridable wholesale — and every
# one is read back afterwards, because a camera that silently clamps or ignores a setting looks
# exactly like one that accepted it.
# Two presets rather than one, because the rig runs in two lighting regimes and the camera has
# to be told which. `day` locks a colour sensor against a scene the show keeps changing. `night`
# takes the IR-cut filter out so the sensor sees 850 nm, and turns on the camera's own
# illuminator — at which point the picture is monochrome and lit by something the show's LEDs
# emit almost none of, which is the whole appeal.
#
# Exposure is held in day and left automatic at night, deliberately. Holding it means freezing
# whatever value was current when the daemon started, which is right for a scene whose base
# illumination is constant and wrong for one that has just been handed a different light source.
# Once the IR exposure that works is known, it becomes a number and gets locked like the rest.
CAMERA_MODES = {
    "day": (
        "ImageSource.I0.DayNight.IrCutFilter=yes,"
        "ImageSource.I0.Sensor.WhiteBalance=fixed_outdoor1,"
        "ImageSource.I0.Sensor.Exposure=hold"
    ),
    # The illuminator is not commanded here. `Light.L0.Enabled` does not exist on this camera —
    # the write is refused and the read-back returns nothing — and the lamp evidently follows
    # the IR-cut filter on its own, since infrared works without ever being asked for. A setting
    # that fails on every startup is noise that teaches you to skip past error lines, which is
    # worse than not having it. The real key, if one is wanted, is in `eyeball/camera/Light/`.
    "night": (
        "ImageSource.I0.DayNight.IrCutFilter=no,"
        "ImageSource.I0.Sensor.WhiteBalance=fixed_outdoor1,"
        "ImageSource.I0.Sensor.Exposure=auto"
    ),
}
#
# `night` is the default because the show is a night show and because infrared measurably beats
# visible light here — see the reasoning in EYEBALL.md, which is where that finding lives.
CAMERA_MODE = os.environ.get("EYEBALL_CAMERA_MODE", "night")

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

# One Euro smoothing on the landmarks.
#
# A neural estimator's output wobbles by a pixel or two per frame even when nothing moves, and
# at this pose rate that wobble reaches the heads as a target that will not sit still. A plain
# low-pass fixes it and adds lag to every movement, which is the wrong trade for a show where
# the interesting gestures are the fast ones.
#
# One Euro varies its cutoff with speed: heavy smoothing when the landmark is nearly still,
# opening up as it moves, so a held pose is rock steady and a thrown arm is barely delayed.
#
# `min_cutoff` sets the stillness case — lower is steadier and laggier. `beta` sets how fast the
# filter gets out of the way — higher is more responsive and lets more jitter through while
# moving. The pair below was chosen against this rig's pose rate rather than taken from the
# paper, whose defaults assume something nearer 60 Hz and barely smooth anything at seven: it
# cuts a still landmark's wander to about a third while a half-frame arm sweep still settles as
# quickly as the looser settings manage. Coordinates are normalised, so `beta` is scaled for a
# brisk wrist covering perhaps half a frame per second.
#
# Both are a starting point for tuning on a person, not a result.
SMOOTHING = os.environ.get("EYEBALL_SMOOTHING", "on") != "off"
SMOOTH_MIN_CUTOFF = float(os.environ.get("EYEBALL_SMOOTH_MIN_CUTOFF", "0.2"))
SMOOTH_BETA = float(os.environ.get("EYEBALL_SMOOTH_BETA", "2.0"))
SMOOTH_D_CUTOFF = float(os.environ.get("EYEBALL_SMOOTH_D_CUTOFF", "1.0"))
# How long a landmark may be missing before its filter starts fresh. Without this, an arm that
# leaves the frame and returns elsewhere slides across the picture from where it used to be.
SMOOTH_RESET_S = float(os.environ.get("EYEBALL_SMOOTH_RESET_S", "0.5"))

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

# How the crop is shrunk to the model's input. INTER_AREA is the right answer for quality on a
# large downscale and it is not cheap — 720×720 to 192×192 is a factor of nearly four, and that
# work is charged to the estimator. INTER_LINEAR trades some aliasing for speed, which a model
# this tolerant may not notice.
RESIZE = cv2.INTER_LINEAR if os.environ.get("EYEBALL_RESIZE") == "linear" else cv2.INTER_AREA

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


CAMERA_SETTINGS = parse_settings(
    os.environ.get("EYEBALL_CAMERA_SETTINGS", CAMERA_MODES.get(CAMERA_MODE, ""))
)


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
    # The face, which the model reports and nothing consumed until now. Drawn because a
    # skeleton with a head reads as a person and one without reads as a diagram, and this
    # picture has to explain the rig to a child.
    ("nose", "left_eye"), ("nose", "right_eye"),
    ("left_eye", "left_ear"), ("right_eye", "right_ear"),

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
                log(f"pose model on {backend.name}, {backend.size}px input, {MODEL_THREADS} threads")
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

        fitted = cv2.resize(frame, (fitted_w, fitted_h), interpolation=RESIZE)
        square = cv2.copyMakeBorder(
            fitted, pad_y, size - fitted_h - pad_y, pad_x, size - fitted_w - pad_x,
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


# ── Smoothing ────────────────────────────────────────────────────────────────


class OneEuro:
    """One scalar, low-passed with a cutoff that rises as the value moves.

    Casiez, Roussel and Vogel's filter, and the one the build order asks for by name. The whole
    of it is that the smoothing factor is recomputed each sample from a cutoff frequency which
    is itself a function of the signal's own speed — so it is not a compromise between steady
    and responsive, it is both, at the moments each is wanted.

    Rate-adaptive by construction: every sample carries its own timestamp, which matters here
    because the pose rate is neither fixed nor fast.
    """

    def __init__(self, min_cutoff, beta, d_cutoff):
        self.min_cutoff = min_cutoff
        self.beta = beta
        self.d_cutoff = d_cutoff
        self.value = None
        self.derivative = 0.0
        self.at = None

    @staticmethod
    def alpha(cutoff, dt):
        tau = 1.0 / (2.0 * math.pi * cutoff)
        return 1.0 / (1.0 + tau / dt)

    def reset(self):
        self.value = None
        self.derivative = 0.0
        self.at = None

    def __call__(self, value, now):
        if self.value is None:
            self.value, self.at = value, now
            return value
        # Floored, because two samples from one frame would otherwise divide by nearly zero and
        # hand the derivative an enormous number.
        dt = max(now - self.at, 1e-3)
        self.at = now

        # The derivative is low-passed too, at a fixed cutoff. Raw frame-to-frame difference is
        # almost entirely jitter, and feeding that to the cutoff would make the filter open up
        # for noise — which is exactly what it exists to close down on.
        derivative = (value - self.value) / dt
        self.derivative += self.alpha(self.d_cutoff, dt) * (derivative - self.derivative)

        cutoff = self.min_cutoff + self.beta * abs(self.derivative)
        self.value += self.alpha(cutoff, dt) * (value - self.value)
        return self.value


class Smoother:
    """A pair of One Euro filters per landmark, and the bookkeeping to keep them honest."""

    def __init__(self):
        self.filters = {}
        self.seen = {}

    def __call__(self, keypoints, now):
        smoothed = {}
        for name, (x, y, confidence) in keypoints.items():
            # Landmarks the model is unsure of are passed through untouched and not fed to the
            # filter. A low-confidence keypoint is usually somewhere arbitrary, and letting one
            # into the filter drags the smoothed position after it for several frames — the
            # estimate would be corrupted by exactly the samples worth ignoring.
            if confidence < MOVENET_MIN_CONFIDENCE:
                smoothed[name] = (x, y, confidence)
                continue

            axes = self.filters.get(name)
            if axes is None:
                axes = self.filters[name] = (
                    OneEuro(SMOOTH_MIN_CUTOFF, SMOOTH_BETA, SMOOTH_D_CUTOFF),
                    OneEuro(SMOOTH_MIN_CUTOFF, SMOOTH_BETA, SMOOTH_D_CUTOFF),
                )
            elif now - self.seen.get(name, now) > SMOOTH_RESET_S:
                for axis in axes:
                    axis.reset()

            self.seen[name] = now
            smoothed[name] = (axes[0](x, now), axes[1](y, now), confidence)
        return smoothed


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
COLOUR_DETECTION = (255, 0, 255)
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


def bone_colour(first, second):
    """A limb takes its side's colour; anything spanning the middle is white."""
    start, end = side_colour(first), side_colour(second)
    return start if start == end else COLOUR_CENTRE


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
            (placed[first], placed[second], bone_colour(first, second))
            for first, second in estimator.bones
            if first in placed and second in placed
        ]
        # Every outline first, then every fill: drawing each bone's outline immediately before
        # its own fill would let the next bone's outline cut a dark notch through the last
        # bone's body wherever two limbs cross.
        for start, end, _ in bones:
            cv2.line(canvas, start, end, COLOUR_OUTLINE, 6, cv2.LINE_AA)
        for start, end, colour in bones:
            cv2.line(canvas, start, end, colour, 3, cv2.LINE_AA)
        for point in placed.values():
            cv2.circle(canvas, point, 6, COLOUR_OUTLINE, -1, cv2.LINE_AA)
        for name, point in placed.items():
            cv2.circle(canvas, point, 4, side_colour(name), -1, cv2.LINE_AA)

        # The detection: where the confident landmarks actually reached. Reported as fractions
        # of the whole frame and in the order EYEBALL_CROP takes, because tightening the crop
        # around the stool is otherwise a matter of guessing at numbers — stand on it, read the
        # line off the preview, paste it into the configuration.
        if placed:
            xs = [point[0] for point in placed.values()]
            ys = [point[1] for point in placed.values()]
            left, right = min(xs), max(xs)
            top, bottom = min(ys), max(ys)
            cv2.rectangle(canvas, (left, top), (right, bottom), COLOUR_DETECTION, 1, cv2.LINE_AA)
            status.append(
                "pose {:.2f},{:.2f},{:.2f},{:.2f}".format(
                    left / width, top / height,
                    (right - left) / width, (bottom - top) / height,
                )
            )

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

# The page reconnects itself.
#
# An MJPEG stream is one long response, so it dies with the connection and a browser will not
# retry it — after a daemon restart the tab holds a broken image until somebody reloads, which
# during a tuning session is every thirty seconds. So the page polls a cheap endpoint, and on
# seeing the daemon come back it re-requests the stream with a fresh query string, because a
# browser will otherwise serve the dead one from cache.
PAGE = b"""<!doctype html>
<title>eyeball</title>
<style>
body{background:#111;color:#ccc;font:14px system-ui;margin:0;padding:1rem}
img{max-width:100%;display:block;border:1px solid #333}
a{color:#6cf}
#state{margin:.6rem 0;color:#7a7}
#state.down{color:#fc6}
</style>
<h1>eyeball</h1>
<img id="view" src="/annotated.mjpg">
<p id="state">live</p>
<p><a href="/raw.jpg">raw frame</a> &middot;
<a href="/annotated.jpg">annotated frame</a> &middot;
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
    """
    for key, value in CAMERA_SETTINGS.items():
        telemetry.publish(f"camera/requested/{key.replace('.', '/')}", value, retain=True)

    try:
        reply = vapix_update(opener, host, CAMERA_SETTINGS)
    except (urllib.error.URLError, OSError) as e:
        log(f"camera configuration failed: {e}")
        return False
    if reply.upper() != "OK":
        # Not a return. A rejected parameter is worth knowing about, and the read-back below is
        # a better account of what actually happened than this reply is.
        log(f"camera did not accept every setting: {reply}")

    actual = vapix_parameters(opener, host, CAMERA_LIVE_GROUPS)
    publish_parameters(telemetry, actual, published)
    for key, wanted in CAMERA_SETTINGS.items():
        got = actual.get(key)
        if got == wanted:
            log(f"camera {key} = {got}")
        else:
            log(f"camera {key}: asked for {wanted!r}, reads {got!r}")
    return True


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

        # After the camera has answered once, and retried until it takes: a camera still booting
        # will refuse the write, and a daemon that tried once would leave the sensor on auto for
        # the rest of the evening without ever saying so.
        if not configured and link["reachable"]:
            configured = configure_camera(telemetry, opener, camera.hostname, published)

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
    log(f"mode: {CAMERA_MODE}")
    log(f"channel: {os.environ.get('EYEBALL_CHANNEL', 'colour')}, enhance: {ENHANCE}")
    log(f"confidence: {MOVENET_MIN_CONFIDENCE}")
    if SMOOTHING:
        log(f"smoothing: one euro, min_cutoff {SMOOTH_MIN_CUTOFF}, beta {SMOOTH_BETA}")
    else:
        log("smoothing: off")
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
    # Never the password: the telemetry tree is the one part of the rig anyone on the AP reads.
    telemetry.publish("identity", {
        "estimator": estimator.name,
        "mode": CAMERA_MODE,
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

    smoother = Smoother()
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

        # After the timing, so `estimate_ms` stays the model's cost alone, and before everything
        # downstream, so the wire, the preview and the show all see the same smoothed landmarks.
        if SMOOTHING and keypoints:
            keypoints = smoother(keypoints, now)

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
                "previewing": Preview.watching(),
                "seq": sequence,
                "captured": stats["captured"],
                "present": keypoints is not None,
                "estimator": estimator.name,
            }, retain=True)


if __name__ == "__main__":
    main()
