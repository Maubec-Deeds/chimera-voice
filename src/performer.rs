//! The performer: turns notes into the engine's commands. No plugin or GUI code in here, so it
//! can be rendered and tested headlessly.
//!
//! Sing mode is a port of the web app's live Sing controller (`src/ui/singing.ts` upstream): each
//! key sings the next syllable of the lyrics at its pitch; overlapping keys sing legato; a lone
//! `-` in the lyrics holds the vowel onto the next pitch (melisma); releasing the sounding key
//! while another is still down glides back to that key.
//!
//! Vowel mode plays the voice like a monosynth: the keys set pitch and breath, the pad sets the
//! mouth's posture between the calibrated vowels.

use chimera::anatomy::human::{ArticPatch, BodyMix};
use chimera::control::score::ControlKey;
use chimera::engine::{default_f0, Engine};
use chimera::math::midi_to_hz;
use chimera::phonetics::planner::{vowel_artic, EXHALE_LABEL};
use chimera::phonetics::sing::{plan_melisma, plan_note_off, plan_note_on, LegatoFrom, MelismaOptions, NoteOnOptions};
use chimera::phonetics::vowel_data::VOWELS;
use chimera::phonetics::{Segment, Syllable};
use chimera::voice::human_voice::{oversample_for, HumanVoice, SourcePatch, VoiceTelemetry};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// keys sing the lyrics, one syllable per note
    Sing,
    /// keys sing the vowel under the pad's handle
    Vowel,
}

/// The voice's tone, as the knobs set it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tone {
    /// lung pressure (cmH2O)
    pub pressure: f64,
    /// 0 lax … 1 pressed
    pub tension: f64,
    pub brightness: f64,
    pub breathiness: f64,
    pub roughness: f64,
    pub fry: f64,
    /// vibrato depth on held notes (cents)
    pub vibrato: f64,
    /// vibrato rate (Hz)
    pub vibrato_rate: f64,
    /// pitch glide time constant (s)
    pub glide: f64,
    /// random pitch drift (cents)
    pub drift: f64,
    pub whisper: bool,
}

impl Default for Tone {
    fn default() -> Self {
        Tone { pressure: 8.0, tension: 0.5, brightness: 0.25, breathiness: 0.15, roughness: 0.1, fry: 0.0, vibrato: 35.0, vibrato_rate: 6.0, glide: 0.06, drift: 5.0, whisper: false }
    }
}

/// The vowels on the pad: one per position of the vowel chart.
pub const PAD_VOWELS: [&str; 13] = ["i", "ɪ", "e", "ɛ", "æ", "a", "ɑ", "ɔ", "o", "ʊ", "u", "ʌ", "ə"];

/// Where a pad vowel sits: (x: 0 front … 1 back, y: 0 close … 1 open).
pub fn pad_position(ipa: &str) -> (f64, f64) {
    VOWELS.iter().find(|v| v.ipa == ipa).map_or((0.5, 0.5), |v| (v.back, v.height))
}

/// The pad vowel nearest a position.
pub fn nearest_pad_vowel(x: f64, y: f64) -> &'static str {
    let mut best = (f64::INFINITY, PAD_VOWELS[0]);
    for ipa in PAD_VOWELS {
        let (vx, vy) = pad_position(ipa);
        let d = (x - vx) * (x - vx) + (y - vy) * (y - vy);
        if d < best.0 {
            best = (d, ipa);
        }
    }
    best.1
}

/// breath level for a key velocity (0..1); 1 at a velocity of 96/127, as upstream
fn breath_for(velocity: f64) -> f64 {
    let v = velocity * 127.0;
    0.62 + 0.4 * (v / 96.0).clamp(0.0, 1.0) + 0.12 * ((v - 96.0) / 31.0).max(0.0)
}

/// sung when the lyrics are empty
fn ah() -> Syllable {
    Syllable { text: "ah".into(), nucleus: Segment { ipa: "ɑ".into(), stress: Some(1), ..Default::default() }, word_end: true, ..Default::default() }
}

fn is_melisma(s: &Syllable) -> bool {
    s.melisma == Some(true)
}

/// gestures planned "now" start this long after they are scheduled (s), as upstream
const LEAD: f64 = 0.006;
/// a vowel-mode note's vibrato comes in after this long (s), over `VIBRATO_RAMP`
const VIBRATO_DELAY: f64 = 0.22;
const VIBRATO_RAMP: f64 = 0.3;

struct Sounding {
    key: u8,
    /// the pitch being sung (MIDI, with transpose and bend)
    pitch: f64,
    /// the syllable whose vowel is sounding (a melisma keeps its parent's, for the coda)
    syl: Syllable,
    tag: f64,
    /// its index in the lyrics, if it came from them
    index: Option<usize>,
}

pub struct Performer {
    engine: Engine,
    sample_rate: f64,
    mode: Mode,
    body: BodyMix,
    tone: Tone,
    /// semitones added to every key
    transpose: f64,
    /// pitch bend (semitones)
    bend: f64,
    bend_applied: f64,

    syllables: Vec<Syllable>,
    /// the syllable the next key sings
    cursor: usize,
    sounding: Option<Sounding>,
    /// keys down, oldest first
    keys: Vec<u8>,
    sustain: bool,
    /// keys released while the pedal was down
    sustained: Vec<u8>,
    /// gesture tags: a note uses tag (articulation, breath) and tag + 1 (pitch, vibrato)
    next_tag: f64,
    /// endings (glide, coda) that may still be playing
    tails: Vec<f64>,
    exhales_live: bool,

    /// vowel mode: the key sounding, and how long it has (s)
    vowel_key: Option<u8>,
    note_age: f64,
    pad: (f64, f64),
    pad_table: [(f64, f64, ArticPatch); PAD_VOWELS.len()],
    last_source: SourcePatch,
}

impl Performer {
    pub fn new(sample_rate: f64) -> Performer {
        Self::with_engine(Engine::new(sample_rate), BodyMix::MALE)
    }

    /// A performer on an engine built elsewhere (off the audio thread) for `body`.
    pub fn with_engine(engine: Engine, body: BodyMix) -> Performer {
        let sample_rate = engine.sample_rate;
        let mut p = Performer {
            engine,
            sample_rate,
            mode: Mode::Sing,
            body,
            tone: Tone::default(),
            transpose: 0.0,
            bend: 0.0,
            bend_applied: 0.0,
            syllables: Vec::new(),
            cursor: 0,
            sounding: None,
            keys: Vec::with_capacity(128),
            sustain: false,
            sustained: Vec::with_capacity(128),
            next_tag: 2.0,
            tails: Vec::with_capacity(8),
            exhales_live: false,
            vowel_key: None,
            note_age: 0.0,
            pad: (0.85, 1.0),
            pad_table: [(0.0, 0.0, ArticPatch::default()); PAD_VOWELS.len()],
            last_source: SourcePatch::default(),
        };
        p.build_pad_table();
        p.apply_pad();
        p.sync_source();
        p
    }

    /// An engine for `body`, ready to hand to [`Performer::swap_engine`]. Allocates: build it off
    /// the audio thread.
    ///
    /// `eco` simulates the airway at the output rate instead of oversampling it (about half the
    /// processor time, a slightly duller and less exact voice). It is only offered where the
    /// simulation still runs fast enough to be stable (44.1 kHz and up).
    pub fn build_engine(sample_rate: f64, body: BodyMix, eco: bool) -> Engine {
        let mut e = Engine::new(sample_rate);
        if body != BodyMix::MALE {
            e.set_body(body);
        }
        if eco && sample_rate >= 44000.0 && oversample_for(sample_rate) > 1 {
            let mut v = HumanVoice::new(sample_rate, body.body(), 1);
            v.set_source(&SourcePatch::f0(default_f0(body)));
            *e.voice_mut() = v;
        }
        e
    }

    /// Change the body by taking an engine built for it. Whatever was sounding stops. Returns the
    /// old engine, to be dropped off the audio thread.
    pub fn swap_engine(&mut self, engine: Engine, body: BodyMix) -> Engine {
        self.forget_notes();
        let old = std::mem::replace(&mut self.engine, engine);
        self.body = body;
        self.last_source = SourcePatch::default();
        self.build_pad_table();
        self.apply_pad();
        self.sync_source();
        old
    }

    /// Change the body in place (allocates; for offline use and tests).
    pub fn set_body(&mut self, body: BodyMix) {
        if body == self.body {
            return;
        }
        let e = Self::build_engine(self.sample_rate, body, false);
        self.swap_engine(e, body);
    }

    pub fn body(&self) -> BodyMix {
        self.body
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: Mode) {
        if mode != self.mode {
            self.all_notes_off();
            self.mode = mode;
            self.sync_source();
        }
    }

    pub fn set_tone(&mut self, tone: &Tone) {
        if *tone != self.tone {
            self.tone = *tone;
            self.sync_source();
        }
    }

    pub fn set_transpose(&mut self, semitones: f64) {
        self.transpose = semitones;
    }

    pub fn set_bend(&mut self, semitones: f64) {
        self.bend = semitones;
    }

    /// The pad's handle (x: 0 front … 1 back, y: 0 close … 1 open).
    pub fn set_pad(&mut self, x: f64, y: f64) {
        let p = (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
        if p != self.pad {
            self.pad = p;
            self.apply_pad();
        }
    }

    /// New lyrics. The cursor goes back to the start. Returns the old ones (drop them off the
    /// audio thread).
    pub fn set_lyrics(&mut self, syllables: Vec<Syllable>) -> Vec<Syllable> {
        self.cursor = 0;
        std::mem::replace(&mut self.syllables, syllables)
    }

    pub fn syllable_count(&self) -> usize {
        self.syllables.len()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn set_cursor(&mut self, i: usize) {
        self.cursor = if self.syllables.is_empty() { 0 } else { i.min(self.syllables.len() - 1) };
    }

    /// The index in the lyrics of the syllable being sung now.
    pub fn sounding_index(&self) -> Option<usize> {
        self.sounding.as_ref().and_then(|s| s.index)
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn telemetry(&mut self, out: &mut VoiceTelemetry) {
        self.engine.telemetry(out);
    }

    fn pitch_of(&self, key: u8) -> f64 {
        key as f64 + self.transpose + self.bend
    }

    // ------------------------------------------------------------------ notes

    /// `velocity` is 0..1.
    pub fn note_on(&mut self, key: u8, velocity: f64) {
        self.sustained.retain(|k| *k != key);
        self.keys.retain(|k| *k != key);
        self.keys.push(key);
        match self.mode {
            Mode::Sing => self.sing_note_on(key, velocity),
            Mode::Vowel => self.vowel_note_on(key, velocity),
        }
    }

    pub fn note_off(&mut self, key: u8) {
        if self.sustain {
            if self.keys.contains(&key) && !self.sustained.contains(&key) {
                self.sustained.push(key);
            }
            return;
        }
        self.key_up(key);
    }

    fn key_up(&mut self, key: u8) {
        self.keys.retain(|k| *k != key);
        match self.mode {
            Mode::Sing => self.sing_note_off(key),
            Mode::Vowel => self.vowel_note_off(key),
        }
    }

    /// The sustain pedal: keys released while it is down keep sounding until it comes up.
    pub fn set_sustain(&mut self, down: bool) {
        self.sustain = down;
        if !down {
            while let Some(key) = self.sustained.pop() {
                self.key_up(key);
            }
        }
    }

    /// Stop everything sung: notes, endings, and whatever the score still holds.
    pub fn all_notes_off(&mut self) {
        if let Some(s) = self.sounding.take() {
            self.engine.release(s.tag);
            self.engine.release(s.tag + 1.0);
        }
        for i in 0..self.tails.len() {
            let t = self.tails[i];
            self.engine.release(t);
            self.engine.release(t + 1.0);
        }
        self.engine.stop_speech();
        self.engine.set_gate(0.0);
        self.forget_notes();
    }

    fn forget_notes(&mut self) {
        self.sounding = None;
        self.tails.clear();
        self.exhales_live = false;
        self.keys.clear();
        self.sustained.clear();
        self.vowel_key = None;
    }

    // ------------------------------------------------------------------ sing mode

    fn sing_note_on(&mut self, key: u8, velocity: f64) {
        let n = self.syllables.len();
        if n == 0 {
            self.start_note(ah(), key, velocity, None);
            return;
        }
        let i = self.cursor % n;
        if is_melisma(&self.syllables[i]) {
            if self.sounding.is_some() {
                self.glide_to(key);
            } else {
                // a melisma with nothing held: sing its vowel again, without the onset
                let mut parent = self.parent_of(i);
                parent.onset.clear();
                self.start_note(parent, key, velocity, Some(i));
            }
        } else {
            let syl = self.syllables[i].clone();
            self.start_note(syl, key, velocity, Some(i));
        }
        self.cursor = (i + 1) % n;
    }

    /// The syllable a melisma at index i continues.
    fn parent_of(&self, i: usize) -> Syllable {
        self.syllables[..=i].iter().rev().find(|s| !is_melisma(s)).cloned().unwrap_or_else(ah)
    }

    fn start_note(&mut self, syl: Syllable, key: u8, velocity: f64, index: Option<usize>) {
        let tag = self.next_tag;
        self.next_tag += 2.0;
        // earlier codas may finish, but their breath-out must not open the folds on this onset
        if self.exhales_live {
            for i in 0..self.tails.len() {
                let t = self.tails[i];
                self.engine.release(t + 1.0);
            }
        }
        self.exhales_live = false;
        let legato_from = self.sounding.take().map(|from| {
            self.engine.release(from.tag);
            self.engine.release(from.tag + 1.0);
            LegatoFrom { syl: from.syl, midi: from.pitch }
        });
        let pitch = self.pitch_of(key);
        let mut gestures = plan_note_on(&syl, pitch, &NoteOnOptions { body: self.body, vibrato: self.tone.vibrato, tag, legato_from });
        let breath = breath_for(velocity);
        for g in &mut gestures {
            if let Some(b) = g.targets.get(ControlKey::BREATH) {
                g.targets.set(ControlKey::BREATH, b * breath);
            }
        }
        self.engine.schedule(&gestures, LEAD, false);
        self.bend_applied = self.bend;
        self.sounding = Some(Sounding { key, pitch, syl, tag, index });
    }

    /// The held vowel moves to a new pitch (melisma, or back to a key still down).
    fn glide_to(&mut self, key: u8) {
        let pitch = self.pitch_of(key);
        let vibrato = self.tone.vibrato;
        if let Some(s) = self.sounding.as_mut() {
            self.engine.release(s.tag + 1.0);
            self.engine.schedule(&plan_melisma(pitch, &MelismaOptions { vibrato, tag: s.tag }), LEAD, false);
            s.key = key;
            s.pitch = pitch;
        }
        self.bend_applied = self.bend;
    }

    fn sing_note_off(&mut self, key: u8) {
        match &self.sounding {
            Some(s) if s.key == key => {}
            _ => return,
        }
        if let Some(&last) = self.keys.last() {
            self.glide_to(last);
            return;
        }
        let Some(s) = self.sounding.take() else { return };
        self.engine.release(s.tag);
        self.engine.release(s.tag + 1.0);
        // a melisma comes next: this vowel carries on there, and the coda waits for the chain's end
        let n = self.syllables.len();
        if n > 0 && is_melisma(&self.syllables[self.cursor % n]) {
            return;
        }
        let mut gestures = plan_note_off(&s.syl, s.pitch, self.body);
        if gestures.is_empty() {
            return;
        }
        let tag = self.next_tag;
        self.next_tag += 2.0;
        for g in &mut gestures {
            g.tag = Some(if g.label_str() == EXHALE_LABEL { tag + 1.0 } else { tag });
        }
        self.engine.schedule(&gestures, LEAD, false);
        if self.tails.len() == 8 {
            self.tails.remove(0); // long finished by now
        }
        self.tails.push(tag);
        self.exhales_live = true;
    }

    // ------------------------------------------------------------------ vowel mode

    fn vowel_note_on(&mut self, key: u8, velocity: f64) {
        if self.vowel_key.is_none() {
            self.note_age = 0.0;
        }
        self.vowel_key = Some(key);
        self.engine.set_gate(breath_for(velocity));
        self.sync_source();
    }

    fn vowel_note_off(&mut self, key: u8) {
        if self.vowel_key != Some(key) {
            return;
        }
        match self.keys.last() {
            Some(&last) => self.vowel_key = Some(last),
            None => {
                self.vowel_key = None;
                self.engine.set_gate(0.0);
            }
        }
        self.sync_source();
    }

    fn build_pad_table(&mut self) {
        for (i, ipa) in PAD_VOWELS.iter().enumerate() {
            let (x, y) = pad_position(ipa);
            self.pad_table[i] = (x, y, vowel_artic(ipa, self.body).unwrap_or_default());
        }
    }

    /// The posture under the pad's handle: the calibrated vowels around it, blended by distance.
    fn apply_pad(&mut self) {
        let (x, y) = self.pad;
        let mut parts = [(0.0, ArticPatch::default()); PAD_VOWELS.len()];
        for (i, (vx, vy, patch)) in self.pad_table.iter().enumerate() {
            let d2 = (x - vx) * (x - vx) + (y - vy) * (y - vy);
            let w = 1.0 / (d2 + 2e-4);
            parts[i] = (w * w, *patch);
        }
        self.engine.set_artic(&ArticPatch::blend(&parts), None);
    }

    // ------------------------------------------------------------------ source

    /// Send the tone (and, in vowel mode, the pitch) to the voice when it has changed.
    fn sync_source(&mut self) {
        let t = &self.tone;
        let mut p = SourcePatch {
            pressure: Some(t.pressure),
            tension: Some(t.tension),
            brightness: Some(t.brightness),
            breathiness: Some(t.breathiness),
            roughness: Some(t.roughness),
            fry: Some(t.fry),
            vibrato_rate: Some(t.vibrato_rate),
            drift: Some(t.drift),
            voiced: Some(!t.whisper),
            glide: Some(t.glide),
            // sung notes carry their own vibrato in the score
            vibrato_depth: Some(0.0),
            f0: None,
        };
        if self.mode == Mode::Vowel {
            if let Some(key) = self.vowel_key {
                p.f0 = Some(midi_to_hz(self.pitch_of(key)));
                // a note from silence starts on its pitch instead of sliding up to it
                if self.note_age < 0.03 {
                    p.glide = Some(t.glide.min(0.004));
                }
                let fade = ((self.note_age - VIBRATO_DELAY) / VIBRATO_RAMP).clamp(0.0, 1.0);
                p.vibrato_depth = Some(t.vibrato * fade);
            }
        }
        if p != self.last_source {
            self.engine.set_source(&p);
            self.last_source = p;
        }
    }

    // ------------------------------------------------------------------ audio

    /// Render `out.len()` samples (mono).
    pub fn render(&mut self, out: &mut [f32]) {
        match self.mode {
            Mode::Sing => {
                // a bend moves the held vowel like a melisma does
                if self.sounding.is_some() && (self.bend - self.bend_applied).abs() > 0.01 {
                    let key = self.sounding.as_ref().map(|s| s.key).unwrap_or(60);
                    self.glide_to(key);
                }
            }
            Mode::Vowel => self.sync_source(),
        }
        self.engine.render(out);
        self.note_age += out.len() as f64 / self.sample_rate;
    }
}
