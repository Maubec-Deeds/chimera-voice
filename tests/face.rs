//! Face control: the tracker's packets, the listener, and how frames become notes and mouth
//! shapes. No camera needed: the frames are made up here.

use chimera_voice::face::{self, Follower, Frame, Listener, State};
use chimera_voice::lyrics;
use chimera_voice::performer::{Mode, Performer};
use std::net::UdpSocket;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

const FS: f64 = 48000.0;
const BLOCK: usize = 480; // 10 ms

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

/// Energy between two frequencies (Hz), from a plain DFT every 25 Hz under a Hann window.
fn band(x: &[f32], lo: f64, hi: f64) -> f64 {
    let n = x.len();
    let mut total = 0.0;
    let mut f = lo;
    while f <= hi {
        let (mut re, mut im) = (0.0, 0.0);
        for (i, v) in x.iter().enumerate() {
            let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos();
            let ph = 2.0 * std::f64::consts::PI * f * i as f64 / FS;
            re += w * *v as f64 * ph.cos();
            im -= w * *v as f64 * ph.sin();
        }
        total += re * re + im * im;
        f += 25.0;
    }
    total
}

#[test]
fn packets_are_read_and_junk_is_not() {
    let f = face::parse(b"CHFACE 1 42 1 0.5000 -0.2500 1 7 31.5").unwrap();
    assert_eq!(f, Frame { seq: 42, present: true, opening: 0.5, rounding: -0.25, open: true, syllables: 7, age_ms: 31.5 });
    // out-of-range values are held in range
    let f = face::parse(b"CHFACE 1 1 1 3 -9 0 0 0").unwrap();
    assert_eq!((f.opening, f.rounding), (1.0, -1.0));
    for junk in [&b""[..], b"hello", b"CHFACE 2 1 1 0.5 0 1 1 0", b"CHFACE 1 1 1 nan 0 1 1 0", b"CHFACE 1 1 1 0.5", &[0xff, 0xfe, 0x00][..]] {
        assert!(face::parse(junk).is_none(), "{:?}", String::from_utf8_lossy(junk));
    }
}

#[test]
fn the_listener_receives_and_stops() {
    let state = Arc::new(State::default());
    let listener = Listener::start(0, state.clone()).expect("a free port");
    assert_eq!(state.status.load(Ordering::Acquire), face::LISTENING);
    let port = state.port.load(Ordering::Acquire) as u16;
    let out = UdpSocket::bind("127.0.0.1:0").unwrap();
    out.send_to(b"not a face", ("127.0.0.1", port)).unwrap();
    out.send_to(b"CHFACE 1 5 1 0.4 0.6 1 3 20", ("127.0.0.1", port)).unwrap();
    let t = Instant::now();
    while state.packets.load(Ordering::Acquire) == 0 && t.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(5));
    }
    let f = state.load();
    assert_eq!(f.seq, 1, "junk is not counted");
    assert!(f.present && f.open && f.syllables == 3 && (f.rounding - 0.6).abs() < 1e-6);
    // a second listener on the same port is told the port is busy
    let other = Arc::new(State::default());
    assert!(Listener::start(port, other.clone()).is_none());
    assert_eq!(other.status.load(Ordering::Acquire), face::BUSY);
    // dropping it stops the thread (within its 100 ms wake-up)
    let t = Instant::now();
    drop(listener);
    assert!(t.elapsed() < Duration::from_millis(500));
    assert_eq!(state.status.load(Ordering::Acquire), face::OFF);
    // and frees the port
    assert!(Listener::start(port, Arc::new(State::default())).is_some());
}

/// Play frames at 30 a second into a follower, rendering in 10 ms blocks.
struct Rig {
    perf: Performer,
    follower: Follower,
    frame: Frame,
    out: Vec<f32>,
}

impl Rig {
    fn new(mode: Mode) -> Rig {
        let mut perf = Performer::new(FS);
        perf.set_mode(mode);
        Rig { perf, follower: Follower::default(), frame: Frame::default(), out: Vec::new() }
    }

    /// Hold this mouth for `seconds`; `syllable` starts a new syllable first.
    fn mouth(&mut self, opening: f32, rounding: f32, open: bool, syllable: bool, seconds: f64) {
        self.frame.present = true;
        self.frame.opening = opening;
        self.frame.rounding = rounding;
        self.frame.open = open;
        if syllable {
            self.frame.syllables += 1;
        }
        self.run(seconds, true);
    }

    /// Render, with a new frame every third block when `frames`.
    fn run(&mut self, seconds: f64, frames: bool) {
        let blocks = (seconds * FS / BLOCK as f64).round() as usize;
        for b in 0..blocks {
            if frames && b % 3 == 0 {
                self.frame.seq += 1;
            }
            self.follower.apply(&self.frame, &mut self.perf, 57, BLOCK as f64 / FS);
            let at = self.out.len();
            self.out.resize(at + BLOCK, 0.0);
            self.perf.render(&mut self.out[at..]);
        }
    }

    fn tail(&self, seconds: f64) -> f64 {
        rms(&self.out[self.out.len() - (seconds * FS) as usize..])
    }
}

#[test]
fn each_opening_sings_the_next_syllable() {
    let mut r = Rig::new(Mode::Sing);
    r.perf.set_lyrics(lyrics::parse("hello there friend"));
    r.run(0.3, true);
    assert!(r.tail(0.1) < 0.003, "silent before the mouth opens");
    // "hel-": the mouth opens
    r.mouth(0.5, 0.0, true, true, 0.4);
    assert_eq!(r.perf.sounding_index(), Some(0));
    assert!(r.tail(0.2) > 0.02);
    // "-lo": it closes part way and opens again (a new syllable while open: legato)
    r.mouth(0.55, 0.3, true, true, 0.4);
    assert_eq!(r.perf.sounding_index(), Some(1));
    assert!(r.tail(0.3) > 0.02, "the legato note is silent");
    // the mouth shuts: the note ends
    r.mouth(0.0, 0.0, false, false, 0.6);
    assert_eq!(r.perf.sounding_index(), None);
    assert!(r.tail(0.15) < 0.01, "it sings on after the mouth shut: {}", r.tail(0.15));
    // "there"
    r.mouth(0.45, -0.3, true, true, 0.4);
    assert_eq!(r.perf.sounding_index(), Some(2));
    // the tracker stops sending (closed, crashed): the note ends after the timeout
    r.run(1.0, false);
    assert_eq!(r.perf.sounding_index(), None);
    assert!(r.tail(0.15) < 0.01);
    assert_eq!(r.perf.cursor(), 3);
}

#[test]
fn the_mouth_shapes_the_vowel_pad() {
    let mut r = Rig::new(Mode::Vowel);
    r.perf.set_tone(&chimera_voice::performer::Tone { vibrato: 0.0, drift: 0.0, ..Default::default() });
    // a spread mouth sings "ee", a rounded one "oo": "ee" has its second resonance up near
    // 2.3 kHz, "oo" down near 0.9 kHz, so far more of its energy lies high
    let mut high = Vec::new();
    for rounding in [-1.0f32, 1.0] {
        r.mouth(0.12, rounding, true, true, 0.8);
        let held = &r.out[r.out.len() - 4096..];
        assert!(rms(held) > 0.02);
        high.push(band(held, 1700.0, 3200.0) / band(held, 150.0, 3200.0));
    }
    assert!(high[0] > 3.0 * high[1], "ee {:.4} vs oo {:.4} of the energy above 1.7 kHz", high[0], high[1]);
    r.mouth(0.0, 0.0, false, false, 0.5);
    assert!(r.tail(0.1) < 0.005, "the vowel does not stop when the mouth shuts");
    let near = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9;
    assert!(near(face::pad_from_face(0.0, -1.0), (0.02, 0.0)));
    assert!(near(face::pad_from_face(1.0, 1.0), (0.98, 1.0)));
}

#[test]
fn switching_face_control_off_ends_its_note() {
    let mut r = Rig::new(Mode::Vowel);
    r.mouth(0.5, 0.0, true, true, 0.4);
    assert!(r.tail(0.1) > 0.02);
    r.follower.release(&mut r.perf);
    let at = r.out.len();
    r.out.resize(at + (0.5 * FS) as usize, 0.0);
    r.perf.render(&mut r.out[at..]);
    assert!(r.tail(0.1) < 0.005);
}
