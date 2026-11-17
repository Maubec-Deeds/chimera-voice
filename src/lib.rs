//! Chimera Voice: the CHIMERA physical-modelling voice engine (https://github.com/jamesondh/chimera-vocal-synth,
//! MIT, Â© Jameson Hodge) as a monophonic CLAP instrument.

use chimera::anatomy::human::BodyMix;
use chimera::voice::human_voice::VoiceTelemetry;
use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use parking_lot::RwLock;
use std::sync::atomic::Ordering;
use std::sync::Arc;

mod editor;
pub mod face;
pub mod lyrics;
pub mod performer;
pub mod routes;
pub mod shared;

use performer::{Built, KeyAction, Mode, Performer, Tone, CREATURE_KNOBS, PHRASES};
use shared::{Shared, VoiceSpec, SCOPE_LEN, TUBES, TUBE_POINTS};

pub const DEFAULT_LYRICS: &str = "Hello there, I am a voice made of air.";

#[derive(Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeParam {
    #[id = "sing"]
    #[name = "Sing lyrics"]
    Sing,
    #[id = "vowel"]
    #[name = "Vowel pad"]
    Vowel,
    #[id = "creature"]
    #[name = "Creature"]
    Creature,
    #[id = "beatbox"]
    #[name = "Beatbox"]
    Beatbox,
}

#[derive(Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreatureParam {
    #[id = "songbird"]
    Songbird,
    #[id = "seal"]
    Seal,
    #[id = "alligator"]
    Alligator,
    #[id = "elephant"]
    Elephant,
}

impl CreatureParam {
    pub const ALL: [CreatureParam; 4] = [CreatureParam::Songbird, CreatureParam::Seal, CreatureParam::Alligator, CreatureParam::Elephant];
    /// Its index in the engine's bestiary.
    pub fn index(self) -> usize {
        self as usize
    }
}

#[derive(Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeysParam {
    #[id = "hold"]
    #[name = "Held note"]
    Hold,
    #[id = "call1"]
    #[name = "Call 1"]
    Call1,
    #[id = "call2"]
    #[name = "Call 2"]
    Call2,
    #[id = "call3"]
    #[name = "Call 3"]
    Call3,
    #[id = "call4"]
    #[name = "Call 4"]
    Call4,
}

impl KeysParam {
    pub const CALLS: [KeysParam; 4] = [KeysParam::Call1, KeysParam::Call2, KeysParam::Call3, KeysParam::Call4];
    fn action(self) -> KeyAction {
        match self {
            KeysParam::Hold => KeyAction::Hold,
            KeysParam::Call1 => KeyAction::Call(0),
            KeysParam::Call2 => KeyAction::Call(1),
            KeysParam::Call3 => KeyAction::Call(2),
            KeysParam::Call4 => KeyAction::Call(3),
        }
    }
}

/// The body slider: 0 adult male, 1 adult female, 2 child, blending in between.
pub fn body_for(v: f32) -> BodyMix {
    let v = ((v as f64) * 50.0).round() / 50.0;
    if v <= 1.0 {
        BodyMix::new(v, 0.0)
    } else {
        BodyMix::new(1.0, v - 1.0)
    }
}

fn body_name(v: f32) -> String {
    let pct = |x: f32| (x * 100.0).round() as i32;
    if v <= 0.01 {
        "Man".into()
    } else if (v - 1.0).abs() <= 0.01 {
        "Woman".into()
    } else if v >= 1.99 {
        "Child".into()
    } else if v < 1.0 {
        format!("Man {} / Woman {}", 100 - pct(v), pct(v))
    } else {
        format!("Woman {} / Child {}", 100 - pct(v - 1.0), pct(v - 1.0))
    }
}

#[derive(Params)]
pub struct ChimeraParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,
    /// the eight phrases of lyrics
    #[persist = "phrases"]
    pub phrases: Arc<RwLock<Vec<String>>>,
    /// which part of the face changes which setting, and by how much (see `routes`)
    #[persist = "face-routes"]
    pub face_routes: Arc<RwLock<routes::Routes>>,

    #[id = "mode"]
    pub mode: EnumParam<ModeParam>,
    #[id = "body"]
    pub body: FloatParam,
    #[id = "octave"]
    pub octave: IntParam,
    #[id = "phrase"]
    pub phrase: IntParam,

    #[id = "breath"]
    pub breath: FloatParam,
    #[id = "pressed"]
    pub pressed: FloatParam,
    #[id = "bright"]
    pub bright: FloatParam,
    #[id = "breathy"]
    pub breathy: FloatParam,
    #[id = "rough"]
    pub rough: FloatParam,
    #[id = "fry"]
    pub fry: FloatParam,
    #[id = "whisper"]
    pub whisper: BoolParam,

    #[id = "vibrato"]
    pub vibrato: FloatParam,
    #[id = "vibrate"]
    pub vibrato_rate: FloatParam,
    #[id = "glide"]
    pub glide: FloatParam,
    #[id = "drift"]
    pub drift: FloatParam,
    #[id = "bendrange"]
    pub bend_range: IntParam,

    #[id = "vowelx"]
    pub vowel_x: FloatParam,
    #[id = "vowely"]
    pub vowel_y: FloatParam,

    #[id = "creature"]
    pub creature: EnumParam<CreatureParam>,
    #[id = "keys"]
    pub keys: EnumParam<KeysParam>,
    /// the creature's knobs: what each one does depends on the creature
    #[nested(array, group = "Creature")]
    pub knobs: [KnobParam; CREATURE_KNOBS],

    #[id = "eco"]
    pub eco: BoolParam,
    #[id = "restart"]
    pub restart_on_play: BoolParam,
    /// follow a webcam face tracker (tools/facetrack)
    #[id = "face"]
    pub face: BoolParam,
    #[id = "gain"]
    pub gain: FloatParam,
}

#[derive(Params)]
pub struct KnobParam {
    #[id = "knob"]
    pub value: FloatParam,
}

fn percent(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_unit(" %")
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

impl Default for ChimeraParams {
    fn default() -> Self {
        let mut phrases = vec![String::new(); PHRASES];
        phrases[0] = DEFAULT_LYRICS.to_string();
        let knob_defaults = performer::creature_defaults(performer::creature(0));
        Self {
            editor_state: {
                // CHIMERA_UI_SCALE opens the window at another size (for checking the layout)
                let scale = std::env::var("CHIMERA_UI_SCALE").ok().and_then(|v| v.parse::<f32>().ok()).map_or(1.0, |v| v.clamp(0.6, 2.5));
                EguiState::from_size((editor::WIDTH as f32 * scale).round() as u32, (editor::HEIGHT as f32 * scale).round() as u32)
            },
            phrases: Arc::new(RwLock::new(phrases)),
            face_routes: Arc::new(RwLock::new(routes::defaults())),

            mode: EnumParam::new("Mode", ModeParam::Sing),
            body: FloatParam::new("Body", 0.0, FloatRange::Linear { min: 0.0, max: 2.0 })
                .with_value_to_string(Arc::new(body_name))
                .with_string_to_value(Arc::new(|s| {
                    let s = s.trim().to_lowercase();
                    match s.as_str() {
                        "man" | "male" => Some(0.0),
                        "woman" | "female" => Some(1.0),
                        "child" => Some(2.0),
                        _ => s.parse().ok(),
                    }
                })),
            octave: IntParam::new("Octave", 0, IntRange::Linear { min: -3, max: 3 }),
            phrase: IntParam::new("Phrase", 1, IntRange::Linear { min: 1, max: PHRASES as i32 }),

            breath: FloatParam::new("Breath", 8.0, FloatRange::Linear { min: 2.0, max: 22.0 })
                .with_unit(" cmH2O")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
            pressed: percent("Pressed", 0.5),
            bright: percent("Bright", 0.25),
            breathy: percent("Breathy", 0.15),
            rough: percent("Rough", 0.1),
            fry: percent("Fry", 0.0),
            whisper: BoolParam::new("Whisper", false),

            vibrato: FloatParam::new("Vibrato", 35.0, FloatRange::Linear { min: 0.0, max: 120.0 })
                .with_unit(" cents")
                .with_value_to_string(formatters::v2s_f32_rounded(0)),
            vibrato_rate: FloatParam::new("Vibrato Rate", 6.0, FloatRange::Linear { min: 3.0, max: 9.0 })
                .with_unit(" Hz")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
            glide: FloatParam::new("Glide", 60.0, FloatRange::Skewed { min: 5.0, max: 600.0, factor: FloatRange::skew_factor(-1.5) })
                .with_unit(" ms")
                .with_value_to_string(formatters::v2s_f32_rounded(0)),
            drift: FloatParam::new("Drift", 5.0, FloatRange::Linear { min: 0.0, max: 40.0 })
                .with_unit(" cents")
                .with_value_to_string(formatters::v2s_f32_rounded(0)),
            bend_range: IntParam::new("Bend Range", 2, IntRange::Linear { min: 0, max: 12 }).with_unit(" st"),

            vowel_x: FloatParam::new("Vowel Front-Back", 0.85, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_value_to_string(formatters::v2s_f32_rounded(2)),
            vowel_y: FloatParam::new("Vowel Close-Open", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_value_to_string(formatters::v2s_f32_rounded(2)),

            creature: EnumParam::new("Creature", CreatureParam::Songbird).non_automatable(),
            keys: EnumParam::new("Creature Keys Play", KeysParam::Hold),
            knobs: std::array::from_fn(|i| KnobParam { value: percent(&format!("Creature Knob {}", i + 1), knob_defaults[i] as f32) }),

            eco: BoolParam::new("Eco Quality", false).non_automatable(),
            restart_on_play: BoolParam::new("Restart Lyrics On Play", true),
            face: BoolParam::new("Face Control", false).non_automatable(),
            gain: FloatParam::new("Volume", 0.0, FloatRange::Linear { min: -30.0, max: 12.0 })
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_rounded(1))
                .with_smoother(SmoothingStyle::Linear(30.0)),
        }
    }
}

impl ChimeraParams {
    /// Every phrase's syllables, as the audio thread wants them.
    pub fn parse_phrases(&self) -> Vec<Vec<chimera::phonetics::Syllable>> {
        let phrases = self.phrases.read();
        (0..PHRASES).map(|i| phrases.get(i).map_or_else(Vec::new, |t| lyrics::parse(t))).collect()
    }
}

pub enum Task {
    /// build an engine, off the audio thread
    Build { sample_rate: f64, spec: VoiceSpec },
    /// listen for the face tracker, or stop
    Face(bool),
}

pub struct ChimeraVoice {
    params: Arc<ChimeraParams>,
    shared: Arc<Shared>,
    performer: Option<Performer>,
    sample_rate: f64,
    mono: Vec<f32>,
    tele: VoiceTelemetry,
    tele_wait: usize,
    scope: [f32; SCOPE_LEN],
    scope_pos: usize,
    /// the voice last asked for, and for how many samples that has stood still
    spec_seen: VoiceSpec,
    spec_still: usize,
    /// the voice the engine is running, and whether another is being built in the background
    spec_now: VoiceSpec,
    building: bool,
    was_playing: bool,
    face: face::Follower,
    /// whether the listener has been asked to run, and the key the face sings
    face_asked: bool,
    face_key: u8,
    phrase_seen: i32,
    bend: f64,
    mod_wheel: f64,
    /// the face routing (a copy, refreshed each block) and what the face does through it now
    routes: routes::Routes,
    mods: [f64; routes::NDST],
}

impl Default for ChimeraVoice {
    fn default() -> Self {
        let spec = (BodyMix::MALE, false, None);
        Self {
            params: Arc::new(ChimeraParams::default()),
            shared: Arc::new(Shared::new()),
            performer: None,
            sample_rate: 48000.0,
            mono: Vec::new(),
            tele: VoiceTelemetry::default(),
            tele_wait: 0,
            scope: [0.0; SCOPE_LEN],
            scope_pos: 0,
            spec_seen: spec,
            spec_still: 0,
            spec_now: spec,
            building: false,
            was_playing: false,
            face: face::Follower::default(),
            face_asked: false,
            face_key: 57,
            phrase_seen: 1,
            bend: 0.0,
            mod_wheel: 0.0,
            routes: routes::defaults(),
            mods: [0.0; routes::NDST],
        }
    }
}

/// The creature's first two knobs that are not its pitch: where CREATURE 1 and 2 of the face
/// routing go.
pub fn creature_free_knobs(def: &'static chimera::creatures::types::CreatureDef) -> [Option<usize>; 2] {
    let mut free = performer::creature_knobs(def).enumerate().filter(|(_, (_, p))| p.role.is_none()).map(|(k, _)| k);
    [free.next(), free.next()]
}

impl ChimeraVoice {
    /// What the face adds to each knob now, for the editor (in the settings' own units; the
    /// creature knobs normalized), and the face's signals for the routing screen.
    fn publish_face(&self, leaned: &[f32; CREATURE_KNOBS]) {
        let p = &self.params;
        let shared = &self.shared;
        let set = [0.0, p.breath.value() as f64, p.pressed.value() as f64, p.bright.value() as f64, p.breathy.value() as f64, p.rough.value() as f64, p.fry.value() as f64, p.vibrato.value() as f64 + 80.0 * self.mod_wheel];
        let t = self.tone();
        let used = [self.mods[routes::PITCH], t.pressure, t.tension, t.brightness, t.breathiness, t.roughness, t.fry, t.vibrato];
        for (i, a) in shared.face_mods.iter().enumerate() {
            let d = if i < used.len() { used[i] - set[i] } else { self.mods[i] };
            a.store((d as f32).to_bits(), Ordering::Relaxed);
        }
        for (a, d) in shared.face_knobs.iter().zip(leaned) {
            a.store(d.to_bits(), Ordering::Relaxed);
        }
        for (a, v) in shared.face_sources.iter().zip(self.face.expression().sources) {
            a.store((v as f32).to_bits(), Ordering::Relaxed);
        }
    }

    fn tone(&self) -> Tone {
        let p = &self.params;
        // the face adds to the knobs while it is tracked (nothing otherwise)
        let e = self.face.expression();
        let m = &self.mods;
        Tone {
            pressure: (p.breath.value() as f64 * e.pressure + m[routes::BREATH]).clamp(2.0, 26.0),
            tension: (p.pressed.value() as f64 + m[routes::PRESSED]).clamp(0.0, 1.0),
            brightness: (p.bright.value() as f64 + m[routes::BRIGHT]).clamp(0.0, 1.0),
            breathiness: (p.breathy.value() as f64 + m[routes::BREATHY]).clamp(0.0, 1.0),
            roughness: (p.rough.value() as f64 + m[routes::ROUGH]).clamp(0.0, 1.0),
            fry: (p.fry.value() as f64 + m[routes::FRY]).clamp(0.0, 1.0),
            vibrato: (p.vibrato.value() as f64 + 80.0 * self.mod_wheel + m[routes::VIBRATO]).clamp(0.0, 160.0),
            vibrato_rate: p.vibrato_rate.value() as f64,
            glide: p.glide.value() as f64 / 1000.0,
            drift: p.drift.value() as f64,
            whisper: p.whisper.value(),
        }
    }

    /// The voice the parameters ask for.
    fn spec(&self) -> VoiceSpec {
        let creature = (self.params.mode.value() == ModeParam::Creature).then(|| self.params.creature.value().index());
        (body_for(self.params.body.value()), self.params.eco.value(), creature)
    }

    fn handle_event(&mut self, event: PluginNoteEvent<Self>) {
        let Some(perf) = self.performer.as_mut() else { return };
        match event {
            NoteEvent::NoteOn { note, velocity, .. } => {
                // the face sings on the last key played (not a phrase keyswitch)
                if !(performer::PHRASE_KEY..performer::PHRASE_KEY + PHRASES as u8).contains(&note) {
                    self.face_key = note;
                }
                perf.note_on(note, velocity as f64)
            }
            NoteEvent::NoteOff { note, .. } | NoteEvent::Choke { note, .. } => perf.note_off(note),
            NoteEvent::MidiPitchBend { value, .. } => {
                self.bend = (value as f64 * 2.0 - 1.0) * self.params.bend_range.value() as f64;
                perf.set_bend(self.bend + self.mods[routes::PITCH]);
            }
            NoteEvent::MidiCC { cc, value, .. } => match cc {
                1 => self.mod_wheel = value as f64,
                64 => perf.set_sustain(value >= 0.5),
                120 | 123 => perf.all_notes_off(),
                _ => {}
            },
            _ => {}
        }
    }
}

impl Plugin for ChimeraVoice {
    const NAME: &'static str = "Chimera Voice";
    const VENDOR: &'static str = "Chimera";
    const URL: &'static str = "https://github.com/jamesondh/chimera-vocal-synth";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout { main_input_channels: None, main_output_channels: NonZeroU32::new(2), ..AudioIOLayout::const_default() },
        AudioIOLayout { main_input_channels: None, main_output_channels: NonZeroU32::new(1), ..AudioIOLayout::const_default() },
    ];

    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    type SysExMessage = ();
    type BackgroundTask = Task;

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn task_executor(&mut self) -> TaskExecutor<Self> {
        let shared = self.shared.clone();
        Box::new(move |task| match task {
            Task::Build { sample_rate, spec } => {
                let built = Built::new(sample_rate, spec.0, spec.1, spec.2);
                let mut b = shared.engine_box.lock();
                b.outgoing = None;
                b.incoming = Some((spec, built));
                drop(b);
                shared.engine_ready.store(true, Ordering::Release);
            }
            Task::Face(on) => {
                let mut l = shared.face_listener.lock();
                if on && l.is_none() {
                    *l = face::Listener::start(face::DEFAULT_PORT, shared.face.clone());
                } else if !on {
                    *l = None; // stops the thread
                    shared.face.status.store(face::OFF, Ordering::Release);
                }
            }
        })
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(self.params.clone(), self.shared.clone())
    }

    fn initialize(&mut self, _layout: &AudioIOLayout, config: &BufferConfig, _context: &mut impl InitContext<Self>) -> bool {
        self.sample_rate = config.sample_rate as f64;
        let spec = self.spec();
        let mut perf = Performer::with(Built::new(self.sample_rate, spec.0, spec.1, spec.2));
        self.params.phrases.write().resize(PHRASES, String::new());
        perf.set_phrases(self.params.parse_phrases());
        self.phrase_seen = self.params.phrase.value();
        perf.select_phrase((self.phrase_seen - 1).max(0) as usize);
        self.shared.state_version.fetch_add(1, Ordering::Relaxed);
        // a first snapshot sizes the telemetry's buffers, so the audio thread never grows them
        perf.telemetry(&mut self.tele);
        self.performer = Some(perf);
        self.mono = vec![0.0; (config.max_buffer_size as usize).max(64)];
        self.spec_seen = spec;
        self.spec_now = spec;
        self.spec_still = 0;
        self.building = false;
        self.bend = 0.0;
        self.mod_wheel = 0.0;
        // nothing older may arrive after this
        *self.shared.lyrics_box.lock() = Default::default();
        *self.shared.engine_box.lock() = Default::default();
        self.shared.lyrics_ready.store(false, Ordering::Release);
        self.shared.engine_ready.store(false, Ordering::Release);
        true
    }

    fn reset(&mut self) {
        if let Some(p) = self.performer.as_mut() {
            p.all_notes_off();
        }
    }

    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, context: &mut impl ProcessContext<Self>) -> ProcessStatus {
        let n = buffer.samples();
        if self.performer.is_none() || n == 0 {
            return ProcessStatus::Normal;
        }
        if self.mono.len() < n {
            self.mono.resize(n, 0.0); // a host breaking its promised maximum
        }
        if let Some(r) = self.params.face_routes.try_read() {
            self.routes = *r;
        }
        self.mods = self.face.expression().mods(&self.routes);
        let tone = self.tone();
        let shared = self.shared.clone();
        let playing = context.transport().playing;
        let want = self.spec();

        // ---- what the editor and the background task sent
        let mut leaned = [0.0f32; CREATURE_KNOBS];
        {
            let perf = self.performer.as_mut().unwrap();
            if shared.lyrics_ready.load(Ordering::Acquire) {
                if let Some(mut b) = shared.lyrics_box.try_lock() {
                    if let Some(new) = b.incoming.take() {
                        let cursor = perf.cursor();
                        b.outgoing = Some(perf.set_phrases(new));
                        perf.set_cursor(cursor); // typing does not send the singer back to the start
                    }
                    shared.lyrics_ready.store(false, Ordering::Release);
                }
            }
            if shared.engine_ready.load(Ordering::Acquire) {
                if let Some(mut b) = shared.engine_box.try_lock() {
                    if let Some((spec, built)) = b.incoming.take() {
                        let cursor = perf.cursor();
                        let old = perf.swap(built);
                        perf.set_cursor(cursor);
                        b.outgoing = Some((self.spec_now, old));
                        self.spec_now = spec;
                    }
                    shared.engine_ready.store(false, Ordering::Release);
                    self.building = false;
                }
            }

            perf.set_mode(match self.params.mode.value() {
                ModeParam::Sing => Mode::Sing,
                ModeParam::Vowel => Mode::Vowel,
                ModeParam::Creature => Mode::Creature,
                ModeParam::Beatbox => Mode::Beatbox,
            });
            perf.set_tone(&tone);
            perf.set_transpose(12.0 * self.params.octave.value() as f64);
            // the face moves the pad while it is being tracked; the parameters otherwise
            let face_on = self.params.face.value();
            if !(face_on && self.face.live()) {
                perf.set_pad(self.params.vowel_x.value() as f64, self.params.vowel_y.value() as f64);
            }
            if face_on {
                self.face.apply(&shared.face.load(), perf, self.face_key, n as f64 / self.sample_rate);
            } else {
                self.face.release(perf);
            }
            // the face through its routing: the pitch on top of the wheel, the knobs (in tone())
            self.mods = self.face.expression().mods(&self.routes);
            perf.set_bend(self.bend + self.mods[routes::PITCH]);
            perf.set_key_action(self.params.keys.value().action());
            let mut knobs = [0.0; CREATURE_KNOBS];
            for (k, p) in knobs.iter_mut().zip(self.params.knobs.iter()) {
                *k = p.value.value() as f64;
            }
            // CREATURE 1 and 2 in the routing are the creature's first two knobs that are not its pitch
            let (c1, c2) = (self.mods[routes::CREATURE_1], self.mods[routes::CREATURE_2]);
            if c1 != 0.0 || c2 != 0.0 {
                let def = performer::creature(self.params.creature.value().index());
                for (k, lean) in creature_free_knobs(def).into_iter().zip([c1, c2]) {
                    if let Some(k) = k {
                        let to = (knobs[k] + lean).clamp(0.0, 1.0);
                        leaned[k] = (to - knobs[k]) as f32;
                        knobs[k] = to;
                    }
                }
            }
            perf.set_knobs(&knobs);

            // the phrase: the parameter (when it moves), the editor, or a keyswitch
            let phrase = self.params.phrase.value();
            if phrase != self.phrase_seen {
                self.phrase_seen = phrase;
                perf.select_phrase((phrase - 1).max(0) as usize);
            }
            let req = shared.phrase_request.swap(-1, Ordering::AcqRel);
            if req >= 0 {
                perf.select_phrase(req as usize);
            }
            let req = shared.cursor_request.swap(-1, Ordering::AcqRel);
            if req >= 0 {
                perf.set_cursor(req as usize);
            }
            let req = shared.beat_request.swap(-1, Ordering::AcqRel);
            if req >= 0 {
                perf.beat(req as usize, 0.75);
            }
            if shared.panic.swap(false, Ordering::AcqRel) {
                perf.all_notes_off();
            }
            if playing && !self.was_playing && self.params.restart_on_play.value() {
                perf.set_cursor(0);
            }
            self.was_playing = playing;
        }

        // ---- the face listener: started and stopped off the audio thread
        let face_on = self.params.face.value();
        if face_on != self.face_asked {
            self.face_asked = face_on;
            context.execute_background(Task::Face(face_on));
        }

        // ---- the voice: built in the background once what is asked for has stood still a moment
        if want != self.spec_seen {
            self.spec_seen = want;
            self.spec_still = 0;
        } else {
            self.spec_still = self.spec_still.saturating_add(n);
        }
        if want != self.spec_now && !self.building && self.spec_still as f64 > 0.12 * self.sample_rate {
            self.building = true;
            context.execute_background(Task::Build { sample_rate: self.sample_rate, spec: want });
        }

        // ---- audio, split at each event so notes land on their sample
        let mut next_event = context.next_event();
        let mut pos = 0;
        while pos < n {
            let mut end = n;
            while let Some(event) = next_event {
                let at = (event.timing() as usize).min(n);
                if at <= pos {
                    self.handle_event(event);
                    next_event = context.next_event();
                } else {
                    end = at;
                    break;
                }
            }
            let tone = self.tone(); // the mod wheel may have moved
            let perf = self.performer.as_mut().unwrap();
            perf.set_tone(&tone);
            perf.render(&mut self.mono[pos..end]);
            pos = end;
        }
        while let Some(event) = next_event {
            self.handle_event(event);
            next_event = context.next_event();
        }

        for (i, mut frame) in buffer.iter_samples().enumerate() {
            let g = util::db_to_gain_fast(self.params.gain.smoothed.next());
            let mut v = self.mono[i] * g;
            if !v.is_finite() {
                v = 0.0;
            }
            for s in frame.iter_mut() {
                *s = v;
            }
            self.scope[self.scope_pos] = v;
            self.scope_pos = (self.scope_pos + 1) % SCOPE_LEN;
        }

        // ---- what the editor shows
        self.publish_face(&leaned);
        let perf = self.performer.as_mut().unwrap();
        shared.phrase.store(perf.phrase() as i64, Ordering::Relaxed);
        shared.cursor.store(perf.cursor() as i64, Ordering::Relaxed);
        shared.sounding.store(perf.sounding_index().map_or(-1, |i| i as i64), Ordering::Relaxed);
        let (beat, count) = perf.last_beat();
        shared.beat_last.store(beat as u32, Ordering::Relaxed);
        shared.beat_count.store(count, Ordering::Relaxed);
        self.tele_wait += n;
        if self.params.editor_state.is_open() && self.tele_wait as f64 >= self.sample_rate / 60.0 {
            if let Some(mut viz) = shared.viz.try_lock() {
                self.tele_wait = 0;
                viz.body = perf.body();
                for k in 0..SCOPE_LEN {
                    viz.scope[k] = self.scope[(self.scope_pos + k) % SCOPE_LEN];
                }
                viz.active.clear();
                if let Some(t) = perf.creature_telemetry() {
                    viz.creature = t.id;
                    viz.call = t.call;
                    viz.gate = t.gate as f32;
                    viz.level = t.out_rms as f32;
                    viz.f0 = 0.0;
                    viz.voicing = 0.0;
                    viz.tube_count = t.tubes.len().min(TUBES);
                    for (d, s) in viz.tubes.iter_mut().zip(t.tubes.iter()) {
                        d.name = s.name;
                        let m = s.areas.len();
                        for k in 0..TUBE_POINTS {
                            let i = if m == 0 { 0 } else { (k * m / TUBE_POINTS).min(m - 1) };
                            d.areas[k] = s.areas.get(i).copied().unwrap_or(0.0);
                            d.rms[k] = s.rms.get(i).copied().unwrap_or(0.0);
                        }
                    }
                } else {
                    perf.telemetry(&mut self.tele);
                    viz.creature = "";
                    viz.call = None;
                    for (d, s) in viz.artic.iter_mut().zip(self.tele.artic.iter()) {
                        *d = *s;
                    }
                    viz.f0 = self.tele.f0 as f32;
                    viz.voicing = self.tele.voicing as f32;
                    viz.level = self.tele.out_rms as f32;
                    viz.gate = self.tele.gate as f32;
                    for l in &self.tele.active {
                        if viz.active.len() + l.as_str().len() + 1 <= viz.active.capacity() {
                            if !viz.active.is_empty() {
                                viz.active.push(' ');
                            }
                            viz.active.push_str(l.as_str());
                        }
                    }
                }
            }
        }

        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for ChimeraVoice {
    const CLAP_ID: &'static str = "com.chimera-voice.chimera-voice";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("A physically modelled voice: sung lyrics, vowels, animal calls and beatbox");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::Instrument, ClapFeature::Synthesizer, ClapFeature::Stereo, ClapFeature::Mono];
}

nih_export_clap!(ChimeraVoice);
