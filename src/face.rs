//! Face control: the mouth shape a webcam tracker (`tools/facetrack`) sends over UDP on
//! 127.0.0.1, and what it does to the voice.
//!
//! The tracker sends one line per camera frame:
//! `CHFACE 2 <seq> <face 0|1> <opening 0..1> <rounding -1..1> <open 0|1> <syllables> <age ms>
//! <jaw> <lips> <pucker> <spread> <closed> <teeth> <brows -1..1> <tilt -1..1> <smile>`.
//! `open` says the mouth is open (a note should sound); `syllables` counts the times a new
//! syllable started (the mouth opening again), so a lost packet loses no syllable. The nine
//! values after the age are the detail: the jaw's drop, the gap between the lips, how far the
//! lips are pushed forward and how wide they are, whether they are shut, whether the lower lip
//! is on the teeth, the eyebrows, the head's tilt and the smile (0..1 unless said). An older
//! tracker sends `CHFACE 1` lines without them, which still work as they did.
//!
//! The listener runs on its own thread and writes atomics; the audio thread reads them once a
//! block. Nothing here allocates on the audio thread.

use crate::performer::{FaceMouth, Mode, Performer};
use crate::routes;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub const DEFAULT_PORT: u16 = 9123;

/// The listener's state, as the editor shows it.
pub const OFF: u32 = 0;
pub const LISTENING: u32 = 1;
/// another program (another Chimera, most likely) has the port
pub const BUSY: u32 = 2;

/// One frame from the tracker.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    pub seq: u32,
    pub present: bool,
    /// 0 shut … 1 wide open
    pub opening: f32,
    /// −1 spread ("ee") … +1 rounded ("oo")
    pub rounding: f32,
    pub open: bool,
    pub syllables: u32,
    /// how old the camera image was when the tracker sent this (ms)
    pub age_ms: f32,
    /// whether the tracker sent the detail below (a version-2 tracker)
    pub detail: bool,
    /// the detail, in the packet's order: see the names below
    pub extra: [f32; EXTRAS],
}

/// How many values of detail a version-2 frame carries, and where each one is.
pub const EXTRAS: usize = 9;
pub const JAW: usize = 0;
pub const LIPS: usize = 1;
pub const PUCKER: usize = 2;
pub const SPREAD: usize = 3;
pub const CLOSED: usize = 4;
pub const TEETH: usize = 5;
pub const BROWS: usize = 6;
pub const TILT: usize = 7;
pub const SMILE: usize = 8;

impl Frame {
    pub fn get(&self, which: usize) -> f64 {
        self.extra[which] as f64
    }
}

/// Read one packet; `None` for anything that is not a version-1 or version-2 face frame.
pub fn parse(packet: &[u8]) -> Option<Frame> {
    let text = std::str::from_utf8(packet).ok()?;
    let mut it = text.split_ascii_whitespace();
    if it.next()? != "CHFACE" {
        return None;
    }
    let detail = match it.next()? {
        "1" => false,
        "2" => true,
        _ => return None,
    };
    let seq = it.next()?.parse().ok()?;
    let present = it.next()? == "1";
    let opening: f32 = it.next()?.parse().ok()?;
    let rounding: f32 = it.next()?.parse().ok()?;
    let open = it.next()? == "1";
    let syllables = it.next()?.parse().ok()?;
    let age_ms: f32 = it.next()?.parse().ok()?;
    if !(opening.is_finite() && rounding.is_finite() && age_ms.is_finite()) {
        return None;
    }
    let mut extra = [0.0f32; EXTRAS];
    if detail {
        for (i, x) in extra.iter_mut().enumerate() {
            let v: f32 = it.next()?.parse().ok()?;
            if !v.is_finite() {
                return None;
            }
            *x = v.clamp(if i == BROWS || i == TILT { -1.0 } else { 0.0 }, 1.0);
        }
    }
    Some(Frame { seq, present, opening: opening.clamp(0.0, 1.0), rounding: rounding.clamp(-1.0, 1.0), open, syllables, age_ms, detail, extra })
}

/// The latest frame, shared between the listener, the audio thread and the editor.
#[derive(Default)]
pub struct State {
    pub status: AtomicU32,
    /// packets received (0: none yet)
    pub packets: AtomicU32,
    present: AtomicBool,
    opening: AtomicU32,
    rounding: AtomicU32,
    open: AtomicBool,
    syllables: AtomicU32,
    age_ms: AtomicU32,
    detail: AtomicBool,
    extra: [AtomicU32; EXTRAS],
    /// the port actually listened on
    pub port: AtomicU32,
}

impl State {
    pub fn store(&self, f: &Frame) {
        self.present.store(f.present, Ordering::Relaxed);
        self.opening.store(f.opening.to_bits(), Ordering::Relaxed);
        self.rounding.store(f.rounding.to_bits(), Ordering::Relaxed);
        self.open.store(f.open, Ordering::Relaxed);
        self.syllables.store(f.syllables, Ordering::Relaxed);
        self.age_ms.store(f.age_ms.to_bits(), Ordering::Relaxed);
        self.detail.store(f.detail, Ordering::Relaxed);
        for (a, x) in self.extra.iter().zip(f.extra.iter()) {
            a.store(x.to_bits(), Ordering::Relaxed);
        }
        // last, so a reader that sees the new count sees the values with it
        self.packets.fetch_add(1, Ordering::Release);
    }

    pub fn load(&self) -> Frame {
        let seq = self.packets.load(Ordering::Acquire);
        Frame {
            seq,
            present: self.present.load(Ordering::Relaxed),
            opening: f32::from_bits(self.opening.load(Ordering::Relaxed)),
            rounding: f32::from_bits(self.rounding.load(Ordering::Relaxed)),
            open: self.open.load(Ordering::Relaxed),
            syllables: self.syllables.load(Ordering::Relaxed),
            age_ms: f32::from_bits(self.age_ms.load(Ordering::Relaxed)),
            detail: self.detail.load(Ordering::Relaxed),
            extra: std::array::from_fn(|i| f32::from_bits(self.extra[i].load(Ordering::Relaxed))),
        }
    }
}

/// Listens for the tracker on its own thread until dropped.
pub struct Listener {
    running: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Listener {
    /// Listen on 127.0.0.1:`port` (0: any free port; `state.port` says which). When the port is
    /// taken, `state.status` says so and `None` comes back.
    pub fn start(port: u16, state: Arc<State>) -> Option<Listener> {
        let socket = match UdpSocket::bind(("127.0.0.1", port)) {
            Ok(s) => s,
            Err(_) => {
                state.status.store(BUSY, Ordering::Release);
                return None;
            }
        };
        // wake up now and then to see whether to stop
        socket.set_read_timeout(Some(Duration::from_millis(100))).ok()?;
        state.port.store(socket.local_addr().map_or(0, |a| a.port() as u32), Ordering::Release);
        state.status.store(LISTENING, Ordering::Release);
        let running = Arc::new(AtomicBool::new(true));
        let run = running.clone();
        let thread = std::thread::Builder::new()
            .name("chimera-face".into())
            .spawn(move || {
                let mut buf = [0u8; 256];
                while run.load(Ordering::Acquire) {
                    if let Ok((n, _)) = socket.recv_from(&mut buf) {
                        if let Some(f) = parse(&buf[..n]) {
                            state.store(&f);
                        }
                    }
                }
                state.status.store(OFF, Ordering::Release);
            })
            .ok()?;
        Some(Listener { running, thread: Some(thread) })
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Where the face puts the vowel pad's handle (x: 0 front … 1 back, y: 0 close … 1 open): a
/// spread mouth is a front vowel ("ee"), a rounded one a back vowel ("oo"), and the opening is
/// the vowel's openness.
pub fn pad_from_face(opening: f32, rounding: f32) -> (f64, f64) {
    let x = 0.5 + 0.48 * rounding as f64;
    let y = (opening as f64 * 1.15).min(1.0);
    (x.clamp(0.0, 1.0), y)
}

/// The same for a whole frame: a smile also pulls the vowel forward, standing in for the tongue,
/// which no camera sees.
pub fn pad_from_frame(f: &Frame) -> (f64, f64) {
    let (x, y) = pad_from_face(f.opening, f.rounding);
    ((x - 0.22 * f.get(SMILE)).clamp(0.0, 1.0), y)
}

/// The face's signals for the routing table (see `routes`), smoothed, and what the mouth does to
/// the breath on its own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Expression {
    /// singing: the mouth's opening scales the lung pressure (1: no change)
    pub pressure: f64,
    /// open, jaw, round, spread, lip on teeth, smile (0..1), eyebrows and head tilt (−1..1); all
    /// 0 while no face is tracked
    pub sources: [f64; routes::NSRC],
}

impl Default for Expression {
    fn default() -> Self {
        Expression { pressure: 1.0, sources: [0.0; routes::NSRC] }
    }
}

impl Expression {
    /// How much the face moves each setting through this routing (in the settings' own units).
    pub fn mods(&self, routes: &routes::Routes) -> [f64; routes::NDST] {
        routes::mods(routes, &self.sources)
    }
}

/// How far a full head tilt bends the pitch with the default routing (semitones).
pub const TILT_BEND: f64 = 2.0;

/// Turns the tracker's frames into notes and mouth shapes, on the audio thread.
pub struct Follower {
    last_seq: u32,
    syllables: u32,
    open: bool,
    /// how long since a frame came (s)
    quiet: f64,
    /// the key the face is playing, while it plays one
    playing: Option<u8>,
    /// how long the lips have been shut while a note sounds (s)
    shut_for: f64,
    /// the opening at the last frame, when it came, and how fast it was growing (1/s)
    last_opening: f64,
    since_frame: f64,
    rate: f64,
    /// how hard the voice breathes (1: as the velocity set it), smoothed
    effort: f64,
    expression: Expression,
}

/// After this long without a frame the face counts as gone (s).
const TIMEOUT: f64 = 0.4;
/// The velocity the face plays with when it cannot tell how fast the mouth opened.
const VELOCITY: f64 = 0.75;
/// Shut lips hum for this long, then the note ends (s).
const HUM: f64 = 0.45;
/// Shut for less than this the lips are a "b"; after it the nose opens and they hum an "m" (s).
const NASAL_AFTER: f64 = 0.07;
/// How quickly the expression and the breath follow the face (s).
const SMOOTH: f64 = 0.05;

impl Default for Follower {
    fn default() -> Self {
        Follower {
            last_seq: 0,
            syllables: 0,
            open: false,
            quiet: f64::INFINITY,
            playing: None,
            shut_for: 0.0,
            last_opening: 0.0,
            since_frame: 0.0,
            rate: 0.0,
            effort: 1.0,
            expression: Expression::default(),
        }
    }
}

impl Follower {
    /// Whether frames are arriving.
    pub fn live(&self) -> bool {
        self.quiet < TIMEOUT
    }

    /// What the face adds to the tone now.
    pub fn expression(&self) -> Expression {
        self.expression
    }

    /// Follow the newest frame. `key` is the note the face sings (the last key played); `dt` is
    /// how long this block lasts (s).
    pub fn apply(&mut self, f: &Frame, perf: &mut Performer, key: u8, dt: f64) {
        let fresh = f.seq != self.last_seq;
        self.since_frame += dt;
        if fresh {
            self.last_seq = f.seq;
            self.quiet = 0.0;
            // how fast the mouth is opening, for the note's attack
            let opening = f.opening as f64;
            self.rate = (opening - self.last_opening) / self.since_frame.clamp(0.005, 0.2);
            self.last_opening = opening;
            self.since_frame = 0.0;
        } else {
            self.quiet += dt;
        }
        let seen = self.live() && f.present;
        let mode = perf.mode();
        let detail = seen && f.detail;
        let new_syllables = f.syllables.wrapping_sub(self.syllables);
        self.syllables = f.syllables;
        let smooth = 1.0 - (-dt / SMOOTH).exp();

        // ---- the mouth: notes, and in vowel mode the lips themselves
        let mut effort = 1.0;
        let open = if mode == Mode::Vowel && detail {
            let (x, y) = pad_from_frame(f);
            perf.set_pad(x, y);
            let shut = f.get(CLOSED) > 0.7;
            let sounding = if self.playing.is_none() {
                // any real parting of the lips starts the note, softly if it is small
                self.shut_for = 0.0;
                f.get(CLOSED) < 0.3 && f.opening > 0.08
            } else {
                self.shut_for = if shut { self.shut_for + dt } else { 0.0 };
                self.shut_for < HUM
            };
            let nasal = ((self.shut_for - NASAL_AFTER) / 0.08).clamp(0.0, 1.0);
            perf.set_face_mouth(Some(FaceMouth {
                jaw: f.get(JAW),
                lips: f.get(LIPS),
                pucker: f.get(PUCKER),
                spread: f.get(SPREAD),
                closed: ((f.get(CLOSED) - 0.5) / 0.4).clamp(0.0, 1.0),
                teeth: f.get(TEETH),
                nasal,
            }));
            // a half-open mouth sings quietly, a wide one loudly
            effort = 0.55 + 0.6 * (f.opening as f64).powf(0.6);
            sounding
        } else {
            perf.set_face_mouth(None);
            if mode == Mode::Vowel && seen {
                let (x, y) = pad_from_face(f.opening, f.rounding);
                perf.set_pad(x, y);
            }
            if mode == Mode::Creature && detail {
                effort = 0.6 + 0.55 * (f.opening as f64).powf(0.6);
            }
            seen && f.open
        };
        self.effort += (effort - self.effort) * smooth;
        perf.set_effort(self.effort);
        // a mouth that snaps open attacks harder than one that eases open
        let velocity = if detail { (0.5 + 0.09 * self.rate).clamp(0.45, 1.0) } else { VELOCITY };
        match mode {
            Mode::Sing => {
                // every new syllable is a note: legato while the mouth stays open
                if open && (!self.open || (fresh && new_syllables > 0)) {
                    if let Some(k) = self.playing.take() {
                        // a new note while the old one sounds sings legato: the old key goes up after
                        perf.note_on(key, velocity);
                        if k != key {
                            perf.note_off(k);
                        }
                    } else {
                        perf.note_on(key, velocity);
                    }
                    self.playing = Some(key);
                }
            }
            Mode::Vowel | Mode::Creature => {
                if open && !self.open {
                    perf.note_on(key, velocity);
                    self.playing = Some(key);
                }
            }
            Mode::Beatbox => {}
        }
        if !open {
            if let Some(k) = self.playing.take() {
                perf.note_off(k);
            }
        }
        self.open = open;

        // ---- the rest of the face: pitch and tone, in every mode
        let mut want = Expression::default();
        if detail {
            if mode == Mode::Sing {
                want.pressure = 0.8 + 0.45 * f.opening as f64;
            }
            want.sources = [f.opening as f64, f.get(JAW), f.get(PUCKER), f.get(SPREAD), f.get(TEETH), f.get(SMILE), f.get(BROWS), f.get(TILT)];
        }
        let e = &mut self.expression;
        e.pressure += (want.pressure - e.pressure) * smooth;
        for (now, to) in e.sources.iter_mut().zip(want.sources) {
            *now += (to - *now) * smooth;
        }
    }

    /// Stop whatever the face is playing and hand the voice back (face control switched off).
    pub fn release(&mut self, perf: &mut Performer) {
        if let Some(k) = self.playing.take() {
            perf.note_off(k);
        }
        perf.set_face_mouth(None);
        perf.set_effort(1.0);
        self.open = false;
        self.effort = 1.0;
        self.expression = Expression::default();
    }
}
