//! What the audio thread, the editor and the background task share.

use crate::performer::Built;
use chimera::anatomy::human::{BodyMix, N_ARTIC, NEUTRAL};
use chimera::phonetics::Syllable;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};

pub const SCOPE_LEN: usize = 512;
pub const TUBES: usize = 8;
pub const TUBE_POINTS: usize = 48;

/// One of a creature's tubes, resampled to a fixed number of points.
#[derive(Clone, Copy)]
pub struct TubeViz {
    pub name: &'static str,
    /// cross-section areas (cm²) and sound pressure (dyn/cm², RMS) along the tube
    pub areas: [f32; TUBE_POINTS],
    pub rms: [f32; TUBE_POINTS],
}

/// A snapshot of the voice for the editor's displays.
pub struct Viz {
    pub artic: [f32; N_ARTIC],
    pub body: BodyMix,
    pub f0: f32,
    pub voicing: f32,
    pub level: f32,
    /// the latest output samples
    pub scope: [f32; SCOPE_LEN],
    /// labels of the gestures active now (the phonemes being said)
    pub active: String,
    /// the creature playing (its id; empty for the human voice), its tubes and its call
    pub creature: &'static str,
    pub tubes: [TubeViz; TUBES],
    pub tube_count: usize,
    pub call: Option<&'static str>,
    pub gate: f32,
}

impl Default for Viz {
    fn default() -> Self {
        let mut artic = [0.0; N_ARTIC];
        for (i, x) in NEUTRAL.to_array().iter().enumerate() {
            artic[i] = *x as f32;
        }
        Viz {
            artic,
            body: BodyMix::MALE,
            f0: 0.0,
            voicing: 0.0,
            level: 0.0,
            scope: [0.0; SCOPE_LEN],
            active: String::with_capacity(128),
            creature: "",
            tubes: [TubeViz { name: "", areas: [0.0; TUBE_POINTS], rms: [0.0; TUBE_POINTS] }; TUBES],
            tube_count: 0,
            call: None,
            gate: 0.0,
        }
    }
}

/// A mailbox to the audio thread. It takes `incoming` and leaves what it replaced in `outgoing`,
/// so nothing big is freed on the audio thread.
pub struct Mailbox<T> {
    pub incoming: Option<T>,
    pub outgoing: Option<T>,
}

impl<T> Default for Mailbox<T> {
    fn default() -> Self {
        Mailbox { incoming: None, outgoing: None }
    }
}

/// What an engine was built for: the body, eco quality, and the creature (an index into the
/// bestiary) if it plays one.
pub type VoiceSpec = (BodyMix, bool, Option<usize>);

#[derive(Default)]
pub struct Shared {
    /// every phrase's syllables
    pub lyrics_box: Mutex<Mailbox<Vec<Vec<Syllable>>>>,
    pub lyrics_ready: AtomicBool,
    pub engine_box: Mutex<Mailbox<(VoiceSpec, Built)>>,
    pub engine_ready: AtomicBool,
    /// bumped whenever the phrases change outside the editor (a preset load)
    pub state_version: AtomicI64,
    /// the phrase being sung, the syllable the next note sings, the syllable sounding (−1: none)
    pub phrase: AtomicI64,
    pub cursor: AtomicI64,
    pub sounding: AtomicI64,
    /// the editor asks for the cursor to move here, or for this phrase (−1: no request)
    pub cursor_request: AtomicI64,
    pub phrase_request: AtomicI64,
    /// the editor plays this drum sound (−1: no request)
    pub beat_request: AtomicI64,
    /// the last drum sound played and how many have been
    pub beat_last: AtomicU32,
    pub beat_count: AtomicU32,
    /// the editor asks for silence
    pub panic: AtomicBool,
    /// the webcam tracker's latest frame, and the listener that receives it (started and stopped
    /// by the background task, never on the audio thread)
    pub face: std::sync::Arc<crate::face::State>,
    pub face_listener: Mutex<Option<crate::face::Listener>>,
    pub viz: Mutex<Viz>,
    /// text a file dialog loaded, for the editor to take
    pub loaded_text: Mutex<Option<String>>,
}

impl Shared {
    pub fn new() -> Shared {
        let s = Shared::default();
        for a in [&s.sounding, &s.cursor_request, &s.phrase_request, &s.beat_request] {
            a.store(-1, Ordering::Relaxed);
        }
        s
    }

    /// Hand new lyrics to the audio thread (called off the audio thread).
    pub fn post_lyrics(&self, phrases: Vec<Vec<Syllable>>) {
        let mut b = self.lyrics_box.lock();
        b.outgoing = None;
        b.incoming = Some(phrases);
        drop(b);
        self.lyrics_ready.store(true, Ordering::Release);
    }
}
