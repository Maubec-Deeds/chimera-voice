"""Chimera Voice face tracker: watches your mouth through a webcam and sends its shape to Chimera.

MediaPipe's face landmarker finds the lips in each camera frame. From them this works out how
open the mouth is, how rounded or spread the lips are, and when a new syllable starts (the
mouth opening again after closing part way). It sends that to every Chimera Voice on this computer
with its FACE switch on, over UDP on 127.0.0.1 (nothing leaves the machine).

Keys in the preview window:
  C  calibrate closed: relax with your lips together, then press
  O  calibrate open:   say a wide "aah", then press
  U  calibrate round:  push your lips into an "oo", then press
  E  calibrate spread: smile into an "ee", then press
  V  video on/off (off saves processor time; the meters stay)
  Q or Esc  quit

The calibration is saved next to this script and loaded next time.
"""

import argparse
import json
import math
import os
import socket
import sys
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
LIPS_OUTER = [61, 185, 40, 39, 37, 0, 267, 269, 270, 409, 291, 375, 321, 405, 314, 17, 84, 181, 91, 146, 61]
LIPS_INNER = [78, 191, 80, 81, 82, 13, 312, 311, 310, 415, 308, 324, 318, 402, 317, 14, 87, 178, 88, 95, 78]

DEFAULT_CAL = {
    # lip gap / face width with the mouth shut and wide open
    "gap_closed": 0.005,
    "gap_open": 0.16,
    # mouth width / face width for a rounded "oo" and a spread "ee"
    "width_round": 0.30,
    "width_spread": 0.48,
}


class OneEuro:
    """The 1€ filter (Casiez et al. 2012): smooths jitter when the value is still, follows fast
    when it moves, so it adds little delay where it matters."""

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
    """Finds syllable starts in the mouth's opening (0 shut … 1 wide open): a new one each time the
    mouth opens by `rise` from the lowest it has been since the last one. The mouth counts as
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


def load_calibration():
    cal = dict(DEFAULT_CAL)
    try:
        with open(CALIBRATION, encoding="utf-8") as f:
            cal.update({k: float(v) for k, v in json.load(f).items() if k in cal})
    except (OSError, ValueError):
        pass
    return cal


def save_calibration(cal):
    try:
        with open(CALIBRATION, "w", encoding="utf-8") as f:
            json.dump(cal, f, indent=2)
    except OSError as e:
        print("could not save the calibration:", e)


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


def meter(img, x, y, w, label, value, color, mark=None):
    cv2.putText(img, label, (x, y - 4), cv2.FONT_HERSHEY_SIMPLEX, 0.42, (200, 220, 230), 1, cv2.LINE_AA)
    cv2.rectangle(img, (x, y), (x + w, y + 10), (60, 60, 60), 1)
    cv2.rectangle(img, (x + 1, y + 1), (x + 1 + int((w - 2) * max(0.0, min(1.0, value))), y + 9), color, -1)
    if mark is not None:
        mx = x + int(w * mark)
        cv2.line(img, (mx, y - 2), (mx, y + 12), (255, 255, 255), 1)


def main():
    ap = argparse.ArgumentParser(description="Send your mouth's shape from a webcam to Chimera Voice.")
    ap.add_argument("--camera", type=int, default=0, help="camera number (0 is usually the built-in one)")
    ap.add_argument("--port", type=int, default=9123, help="UDP port Chimera listens on (default 9123)")
    ap.add_argument("--width", type=int, default=640)
    ap.add_argument("--height", type=int, default=480)
    ap.add_argument("--no-video", action="store_true", help="start with the video preview off")
    ap.add_argument("--seconds", type=float, default=0, help="stop after this many seconds (for tests)")
    ap.add_argument("--headless", action="store_true", help="no window at all; print the values instead")
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
    cal = load_calibration()
    f_open, f_round = OneEuro(), OneEuro()
    syl = Syllables()
    video = not args.no_video
    serial = 0
    seq = 0
    track_ms = 0.0
    fps, fps_frames, fps_t = 0.0, 0, time.perf_counter()
    started = time.perf_counter()
    raw = {"gap": 0.0, "width": 0.0}
    face = False
    window = "Chimera face tracker"
    if not args.headless:
        cv2.namedWindow(window, cv2.WINDOW_AUTOSIZE)
    last_print = 0.0

    try:
        while True:
            frame, serial, stamp = cam.latest(serial)
            if frame is None:
                time.sleep(0.002)
                key = cv2.waitKey(1) & 0xFF if not args.headless else -1
            else:
                t0 = time.perf_counter()
                rgb = cv2.cvtColor(frame, cv2.COLOR_BGR2RGB)
                result = landmarker.detect_for_video(mp.Image(image_format=mp.ImageFormat.SRGB, data=rgb), int(stamp * 1000))
                face = bool(result.face_landmarks)
                opening = rounding = 0.0
                if face:
                    lm = result.face_landmarks[0]
                    h, w = frame.shape[:2]
                    p = lambda i: (lm[i].x * w, lm[i].y * h)
                    dist = lambda a, b: math.dist(p(a), p(b))
                    scale = max(1.0, dist(FACE_LEFT, FACE_RIGHT))
                    raw["gap"] = dist(LIP_TOP, LIP_BOTTOM) / scale
                    raw["width"] = dist(MOUTH_LEFT, MOUTH_RIGHT) / scale
                    shapes = {c.category_name: c.score for c in result.face_blendshapes[0]}
                    # opening: the lip gap between the calibrated shut and wide-open mouths, with
                    # the jaw's own measure as a second opinion
                    by_gap = (raw["gap"] - cal["gap_closed"]) / max(1e-4, cal["gap_open"] - cal["gap_closed"])
                    opening = max(0.0, min(1.0, 0.75 * by_gap + 0.25 * min(1.0, shapes.get("jawOpen", 0.0) / 0.6)))
                    # rounding: -1 spread ("ee") … +1 rounded ("oo"), from the mouth's width and
                    # the pucker and funnel shapes
                    span = max(1e-4, cal["width_spread"] - cal["width_round"])
                    by_width = 1.0 - 2.0 * (raw["width"] - cal["width_round"]) / span
                    pucker = shapes.get("mouthPucker", 0.0) + 0.6 * shapes.get("mouthFunnel", 0.0)
                    rounding = max(-1.0, min(1.0, 0.6 * by_width + 0.8 * pucker))
                now = time.perf_counter()
                opening = f_open(opening, now)
                rounding = f_round(rounding, now)
                is_open = syl(opening) if face else syl(0.0)
                track_ms = 0.8 * track_ms + 0.2 * (now - t0) * 1000
                age_ms = (now - stamp) * 1000
                seq += 1
                msg = f"CHFACE 1 {seq} {int(face)} {opening:.4f} {rounding:.4f} {int(is_open)} {syl.count} {age_ms:.1f}"
                try:
                    sock.sendto(msg.encode("ascii"), target)
                except OSError:
                    pass
                fps_frames += 1
                if now - fps_t >= 1.0:
                    fps, fps_frames, fps_t = fps_frames / (now - fps_t), 0, now
                if args.headless:
                    if now - last_print >= 0.5:
                        last_print = now
                        print(f"{fps:4.1f} fps  track {track_ms:4.1f} ms  face {int(face)}  open {opening:.2f}  round {rounding:+.2f}  syllables {syl.count}", flush=True)
                    key = -1
                else:
                    # video off: just the meters, in a strip
                    view = cv2.flip(frame, 1) if video else frame[:100] * 0
                    if face and video:
                        h, w = frame.shape[:2]
                        lm = result.face_landmarks[0]
                        pts = lambda idx: [(int((1 - lm[i].x) * w), int(lm[i].y * h)) for i in idx]

                        cv2.polylines(view, [np.array(pts(LIPS_OUTER), np.int32)], False, (207, 115, 255), 2, cv2.LINE_AA)
                        cv2.polylines(view, [np.array(pts(LIPS_INNER), np.int32)], False, (64, 179, 255), 1, cv2.LINE_AA)
                    hud = view
                    cv2.rectangle(hud, (0, 0), (hud.shape[1], 94), (8, 12, 18), -1)
                    status = "FACE" if face else "NO FACE - look at the camera, light your face"
                    cv2.putText(hud, f"{fps:4.1f} fps   tracking {track_ms:4.1f} ms   {status}", (10, 18), cv2.FONT_HERSHEY_SIMPLEX, 0.5, (130, 230, 150) if face else (80, 120, 255), 1, cv2.LINE_AA)
                    meter(hud, 10, 42, 200, "OPEN", opening, (64, 179, 255), syl.rise)
                    meter(hud, 230, 42, 200, "SPREAD  <  >  ROUND", (rounding + 1) / 2, (207, 115, 255), 0.5)
                    cv2.circle(hud, (460, 46), 9, (64, 179, 255) if is_open else (60, 60, 60), -1)
                    cv2.putText(hud, f"syllables {syl.count}", (476, 51), cv2.FONT_HERSHEY_SIMPLEX, 0.45, (200, 220, 230), 1, cv2.LINE_AA)
                    cv2.putText(hud, "calibrate: C shut  O aah  U oo  E ee    V video    Q quit", (10, 82), cv2.FONT_HERSHEY_SIMPLEX, 0.42, (150, 170, 180), 1, cv2.LINE_AA)
                    cv2.imshow(window, hud)
                    key = cv2.waitKey(1) & 0xFF
            if key in (ord("q"), ord("Q"), 27):
                break
            if key in (ord("v"), ord("V")):
                video = not video
            if face and key in (ord("c"), ord("C"), ord("o"), ord("O"), ord("u"), ord("U"), ord("e"), ord("E")):
                k = chr(key).lower()
                field = {"c": ("gap_closed", "gap"), "o": ("gap_open", "gap"), "u": ("width_round", "width"), "e": ("width_spread", "width")}[k]
                cal[field[0]] = raw[field[1]]
                save_calibration(cal)
                print(f"calibrated {field[0]} = {raw[field[1]]:.4f}")
            if not args.headless and cv2.getWindowProperty(window, cv2.WND_PROP_VISIBLE) < 1:
                break
            if args.seconds and time.perf_counter() - started > args.seconds:
                break
    finally:
        # tell Chimera the face has gone, so a held note ends
        try:
            sock.sendto(f"CHFACE 1 {seq + 1} 0 0 0 0 {syl.count} 0".encode("ascii"), target)
        except OSError:
            pass
        cam.close()
        landmarker.close()
        cv2.destroyAllWindows()


if __name__ == "__main__":
    main()
