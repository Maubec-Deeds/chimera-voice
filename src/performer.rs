//! The performer: turns notes into the engine's commands. No plugin or GUI code in here, so it
//! can be rendered and tested headlessly.
//!
//! Sing mode is a port of the web app's live Sing controller (`src/ui/singing.ts` upstream): each
//! key sings the next syllable of the lyrics at its pitch; overlapping keys sing legato; a lone
//! `-` in the lyrics holds the vowel onto the next pitch (melisma); releasing the sounding key
//! while another is still down glides back to that key. The lyrics are one of eight phrases; the
//! keys C0 to G0 (MIDI 24 to 31) pick the phrase.
//!
//! Vowel mode plays the voice like a monosynth: the keys set pitch and breath, the pad sets the
//! mouth's posture between the calibrated vowels.
//!
//! Creature mode plays one of the bestiary's animals: a held key breathes and sets its pitch, or
//! each key triggers one of its calls at that pitch.
//!
//! Beatbox mode maps the keys C1 to B1 to the mouth's drum sounds; every other key hums a bass
//! note.

use chimera::anatomy::human::{ArticKey, ArticPatch, BodyMix, NEUTRAL};
use chimera::control::score::{ControlKey, Gesture, Label};
use chimera::creatures::types::{CreatureDef, ParamDef, ParamRole};
use chimera::creatures::voice::CreatureTelemetry;
use chimera::creatures::CREATURES;
use chimera::engine::{default_f0, Engine};
use chimera::math::midi_to_hz;
use chimera::phonetics::beatbox::{beat_ready, beat_sound, BeatBuild, BeatStep, BEAT_SOUNDS, MAX_SOUND_GESTURES};
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
    /// keys play an animal
    Creature,
    /// keys play the mouth's drum sounds
    Beatbox,
}

/// What a key does in creature mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    /// the creature breathes while a key is down, at the key's pitch
    Hold,
    /// each key plays this call (an index into the creature's calls), at the key's pitch
    Call(usize),
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

// ---------------------------------------------------------------------- the face's mouth

/// The mouth a face tracker sees, laid over the vowel pad's posture in vowel mode. Everything
/// is 0..1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceMouth {
    /// how far the jaw has dropped
    pub jaw: f64,
    /// the gap between the lips
    pub lips: f64,
    /// how far the lips are pushed forward
    pub pucker: f64,
    /// how wide the lips are spread
    pub spread: f64,
    /// the lips are shut (a "b"; an "m" once the nose opens)
    pub closed: f64,
    /// the lower lip is on the upper teeth ("v", or "f" when whispering)
    pub teeth: f64,
    /// the soft palate is open: sound goes through the nose
    pub nasal: f64,
}

/// Lip aperture with the lips shut, and with the lower lip against the teeth (as the engine's
/// own "b" and "v" have them).
const LIPS_SHUT: f64 = -0.25;
const LIPS_ON_TEETH: f64 = 0.07;
/// The soft palate when it lets the sound through the nose (as the engine's "m" has it).
const VELUM_OPEN: f64 = 0.9;

// ---------------------------------------------------------------------- the vowel pad

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

// ---------------------------------------------------------------------- phrases

/// How many phrases the lyrics hold.
pub const PHRASES: usize = 8;
/// The key that picks the first phrase (C0 in Bitwig's naming); the next seven keys pick the rest.
pub const PHRASE_KEY: u8 = 24;

// ---------------------------------------------------------------------- creatures

/// How many knobs a creature has at most (its parameters other than the pitch the keys set).
pub const CREATURE_KNOBS: usize = 8;

pub fn creature(index: usize) -> &'static CreatureDef {
    &CREATURES[index.min(CREATURES.len() - 1)]
}

/// The creature's knobs: its parameters other than the one the keys set, with their indices.
pub fn creature_knobs(def: &'static CreatureDef) -> impl Iterator<Item = (usize, &'static ParamDef)> {
    def.params.iter().enumerate().filter(|(_, p)| p.role != Some(ParamRole::Pitch)).take(CREATURE_KNOBS)
}

/// A knob position (0..1) as the parameter's value.
pub fn knob_value(p: &ParamDef, u: f64) -> f64 {
    let u = u.clamp(0.0, 1.0);
    if p.log == Some(true) {
        p.min * (p.max / p.min).powf(u)
    } else {
        p.min + (p.max - p.min) * u
    }
}

/// A parameter's value as a knob position (0..1).
pub fn knob_position(p: &ParamDef, v: f64) -> f64 {
    let u = if p.log == Some(true) { (v / p.min).ln() / (p.max / p.min).ln() } else { (v - p.min) / (p.max - p.min) };
    u.clamp(0.0, 1.0)
}

/// The knob positions a creature starts from.
pub fn creature_defaults(def: &'static CreatureDef) -> [f64; CREATURE_KNOBS] {
    let mut out = [0.5; CREATURE_KNOBS];
    for (k, (_, p)) in creature_knobs(def).enumerate() {
        out[k] = knob_position(p, p.default);
    }
    out
}

/// The pitch a key plays on a creature: the keyboard is moved by whole octaves so middle C sits
/// in the middle of the creature's range, and notes outside the range fold back into it (upstream's
/// `creatureKeyHz`).
fn creature_key_hz(midi: f64, p: &ParamDef) -> f64 {
    let shift = ((p.min * p.max).sqrt() / midi_to_hz(60.0)).log2().round();
    let mut f = midi_to_hz(midi) * 2f64.powf(shift);
    while f > p.max {
        f /= 2.0;
    }
    while f < p.min {
        f *= 2.0;
    }
    f.min(p.max)
}

// ---------------------------------------------------------------------- beatbox

/// The key of the first drum sound (C1 in Bitwig's naming, where drum clips start).
pub const BEAT_KEY: u8 = 36;
/// The hum's index in the engine's sounds.
pub const HUM: usize = 9;

/// The drum sound a key plays (an index into the engine's `BEAT_SOUNDS`), laid out like a General
/// MIDI kit; `None` for the keys that hum.
pub fn beat_for_key(key: u8) -> Option<usize> {
    Some(match key {
        36 => 0, // kick
        37 => 4, // rimshot
        38 => 3, // snare
        39 => 7, // lip pop
        40 => 5, // inward snare
        41 => 6, // tongue click
        42 => 1, // hi-hat
        43 => 6,
        44 => 1,
        45 => 7,
        46 => 2, // open hat
        47 => 8, // crash
        _ => return None,
    })
}

/// The first key that plays a sound.
pub fn key_for_beat(sound: usize) -> Option<u8> {
    (36..48).find(|k| beat_for_key(*k) == Some(sound))
}

// ----------------------------------------------------------------------

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

/// An engine and what goes with it, built off the audio thread.
pub struct Built {
    pub engine: Engine,
    pub body: BodyMix,
    /// sized for the engine's creature, so filling it never allocates
    creature_tele: CreatureTelemetry,
}

impl Built {
    /// `eco` simulates the human airway at the output rate instead of oversampling it (about half
    /// the processor time, a slightly duller and less exact voice). It is only used where the
    /// simulation still runs fast enough to be stable (44.1 kHz and up). `creature` (an index
    /// into the bestiary) plays that animal instead of the human voice.
    pub fn new(sample_rate: f64, body: BodyMix, eco: bool, creature: Option<usize>) -> Built {
        let mut e = Engine::new(sample_rate);
        if body != BodyMix::MALE {
            e.set_body(body);
        }
        if eco && sample_rate >= 44000.0 && oversample_for(sample_rate) > 1 {
            let mut v = HumanVoice::new(sample_rate, body.body(), 1);
            v.set_source(&SourcePatch::f0(default_f0(body)));
            *e.voice_mut() = v;
        }
        if let Some(i) = creature {
            e.set_creature(Some(self::creature(i)), &[]);
        }
        let creature_tele = e.creature().map(|c| c.new_telemetry()).unwrap_or_default();
        Built { engine: e, body, creature_tele }
    }
}

pub struct Performer {
    engine: Engine,
    creature_tele: CreatureTelemetry,
    sample_rate: f64,
    mode: Mode,
    body: BodyMix,
    tone: Tone,
    /// semitones added to every key
    transpose: f64,
    /// pitch bend (semitones)
    bend: f64,
    bend_applied: f64,

    phrases: Vec<Vec<Syllable>>,
    phrase: usize,
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

    /// vowel and creature modes: the key sounding, and how long it has (s)
    held_key: Option<u8>,
    note_age: f64,
    pad: (f64, f64),
    pad_table: [(f64, f64, ArticPatch); PAD_VOWELS.len()],
    /// the least and most the pad's vowels use of the jaw, lip aperture, protrusion and width
    lip_range: [(f64, f64); 4],
    /// the mouth a face tracker sees, while it drives the lips (vowel mode)
    face_mouth: Option<FaceMouth>,
    /// the lips have been driven by a face, so the keys it touched are set back when it stops
    face_touched: bool,
    /// how hard a held note breathes: the velocity's breath, and what the face scales it by
    gate_base: f64,
    effort: f64,
    last_source: SourcePatch,

    key_action: KeyAction,
    knobs: [f64; CREATURE_KNOBS],
    knobs_sent: bool,

    /// the hum sounding: its key and tag
    hum: Option<(u8, f64)>,
    scratch: Vec<Gesture>,
    /// the last drum sound played, and how many have been (for the editor's pads)
    last_beat: usize,
    beat_count: u32,
    /// samples still to mute: a new engine thumps faintly while its mouth settles
    settling: usize,
}

impl Performer {
    pub fn new(sample_rate: f64) -> Performer {
        Self::with(Built::new(sample_rate, BodyMix::MALE, false, None))
    }

    /// A performer on an engine built elsewhere (off the audio thread).
    pub fn with(built: Built) -> Performer {
        let sample_rate = built.engine.sample_rate;
        let mut p = Performer {
            engine: built.engine,
            creature_tele: built.creature_tele,
            sample_rate,
            mode: Mode::Sing,
            body: built.body,
            tone: Tone::default(),
            transpose: 0.0,
            bend: 0.0,
            bend_applied: 0.0,
            phrases: (0..PHRASES).map(|_| Vec::new()).collect(),
            phrase: 0,
            cursor: 0,
            sounding: None,
            keys: Vec::with_capacity(128),
            sustain: false,
            sustained: Vec::with_capacity(128),
            next_tag: 2.0,
            tails: Vec::with_capacity(8),
            exhales_live: false,
            held_key: None,
            note_age: 0.0,
            pad: (0.85, 1.0),
            pad_table: [(0.0, 0.0, ArticPatch::default()); PAD_VOWELS.len()],
            lip_range: [(0.0, 1.0); 4],
            face_mouth: None,
            face_touched: false,
            gate_base: 0.0,
            effort: 1.0,
            last_source: SourcePatch::default(),
            key_action: KeyAction::Hold,
            knobs: [0.5; CREATURE_KNOBS],
            knobs_sent: false,
            hum: None,
            scratch: Vec::with_capacity(MAX_SOUND_GESTURES),
            last_beat: 0,
            beat_count: 0,
            settling: 0,
        };
        p.settle();
        p.build_pad_table();
        p.apply_posture();
        p.sync_source();
        p
    }

    /// Take an engine built for another body, quality or creature. Whatever was sounding stops.
    /// Returns the old one, to be dropped off the audio thread.
    pub fn swap(&mut self, built: Built) -> Built {
        self.forget_notes();
        let old = Built { engine: std::mem::replace(&mut self.engine, built.engine), body: self.body, creature_tele: std::mem::replace(&mut self.creature_tele, built.creature_tele) };
        self.body = built.body;
        self.last_source = SourcePatch::default();
        self.knobs_sent = false;
        self.build_pad_table();
        self.apply_posture();
        self.sync_source();
        self.sync_knobs();
        self.settle();
        old
    }

    /// Mute the engine's first moments.
    fn settle(&mut self) {
        self.settling = (0.2 * self.sample_rate) as usize;
    }

    /// Change the body in place (allocates; for offline use and tests).
    pub fn set_body(&mut self, body: BodyMix) {
        if body != self.body {
            self.swap(Built::new(self.sample_rate, body, false, None));
        }
    }

    /// Play a creature, or the human voice again (allocates; for offline use and tests).
    pub fn set_creature(&mut self, creature: Option<usize>) {
        self.swap(Built::new(self.sample_rate, self.body, false, creature));
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
            self.apply_posture();
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
            self.apply_posture();
        }
    }

    /// The mouth a face tracker sees drives the jaw and lips directly in vowel mode (the pad then
    /// only places the tongue); `None` hands them back to the pad.
    pub fn set_face_mouth(&mut self, mouth: Option<FaceMouth>) {
        if mouth != self.face_mouth {
            self.face_mouth = mouth;
            if self.mode == Mode::Vowel {
                self.apply_posture();
            }
        }
    }

    /// Scale how hard a held vowel or creature note breathes (1: as its velocity set it).
    pub fn set_effort(&mut self, effort: f64) {
        let effort = effort.clamp(0.3, 1.3);
        if (effort - self.effort).abs() > 0.004 {
            self.effort = effort;
            if self.held_key.is_some() && matches!(self.mode, Mode::Vowel | Mode::Creature) {
                self.engine.set_gate(self.gate_base * effort);
            }
        }
    }

    // ------------------------------------------------------------------ lyrics

    /// New lyrics for every phrase (missing ones are empty). The cursor goes back to the start.
    /// Returns the old ones (drop them off the audio thread).
    pub fn set_phrases(&mut self, mut phrases: Vec<Vec<Syllable>>) -> Vec<Vec<Syllable>> {
        phrases.resize_with(PHRASES, Vec::new);
        self.cursor = 0;
        std::mem::replace(&mut self.phrases, phrases)
    }

    /// One phrase's lyrics, as the first phrase (for tests and simple hosts).
    pub fn set_lyrics(&mut self, syllables: Vec<Syllable>) {
        self.set_phrases(vec![syllables]);
        self.phrase = 0;
    }

    /// Sing another phrase, from its first syllable.
    pub fn select_phrase(&mut self, i: usize) {
        self.phrase = i.min(PHRASES - 1);
        self.cursor = 0;
    }

    pub fn phrase(&self) -> usize {
        self.phrase
    }

    fn syllables(&self) -> &[Syllable] {
        &self.phrases[self.phrase]
    }

    pub fn syllable_count(&self) -> usize {
        self.syllables().len()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn set_cursor(&mut self, i: usize) {
        let n = self.syllables().len();
        self.cursor = if n == 0 { 0 } else { i.min(n - 1) };
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

    /// The creature's telemetry, when one is playing.
    pub fn creature_telemetry(&mut self) -> Option<&CreatureTelemetry> {
        if self.engine.creature_telemetry(&mut self.creature_tele) {
            Some(&self.creature_tele)
        } else {
            None
        }
    }

    /// The last drum sound played and how many have been.
    pub fn last_beat(&self) -> (usize, u32) {
        (self.last_beat, self.beat_count)
    }

    fn pitch_of(&self, key: u8) -> f64 {
        key as f64 + self.transpose + self.bend
    }

    /// The engine is playing what the mode needs (a creature in creature mode, the human
    /// otherwise). It is not for a moment after the mode changes, while the other is built.
    fn ready(&self) -> bool {
        self.engine.creature().is_some() == (self.mode == Mode::Creature)
    }

    // ------------------------------------------------------------------ notes

    /// `velocity` is 0..1.
    pub fn note_on(&mut self, key: u8, velocity: f64) {
        if self.mode == Mode::Sing && (PHRASE_KEY..PHRASE_KEY + PHRASES as u8).contains(&key) {
            self.select_phrase((key - PHRASE_KEY) as usize);
            return;
        }
        if !self.ready() {
            return;
        }
        self.sustained.retain(|k| *k != key);
        self.keys.retain(|k| *k != key);
        self.keys.push(key);
        match self.mode {
            Mode::Sing => self.sing_note_on(key, velocity),
            Mode::Vowel => self.vowel_note_on(key, velocity),
            Mode::Creature => self.creature_note_on(key, velocity),
            Mode::Beatbox => self.beat_note_on(key, velocity),
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
        if !self.keys.contains(&key) {
            return;
        }
        self.keys.retain(|k| *k != key);
        match self.mode {
            Mode::Sing => self.sing_note_off(key),
            Mode::Vowel => self.vowel_note_off(key),
            Mode::Creature => self.creature_note_off(key),
            Mode::Beatbox => self.beat_note_off(key),
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

    /// Stop everything: notes, endings, calls, and whatever the score still holds.
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
        self.engine.stop_call();
        self.engine.set_gate(0.0);
        self.forget_notes();
    }

    fn forget_notes(&mut self) {
        self.sounding = None;
        self.tails.clear();
        self.exhales_live = false;
        self.keys.clear();
        self.sustained.clear();
        self.held_key = None;
        self.hum = None;
    }

    // ------------------------------------------------------------------ sing mode

    fn sing_note_on(&mut self, key: u8, velocity: f64) {
        let n = self.syllables().len();
        if n == 0 {
            self.start_note(ah(), key, velocity, None);
            return;
        }
        let i = self.cursor % n;
        if is_melisma(&self.syllables()[i]) {
            if self.sounding.is_some() {
                self.glide_to(key);
            } else {
                // a melisma with nothing held: sing its vowel again, without the onset
                let mut parent = self.parent_of(i);
                parent.onset.clear();
                self.start_note(parent, key, velocity, Some(i));
            }
        } else {
            let syl = self.syllables()[i].clone();
            self.start_note(syl, key, velocity, Some(i));
        }
        self.cursor = (i + 1) % n;
    }

    /// The syllable a melisma at index i continues.
    fn parent_of(&self, i: usize) -> Syllable {
        self.syllables()[..=i].iter().rev().find(|s| !is_melisma(s)).cloned().unwrap_or_else(ah)
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
        let n = self.syllables().len();
        if n > 0 && is_melisma(&self.syllables()[self.cursor % n]) {
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
        if self.held_key.is_none() {
            self.note_age = 0.0;
        }
        self.held_key = Some(key);
        self.gate_base = breath_for(velocity);
        self.engine.set_gate(self.gate_base * self.effort);
        self.sync_source();
    }

    fn vowel_note_off(&mut self, key: u8) {
        if self.held_key != Some(key) {
            return;
        }
        match self.keys.last() {
            Some(&last) => self.held_key = Some(last),
            None => {
                self.held_key = None;
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
        for (r, key) in self.lip_range.iter_mut().zip([ArticKey::Jaw, ArticKey::LipAp, ArticKey::LipProt, ArticKey::LipWidth]) {
            let values = self.pad_table.iter().filter_map(|(_, _, patch)| patch.get(key));
            let (lo, hi) = values.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)));
            *r = if lo <= hi { (lo, hi) } else { (0.0, 1.0) };
        }
    }

    /// The mouth's resting posture: the vowel under the pad's handle (the calibrated vowels around
    /// it, blended by distance), or a beatboxer's ready posture.
    fn apply_posture(&mut self) {
        if self.mode == Mode::Beatbox {
            self.engine.set_artic(&beat_ready(self.body), None);
            return;
        }
        let (x, y) = self.pad;
        let mut parts = [(0.0, ArticPatch::default()); PAD_VOWELS.len()];
        for (i, (vx, vy, patch)) in self.pad_table.iter().enumerate() {
            let d2 = (x - vx) * (x - vx) + (y - vy) * (y - vy);
            let w = 1.0 / (d2 + 2e-4);
            parts[i] = (w * w, *patch);
        }
        let mut posture = ArticPatch::blend(&parts);
        let lerp = |r: (f64, f64), u: f64| r.0 + (r.1 - r.0) * u.clamp(0.0, 1.0);
        match self.face_mouth {
            Some(m) if self.mode == Mode::Vowel => {
                // the face's own jaw and lips, across the range the vowels use
                let [jaw, aperture, protrusion, width] = self.lip_range;
                let mut ap = lerp((aperture.0.min(0.15), aperture.1), m.lips);
                ap += (LIPS_ON_TEETH - ap) * m.teeth.clamp(0.0, 1.0);
                ap += (LIPS_SHUT - ap) * m.closed.clamp(0.0, 1.0);
                posture.set(ArticKey::Jaw, lerp(jaw, m.jaw));
                posture.set(ArticKey::LipAp, ap);
                posture.set(ArticKey::LipProt, lerp(protrusion, m.pucker));
                posture.set(ArticKey::LipWidth, lerp(width, m.spread));
                posture.set(ArticKey::Labiodental, m.teeth.clamp(0.0, 1.0) * (1.0 - m.closed.clamp(0.0, 1.0)));
                posture.set(ArticKey::Velum, lerp((NEUTRAL.velum, VELUM_OPEN), m.nasal));
                self.face_touched = true;
            }
            _ if self.face_touched => {
                // the face let go: the teeth and the nose go back to rest
                posture.set(ArticKey::Labiodental, NEUTRAL.labiodental);
                posture.set(ArticKey::Velum, NEUTRAL.velum);
                self.face_touched = false;
            }
            _ => {}
        }
        self.engine.set_artic(&posture, None);
    }

    // ------------------------------------------------------------------ creature mode

    pub fn set_key_action(&mut self, action: KeyAction) {
        if action != self.key_action {
            if self.mode == Mode::Creature {
                self.all_notes_off();
            }
            self.key_action = action;
        }
    }

    /// The creature's knobs (0..1 each, in the order of [`creature_knobs`]).
    pub fn set_knobs(&mut self, knobs: &[f64; CREATURE_KNOBS]) {
        if *knobs != self.knobs || !self.knobs_sent {
            self.knobs = *knobs;
            self.sync_knobs();
        }
    }

    fn sync_knobs(&mut self) {
        let Some(def) = self.engine.creature().map(|c| c.def) else { return };
        let mut values = [(0usize, 0.0f64); CREATURE_KNOBS];
        let mut n = 0;
        for (k, (i, p)) in creature_knobs(def).enumerate() {
            values[n] = (i, knob_value(p, self.knobs[k]));
            n += 1;
        }
        self.engine.set_creature_params(&values[..n]);
        self.knobs_sent = true;
    }

    /// The creature's pitch parameter and the pitch this key plays on it.
    fn creature_pitch(&self, key: u8) -> Option<(usize, f64)> {
        let def = self.engine.creature()?.def;
        let i = def.params.iter().position(|p| p.role == Some(ParamRole::Pitch))?;
        Some((i, creature_key_hz(self.pitch_of(key), &def.params[i])))
    }

    fn creature_note_on(&mut self, key: u8, velocity: f64) {
        let pitch = self.creature_pitch(key);
        let calls = self.engine.creature().map_or(0, |c| c.def.calls.len());
        match self.key_action {
            KeyAction::Call(i) if i < calls => {
                self.engine.play_call(i, pitch.map(|p| p.1), Some(0.5 + 0.6 * velocity));
            }
            _ => {
                if let Some(p) = pitch {
                    self.engine.set_creature_params(&[p]);
                }
                self.held_key = Some(key);
                self.bend_applied = self.bend;
                self.gate_base = breath_for(velocity);
                self.engine.set_gate(self.gate_base * self.effort);
            }
        }
    }

    fn creature_note_off(&mut self, key: u8) {
        if self.held_key != Some(key) {
            return;
        }
        match self.keys.last().copied() {
            Some(last) => {
                self.held_key = Some(last);
                if let Some(p) = self.creature_pitch(last) {
                    self.engine.set_creature_params(&[p]);
                }
            }
            None => {
                self.held_key = None;
                self.engine.set_gate(0.0);
            }
        }
    }

    // ------------------------------------------------------------------ beatbox mode

    fn beat_note_on(&mut self, key: u8, velocity: f64) {
        match beat_for_key(key) {
            Some(sound) => self.beat(sound, velocity),
            None => self.start_hum(key, velocity),
        }
    }

    /// Play a drum sound (an index into the engine's `BEAT_SOUNDS`) as soon as the mouth can make
    /// it. The hum (the last sound) plays a short note here; the keys hold it.
    pub fn beat(&mut self, sound: usize, velocity: f64) {
        if self.mode != Mode::Beatbox || !self.ready() || sound >= BEAT_SOUNDS.len() {
            return;
        }
        let accent = 0.55 + 0.75 * velocity.clamp(0.0, 1.0);
        let note = if sound == HUM { Some(40.0 + self.transpose) } else { None };
        self.engine.beat_hit(&BeatStep { id: Label::new(BEAT_SOUNDS[sound].id), note, accent: Some(accent) });
        self.last_beat = sound;
        self.beat_count = self.beat_count.wrapping_add(1);
    }

    /// A hummed bass note, held until its key comes up.
    fn start_hum(&mut self, key: u8, velocity: f64) {
        let Some(sound) = beat_sound(BEAT_SOUNDS[HUM].id) else { return };
        if let Some((_, tag)) = self.hum.take() {
            self.engine.release(tag);
        }
        let accent = 0.55 + 0.75 * velocity.clamp(0.0, 1.0);
        self.scratch.clear();
        (sound.build)(self.body, &BeatBuild { f0: midi_to_hz(self.pitch_of(key)), len: 30.0, accent }, &mut self.scratch);
        let tag = self.next_tag;
        self.next_tag += 2.0;
        // the hit is at the gestures' time 0: start early enough for the lips to shut first
        let mut lead: f64 = 0.0;
        for g in &mut self.scratch {
            g.tag = Some(tag);
            lead = lead.max(-(g.t0 - g.ramp_in));
        }
        self.engine.schedule(&self.scratch, lead, false);
        self.hum = Some((key, tag));
        self.last_beat = HUM;
        self.beat_count = self.beat_count.wrapping_add(1);
    }

    fn beat_note_off(&mut self, key: u8) {
        match self.hum {
            Some((k, tag)) if k == key => {
                self.engine.release(tag);
                self.hum = None;
                // back to a hum key still held
                if let Some(last) = self.keys.iter().rev().copied().find(|k| beat_for_key(*k).is_none()) {
                    self.start_hum(last, 0.7);
                }
            }
            _ => {}
        }
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
            if let Some(key) = self.held_key {
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
            Mode::Creature => {
                if let Some(key) = self.held_key {
                    if (self.bend - self.bend_applied).abs() > 0.001 {
                        if let Some(p) = self.creature_pitch(key) {
                            self.engine.set_creature_params(&[p]);
                        }
                        self.bend_applied = self.bend;
                    }
                }
            }
            Mode::Beatbox => {}
        }
        self.engine.render(out);
        if self.settling > 0 {
            let n = self.settling.min(out.len());
            out[..n].fill(0.0);
            self.settling -= n;
        }
        self.note_age += out.len() as f64 / self.sample_rate;
    }
}
