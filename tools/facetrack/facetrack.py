"""Chimera Voice face tracker: watches your face through a webcam and sends its shape to Chimera.

MediaPipe's face landmarker finds the face in each camera frame. From it this works out:

  the mouth    how open it is, how far the jaw has dropped, the gap between the lips, how
               rounded ("oo") or spread ("ee") the lips are, whether the lips are shut, whether
               the lower lip is on the teeth ("f", "v"), and when a new syllable starts
  expression   eyebrows up or down, head tilted left or right, smile

It sends that to every Chimera Voice on this computer with its FACE switch on, over UDP on
127.0.0.1 (nothing leaves the machine, and no images are saved).

Keys in the tracker's window (they are also written in the window itself):
  SPACE  guided calibration: four poses, SPACE to capture each, Esc to stop
  R A U E  redo one pose: Rest, "Aah", "oo" (U), "Ee"
  1 2 3  send eyebrows / head tilt / smile to Chimera, or not
  V  video on/off (off saves processor time; the meters stay)
  Q  quit

The calibration is saved next to this script and loaded next time.
"""

import argparse
import json
import math
import os
import socket
import threading
import time

import cv2
import numpy as np
import mediapipe as mp
from mediapipe.tasks.python import BaseOptions, vision

HERE = os.path.dirname(os.path.abspath(__file__))
MODEL = os.path.join(HERE, "face_landmarker.task")
MODEL_URL = "https://storage.googleapis.com/mediapipe-models/face_landmarker/face_landmarker/float16/1/face_landmarker.task"
CALIBRATION = os.path.join(HERE, "calibration.json")

# landmark indices on MediaPipe's face mesh
LIP_TOP, LIP_BOTTOM = 13, 14  # inner lips, centre
MOUTH_LEFT, MOUTH_RIGHT = 61, 291  # mouth corners
FACE_LEFT, FACE_RIGHT = 234, 454  # cheeks, for scale
EYE_LEFT, EYE_RIGHT = 33, 263  # outer eye corners, for the head's tilt
LIPS_OUTER = [61, 185, 40, 39, 37, 0, 267, 269, 270, 409, 291, 375, 321, 405, 314, 17, 84, 181, 91, 146, 61]
LIPS_INNER = [78, 191, 80, 81, 82, 13, 312, 311, 310, 415, 308, 324, 318, 402, 317, 14, 87, 178, 88, 95, 78]

DEFAULT_CAL = {
    # lip gap / face width with the mouth shut and wide open
    "gap_closed": 0.005,
    "gap_open": 0.16,
    # the jaw's own measure (0..1) on a wide "aah"
    "jaw_open": 0.6,
    # mouth width / face width for a rounded "oo" and a spread "ee"
    "width_round": 0.30,
    "width_spread": 0.48,
    # the resting face: eyebrows, smile, and the head's tilt (degrees)
    "brows_rest": 0.0,
    "smile_rest": 0.0,
    "tilt_rest": 0.0,
}

# the four poses of the calibration: key, name, what to do, what it is for
POSES = [
    ("r", "REST", "Relax. Lips together, head straight, look at the camera.", "the shut mouth and your resting face"),
    ("a", "AAH", "Open wide and hold an \"aah\".", "the fully open mouth"),
    ("u", "OO", "Push your lips forward into a small \"oo\".", "rounded lips"),
    ("e", "EE", "Smile wide into an \"ee\".", "spread lips"),
]
# head tilt: nothing inside the dead zone, full at FULL degrees
TILT_DEAD, TILT_FULL = 4.0, 24.0

FONT = cv2.FONT_HERSHEY_SIMPLEX
INK, DIM, GOOD, WARN, BLUE, PINK, AMBER = (230, 220, 200), (160, 150, 130), (130, 230, 150), (80, 120, 255), (242, 182, 79), (207, 115, 255), (64, 179, 255)
PANEL_BG = (18, 12, 8)
PANEL_H = 214


class OneEuro:
    """The 1 euro filter (Casiez et al. 2012): smooths jitter when the value is still, follows
    fast when it moves, so it adds little delay where it matters."""

    def __init__(self, min_cutoff=2.0, beta=8.0, d_cutoff=1.0):
        self.min_cutoff, self.beta, self.d_cutoff = min_cutoff, beta, d_cutoff
        self.x = None
        self.dx = 0.0
        self.t = None

    @staticmethod
    def _alpha(cutoff, dt):
        tau = 1.0 / (2 * math.pi * cutoff)
        return 1.0 / (1.0 + tau / dt)

    def __call__(self, x, t):
        if self.x is None:
            self.x, self.t = x, t
            return x
        dt = max(1e-3, t - self.t)
        self.t = t
        dx = (x - self.x) / dt
        a_d = self._alpha(self.d_cutoff, dt)
        self.dx = a_d * dx + (1 - a_d) * self.dx
        a = self._alpha(self.min_cutoff + self.beta * abs(self.dx), dt)
        self.x = a * x + (1 - a) * self.x
        return self.x


class Syllables:
    """Finds syllable starts in the mouth's opening (0 shut ... 1 wide open): a new one each time
    the mouth opens by `rise` from the lowest it has been since the last one. The mouth counts as
    closed below `shut`, which ends the note."""

    def __init__(self, rise=0.16, shut=0.07, dip=0.12):
        self.rise, self.shut, self.dip = rise, shut, dip
        self.low = 0.0  # lowest since the mouth closed, or since the last peak while open
        self.peak = 0.0
        self.open = False
        self.count = 0

    def __call__(self, a):
        if not self.open:
            self.low = min(self.low, a)
            if a > self.shut and a - self.low > self.rise:
                self.open = True
                self.count += 1
                self.peak = self.low = a
        elif a < self.shut:
            self.open = False
            self.low = a
        else:
            if a > self.peak:
                self.peak = self.low = a
            self.low = min(self.low, a)
            # the mouth closed part way and opened again: the next syllable, sung legato
            if self.peak - self.low > self.dip and a - self.low > self.rise:
                self.count += 1
                self.peak = self.low = a
        return self.open


def clamp(x, lo=0.0, hi=1.0):
    return max(lo, min(hi, x))


def load_calibration():
    """The saved calibration, which poses have been done, and which expressions are sent."""
    cal = dict(DEFAULT_CAL)
    done = set()
    send = {"brows": True, "tilt": True, "smile": True}
    try:
        with open(CALIBRATION, encoding="utf-8") as f:
            data = json.load(f)
        cal.update({k: float(v) for k, v in data.items() if k in cal})
        if "done" in data:
            done = {str(k) for k in data["done"]} & {p[0] for p in POSES}
        else:
            # a file from before the poses were recorded: a value that is not the default was set
            for key, field in (("r", "gap_closed"), ("a", "gap_open"), ("u", "width_round"), ("e", "width_spread")):
                if field in data and abs(float(data[field]) - DEFAULT_CAL[field]) > 1e-9:
                    done.add(key)
        for k in send:
            if k in data.get("send", {}):
                send[k] = bool(data["send"][k])
    except (OSError, ValueError, TypeError, AttributeError):
        pass
    return cal, done, send


def save_calibration(cal, done, send):
    try:
        with open(CALIBRATION, "w", encoding="utf-8") as f:
            json.dump({**cal, "done": sorted(done), "send": send}, f, indent=2)
    except OSError as e:
        print("could not save the calibration:", e)


def capture(pose, raw, cal, done):
    """Take the pose the face is holding now as this calibration pose."""
    if pose == "r":
        cal["gap_closed"] = raw["gap"]
        cal["brows_rest"], cal["smile_rest"], cal["tilt_rest"] = raw["brows"], raw["smile"], raw["tilt"]
    elif pose == "a":
        cal["gap_open"] = raw["gap"]
        cal["jaw_open"] = max(0.2, raw["jaw"])
    elif pose == "u":
        cal["width_round"] = raw["width"]
    elif pose == "e":
        cal["width_spread"] = raw["width"]
    done.add(pose)


def measure(lm, shapes, w, h):
    """The raw measurements of one face: distances over the face's width, the jaw and the
    expression shapes as MediaPipe scores them, and the head's tilt in degrees."""
    p = lambda i: (lm[i].x * w, lm[i].y * h)
    dist = lambda a, b: math.dist(p(a), p(b))
    scale = max(1.0, dist(FACE_LEFT, FACE_RIGHT))
    (lx, ly), (rx, ry) = p(EYE_LEFT), p(EYE_RIGHT)
    s = lambda name: shapes.get(name, 0.0)
    return {
        "gap": dist(LIP_TOP, LIP_BOTTOM) / scale,
        "width": dist(MOUTH_LEFT, MOUTH_RIGHT) / scale,
        "jaw": s("jawOpen"),
        "pucker": s("mouthPucker") + 0.6 * s("mouthFunnel"),
        "brows": 0.5 * s("browInnerUp") + 0.25 * (s("browOuterUpLeft") + s("browOuterUpRight")) - 0.5 * (s("browDownLeft") + s("browDownRight")),
        "smile": 0.5 * (s("mouthSmileLeft") + s("mouthSmileRight")),
        "roll_lower": s("mouthRollLower"),
        "roll_upper": s("mouthRollUpper"),
        "tilt": math.degrees(math.atan2(ry - ly, rx - lx)),
    }


def features(raw, cal):
    """What Chimera is sent, from the raw measurements and the calibration. Everything is 0..1
    except rounding, brows and tilt, which are -1..1."""
    by_gap = (raw["gap"] - cal["gap_closed"]) / max(1e-4, cal["gap_open"] - cal["gap_closed"])
    jaw = clamp(raw["jaw"] / max(0.05, cal["jaw_open"]))
    # opening: the lip gap between the calibrated shut and wide-open mouths, with the jaw's own
    # measure as a second opinion
    opening = clamp(0.75 * by_gap + 0.25 * jaw)
    span = max(1e-4, cal["width_spread"] - cal["width_round"])
    spread01 = (raw["width"] - cal["width_round"]) / span  # 0 at "oo", 1 at "ee"
    # rounding: -1 spread ("ee") ... +1 rounded ("oo"), from the mouth's width and the pucker
    rounding = clamp(0.6 * (1.0 - 2.0 * spread01) + 0.8 * raw["pucker"], -1.0, 1.0)
    lips = clamp(by_gap)
    # the lips are shut when no gap is left between them
    closed = clamp(1.0 - by_gap / 0.05)
    # the lower lip tucked under the upper teeth, as for "f" and "v": it rolls in while the
    # upper lip does not, and the mouth is nearly shut
    teeth = clamp((raw["roll_lower"] - raw["roll_upper"] - 0.25) / 0.35) * clamp(1.0 - by_gap / 0.35)
    brows = clamp((raw["brows"] - cal["brows_rest"]) / 0.45, -1.0, 1.0)
    smile = clamp((raw["smile"] - cal["smile_rest"]) / 0.55)
    t = raw["tilt"] - cal["tilt_rest"]
    tilt = math.copysign(clamp((abs(t) - TILT_DEAD) / (TILT_FULL - TILT_DEAD)), t)
    return {
        "opening": opening,
        "rounding": rounding,
        "jaw": jaw,
        "lips": lips,
        "pucker": clamp(raw["pucker"]),
        "spread": clamp(spread01),
        "closed": closed,
        "teeth": teeth,
        "brows": brows,
        "tilt": tilt,
        "smile": smile,
    }


# the values that are smoothed before they are sent, in the packet's order after the first two
EXTRA = ["jaw", "lips", "pucker", "spread", "closed", "teeth", "brows", "tilt", "smile"]


def packet(seq, face, f, is_open, syllables, age_ms):
    """One frame, as a line of text (see src/face.rs)."""
    head = f"CHFACE 2 {seq} {int(face)} {f['opening']:.4f} {f['rounding']:.4f} {int(is_open)} {syllables} {age_ms:.1f}"
    return head + "".join(f" {f[k]:.3f}" for k in EXTRA)


class Camera:
    """Reads frames on its own thread and keeps only the newest, so a slow frame never queues up
    stale ones behind it (that would add delay)."""

    def __init__(self, index, width, height):
        self.cap = cv2.VideoCapture(index, cv2.CAP_MSMF)
        if not self.cap.isOpened():
            self.cap = cv2.VideoCapture(index, cv2.CAP_DSHOW)
        if not self.cap.isOpened():
            raise SystemExit(f"Camera {index} did not open. Is another program using it, or is camera access off in Windows Settings > Privacy > Camera?")
        self.cap.set(cv2.CAP_PROP_FRAME_WIDTH, width)
        self.cap.set(cv2.CAP_PROP_FRAME_HEIGHT, height)
        self.cap.set(cv2.CAP_PROP_FPS, 30)
        self.lock = threading.Lock()
        self.frame = None
        self.stamp = 0.0
        self.serial = 0
        self.frames = 0
        self.running = True
        threading.Thread(target=self._run, daemon=True).start()

    def _run(self):
        while self.running:
            ok, frame = self.cap.read()
            if not ok:
                time.sleep(0.005)
                continue
            with self.lock:
                self.frame, self.stamp = frame, time.perf_counter()
                self.serial += 1
                self.frames += 1

    def latest(self, after):
        with self.lock:
            if self.serial == after:
                return None, after, 0.0
            return self.frame, self.serial, self.stamp

    def close(self):
        self.running = False
        time.sleep(0.05)
        self.cap.release()


def text(img, s, x, y, size=0.45, color=INK, thick=1):
    cv2.putText(img, s, (x, y), FONT, size, color, thick, cv2.LINE_AA)


def meter(img, x, y, w, label, value, color, centre=False):
    """A labelled bar; `centre` draws a value in -1..1 from the middle."""
    text(img, label, x, y - 5, 0.4, DIM)
    cv2.rectangle(img, (x, y), (x + w, y + 9), (70, 60, 50), 1)
    if centre:
        mid = x + w // 2
        end = mid + int((w // 2 - 1) * clamp(value, -1.0, 1.0))
        cv2.rectangle(img, (min(mid, end), y + 1), (max(mid, end), y + 8), color, -1)
        cv2.line(img, (mid, y - 2), (mid, y + 11), (220, 220, 220), 1)
    else:
        cv2.rectangle(img, (x + 1, y + 1), (x + 1 + int((w - 2) * clamp(value)), y + 8), color, -1)


def draw_panel(width, f, face, is_open, syllables, fps, track_ms, done, send, wizard):
    """The panel under the video: status, what is being sent, and what the keys do."""
    img = np.full((PANEL_H, width, 3), PANEL_BG, np.uint8)
    status = "TRACKING YOUR FACE" if face else "NO FACE: look at the camera and light your face"
    text(img, status, 10, 20, 0.5, GOOD if face else WARN)
    text(img, f"{fps:4.1f} frames/s   {track_ms:4.1f} ms", width - 200, 20, 0.42, DIM)

    # ---- what Chimera is being sent
    col = [10, 10 + (width - 20) // 3, 10 + 2 * (width - 20) // 3]
    bar = (width - 20) // 3 - 16
    text(img, "MOUTH", col[0], 44, 0.4, BLUE)
    meter(img, col[0], 62, bar, "open", f["opening"], AMBER)
    meter(img, col[1], 62, bar, "ee  <  lips  >  oo", f["rounding"], PINK, centre=True)
    meter(img, col[2], 62, bar, "jaw", f["jaw"], AMBER)
    meter(img, col[0], 92, bar, "lips shut  (m, b)", f["closed"], GOOD)
    meter(img, col[1], 92, bar, "lip on teeth  (f, v)", f["teeth"], GOOD)
    cv2.circle(img, (col[2] + 6, 94), 6, AMBER if is_open else (70, 60, 50), -1)
    text(img, f"note    syllables {syllables}", col[2] + 18, 99, 0.42, INK)
    for i, (key, name, label) in enumerate([("brows", "eyebrows", "1"), ("tilt", "head tilt", "2"), ("smile", "smile", "3")]):
        on = send[key]
        meter(img, col[i], 122, bar, f"[{label}] {name}{'' if on else '  (not sent)'}", f[key] if on else 0.0, BLUE if on else (70, 60, 50), centre=key != "smile")

    # ---- the calibration, and the keys
    cv2.line(img, (10, 142), (width - 10, 142), (70, 60, 50), 1)
    if wizard is not None:
        key, name, what, _ = POSES[wizard]
        text(img, f"CALIBRATING  {wizard + 1} of {len(POSES)}:  {name}", 10, 164, 0.55, AMBER, 2)
        text(img, what, 10, 186, 0.48, INK)
        text(img, "Hold it and press SPACE.     Esc stops the calibration.", 10, 206, 0.42, DIM)
    else:
        x = 10
        text(img, "CALIBRATED:", x, 162, 0.42, DIM)
        x += 104
        for key, name, _, _ in POSES:
            ok = key in done
            text(img, f"{name} {'ok' if ok else 'NOT YET'}", x, 162, 0.42, GOOD if ok else WARN)
            x += 78 if ok else 118
        missing = len(POSES) - len(done)
        if missing:
            text(img, "Press SPACE to calibrate: it takes about 20 seconds and is saved.", 10, 184, 0.48, AMBER)
        else:
            text(img, "SPACE calibrates again.  R, A, U, E redo one pose (Rest, Aah, oo, Ee).", 10, 184, 0.42, INK)
        text(img, "1 2 3  send eyebrows / tilt / smile      V  video on or off      Q  quit", 10, 205, 0.42, DIM)
    return img


def draw_prompt(view, wizard):
    """The pose being asked for, large, across the video."""
    if wizard is None:
        return
    _, name, what, _ = POSES[wizard]
    h, w = view.shape[:2]
    cv2.rectangle(view, (0, h - 64), (w, h), (8, 12, 18), -1)
    text(view, f"{wizard + 1}/{len(POSES)}  {name}", 12, h - 36, 0.8, AMBER, 2)
    text(view, what + "  Then press SPACE.", 12, h - 12, 0.5, INK)


def main():
    ap = argparse.ArgumentParser(description="Send your face's shape from a webcam to Chimera Voice.")
    ap.add_argument("--camera", type=int, default=0, help="camera number (0 is usually the built-in one)")
    ap.add_argument("--port", type=int, default=9123, help="UDP port Chimera listens on (default 9123)")
    ap.add_argument("--width", type=int, default=640)
    ap.add_argument("--height", type=int, default=480)
    ap.add_argument("--no-video", action="store_true", help="start with the video preview off")
    ap.add_argument("--seconds", type=float, default=0, help="stop after this many seconds (for tests)")
    ap.add_argument("--headless", action="store_true", help="no window at all; print the values instead")
    ap.add_argument("--snapshot", help="save the window's picture to this file once a second (for tests)")
    args = ap.parse_args()

    if not os.path.exists(MODEL):
        print("Downloading the face model (3.7 MB) from Google...")
        import urllib.request

        urllib.request.urlretrieve(MODEL_URL, MODEL)

    landmarker = vision.FaceLandmarker.create_from_options(
        vision.FaceLandmarkerOptions(
            base_options=BaseOptions(model_asset_path=MODEL),
            running_mode=vision.RunningMode.VIDEO,
            output_face_blendshapes=True,
            min_face_detection_confidence=0.4,
            min_face_presence_confidence=0.4,
            min_tracking_confidence=0.4,
        )
    )
    cam = Camera(args.camera, args.width, args.height)
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    target = ("127.0.0.1", args.port)
    cal, done, send = load_calibration()
    filters = {k: OneEuro() for k in ["opening", "rounding"] + EXTRA}
    # the expressions move slowly and should not shake the pitch or the tone
    for k in ("brows", "tilt", "smile"):
        filters[k] = OneEuro(min_cutoff=1.0, beta=2.0)
    syl = Syllables()
    video = not args.no_video
    wizard = None  # the pose the guided calibration is asking for
    serial = 0
    seq = 0
    track_ms = 0.0
    fps, fps_frames, fps_t = 0.0, 0, time.perf_counter()
    started = time.perf_counter()
    raw = None
    zero = {k: 0.0 for k in ["opening", "rounding"] + EXTRA}
    f = dict(zero)
    face = False
    is_open = False
    window = "Chimera face tracker"
    if not args.headless:
        cv2.namedWindow(window, cv2.WINDOW_AUTOSIZE)
    last_print = 0.0
    last_snap = 0.0

    try:
        while True:
            frame, serial, stamp = cam.latest(serial)
            if frame is None:
                time.sleep(0.002)
                key = cv2.waitKey(1) & 0xFF if not args.headless else 255
            else:
                t0 = time.perf_counter()
                rgb = cv2.cvtColor(frame, cv2.COLOR_BGR2RGB)
                result = landmarker.detect_for_video(mp.Image(image_format=mp.ImageFormat.SRGB, data=rgb), int(stamp * 1000))
                face = bool(result.face_landmarks)
                now = time.perf_counter()
                if face:
                    h, w = frame.shape[:2]
                    shapes = {c.category_name: c.score for c in result.face_blendshapes[0]} if result.face_blendshapes else {}
                    raw = measure(result.face_landmarks[0], shapes, w, h)
                    target_f = features(raw, cal)
                else:
                    target_f = zero
                f = {k: filters[k](target_f[k], now) for k in target_f}
                is_open = syl(f["opening"]) if face else syl(0.0)
                track_ms = 0.8 * track_ms + 0.2 * (now - t0) * 1000
                age_ms = (now - stamp) * 1000
                seq += 1
                sent = dict(f)
                for k in send:
                    if not send[k]:
                        sent[k] = 0.0
                try:
                    sock.sendto(packet(seq, face, sent, is_open, syl.count, age_ms).encode("ascii"), target)
                except OSError:
                    pass
                fps_frames += 1
                if now - fps_t >= 1.0:
                    fps, fps_frames, fps_t = fps_frames / (now - fps_t), 0, now
                if args.headless:
                    if now - last_print >= 0.5:
                        last_print = now
                        print(f"{fps:4.1f} fps  track {track_ms:4.1f} ms  face {int(face)}  open {f['opening']:.2f}  round {f['rounding']:+.2f}  shut {f['closed']:.2f}  brows {f['brows']:+.2f}  tilt {f['tilt']:+.2f}  smile {f['smile']:.2f}  syllables {syl.count}", flush=True)
                    key = 255
                else:
                    panel = draw_panel(frame.shape[1], f, face, is_open, syl.count, fps, track_ms, done, send, wizard)
                    if video:
                        view = cv2.flip(frame, 1)
                        if face:
                            h, w = frame.shape[:2]
                            lm = result.face_landmarks[0]
                            pts = lambda idx: [(int((1 - lm[i].x) * w), int(lm[i].y * h)) for i in idx]
                            cv2.polylines(view, [np.array(pts(LIPS_OUTER), np.int32)], False, PINK, 2, cv2.LINE_AA)
                            cv2.polylines(view, [np.array(pts(LIPS_INNER), np.int32)], False, AMBER, 1, cv2.LINE_AA)
                        draw_prompt(view, wizard)
                        shown = np.vstack([view, panel])
                    else:
                        shown = panel
                    cv2.imshow(window, shown)
                    if args.snapshot and now - last_snap >= 1.0:
                        last_snap = now
                        cv2.imwrite(args.snapshot, shown)
                    key = cv2.waitKey(1) & 0xFF
            ch = chr(key).lower() if key < 128 else ""
            if wizard is not None:
                if key == 27:
                    wizard = None
                elif ch == " " and face and raw is not None:
                    capture(POSES[wizard][0], raw, cal, done)
                    save_calibration(cal, done, send)
                    print(f"calibrated {POSES[wizard][1]}")
                    wizard = wizard + 1 if wizard + 1 < len(POSES) else None
            elif ch == "q" or key == 27:
                break
            elif ch == " ":
                wizard = 0
            elif ch == "v":
                video = not video
            elif ch in ("1", "2", "3"):
                name = ["brows", "tilt", "smile"][int(ch) - 1]
                send[name] = not send[name]
                save_calibration(cal, done, send)
            elif face and raw is not None and ch in [p[0] for p in POSES]:
                capture(ch, raw, cal, done)
                save_calibration(cal, done, send)
                print(f"calibrated {[p[1] for p in POSES if p[0] == ch][0]}")
            if not args.headless and cv2.getWindowProperty(window, cv2.WND_PROP_VISIBLE) < 1:
                break
            if args.seconds and time.perf_counter() - started > args.seconds:
                break
    finally:
        # tell Chimera the face has gone, so a held note ends
        try:
            sock.sendto(packet(seq + 1, False, zero, False, syl.count, 0.0).encode("ascii"), target)
        except OSError:
            pass
        cam.close()
        landmarker.close()
        cv2.destroyAllWindows()


if __name__ == "__main__":
    main()
