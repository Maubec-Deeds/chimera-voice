//! What the audio thread, the editor and the background task share.

use chimera::anatomy::human::{BodyMix, N_ARTIC, NEUTRAL};
use chimera::engine::Engine;
use chimera::phonetics::Syllable;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

pub const SCOPE_LEN: usize = 512;

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
}

impl Default for Viz {
    fn default() -> Self {
        let mut artic = [0.0; N_ARTIC];
        for (i, x) in NEUTRAL.to_array().iter().enumerate() {
            artic[i] = *x as f32;
        }
        Viz { artic, body: BodyMix::MALE, f0: 0.0, voicing: 0.0, level: 0.0, scope: [0.0; SCOPE_LEN], active: String::with_capacity(128) }
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

#[derive(Default)]
pub struct Shared {
    pub lyrics_box: Mutex<Mailbox<Vec<Syllable>>>,
    pub lyrics_ready: AtomicBool,
    pub engine_box: Mutex<Mailbox<(BodyMix, bool, Engine)>>,
    pub engine_ready: AtomicBool,
    /// the syllables as the editor shows them
    pub chips: Mutex<Vec<String>>,
    /// bumped whenever `chips` changes outside the editor (a preset load)
    pub chips_version: AtomicI64,
    /// the syllable the next note sings
    pub cursor: AtomicI64,
    /// the syllable being sung (−1: none)
    pub sounding: AtomicI64,
    /// the editor asks for the cursor to move here (−1: no request)
    pub cursor_request: AtomicI64,
    /// the editor asks for silence
    pub panic: AtomicBool,
    pub viz: Mutex<Viz>,
}

impl Shared {
    pub fn new() -> Shared {
        let s = Shared::default();
        s.sounding.store(-1, Ordering::Relaxed);
        s.cursor_request.store(-1, Ordering::Relaxed);
        s
    }

    /// Hand new lyrics to the audio thread and show them (called off the audio thread).
    pub fn post_lyrics(&self, syllables: Vec<Syllable>) {
        *self.chips.lock() = syllables.iter().map(crate::lyrics::written).collect();
        let mut b = self.lyrics_box.lock();
        b.outgoing = None;
        b.incoming = Some(syllables);
        drop(b);
        self.lyrics_ready.store(true, Ordering::Release);
    }
}
