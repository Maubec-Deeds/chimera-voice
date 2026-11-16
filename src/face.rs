//! Face control: the mouth shape a webcam tracker (`tools/facetrack`) sends over UDP on
//! 127.0.0.1, and what it does to the voice.
//!
//! The tracker sends one line per camera frame:
//! `CHFACE 1 <seq> <face 0|1> <opening 0..1> <rounding -1..1> <open 0|1> <syllables> <age ms>`.
//! `open` says the mouth is open (a note should sound); `syllables` counts the times a new
//! syllable started (the mouth opening again), so a lost packet loses no syllable.
//!
//! The listener runs on its own thread and writes atomics; the audio thread reads them once a
//! block. Nothing here allocates on the audio thread.

use crate::performer::{Mode, Performer};
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
}

/// Read one packet; `None` for anything that is not a version-1 face frame.
pub fn parse(packet: &[u8]) -> Option<Frame> {
    let text = std::str::from_utf8(packet).ok()?;
    let mut it = text.split_ascii_whitespace();
    if it.next()? != "CHFACE" || it.next()? != "1" {
        return None;
    }
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
    Some(Frame { seq, present, opening: opening.clamp(0.0, 1.0), rounding: rounding.clamp(-1.0, 1.0), open, syllables, age_ms })
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

/// Turns the tracker's frames into notes and mouth shapes, on the audio thread.
pub struct Follower {
    last_seq: u32,
    syllables: u32,
    open: bool,
    /// how long since a frame came (s)
    quiet: f64,
    /// the key the face is playing, while it plays one
    playing: Option<u8>,
}

/// After this long without a frame the face counts as gone (s).
const TIMEOUT: f64 = 0.4;
/// The velocity the face plays with.
const VELOCITY: f64 = 0.75;

impl Default for Follower {
    fn default() -> Self {
        Follower { last_seq: 0, syllables: 0, open: false, quiet: f64::INFINITY, playing: None }
    }
}

impl Follower {
    /// Whether frames are arriving.
    pub fn live(&self) -> bool {
        self.quiet < TIMEOUT
    }

    /// Follow the newest frame. `key` is the note the face sings (the last key played); `dt` is
    /// how long this block lasts (s).
    pub fn apply(&mut self, f: &Frame, perf: &mut Performer, key: u8, dt: f64) {
        let fresh = f.seq != self.last_seq;
        if fresh {
            self.last_seq = f.seq;
            self.quiet = 0.0;
        } else {
            self.quiet += dt;
        }
        let open = self.live() && f.present && f.open;
        let mode = perf.mode();
        if mode == Mode::Vowel && self.live() && f.present {
            let (x, y) = pad_from_face(f.opening, f.rounding);
            perf.set_pad(x, y);
        }
        let new_syllables = f.syllables.wrapping_sub(self.syllables);
        self.syllables = f.syllables;
        match mode {
            Mode::Sing => {
                // every new syllable is a note: legato while the mouth stays open
                if open && (!self.open || (fresh && new_syllables > 0)) {
                    if let Some(k) = self.playing.take() {
                        // a new note while the old one sounds sings legato: the old key goes up after
                        perf.note_on(key, VELOCITY);
                        if k != key {
                            perf.note_off(k);
                        }
                    } else {
                        perf.note_on(key, VELOCITY);
                    }
                    self.playing = Some(key);
                }
            }
            Mode::Vowel | Mode::Creature => {
                if open && !self.open {
                    perf.note_on(key, VELOCITY);
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
    }

    /// Stop whatever the face is playing (face control switched off).
    pub fn release(&mut self, perf: &mut Performer) {
        if let Some(k) = self.playing.take() {
            perf.note_off(k);
        }
        self.open = false;
    }
}
