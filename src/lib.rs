//! Chimera Voice: the CHIMERA physical-modelling voice engine (https://github.com/jamesondh/chimera-vocal-synth,
//! MIT, © Jameson Hodge) as a monophonic CLAP instrument.

use chimera::anatomy::human::BodyMix;
use chimera::engine::Engine;
use chimera::voice::human_voice::VoiceTelemetry;
use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use parking_lot::RwLock;
use std::sync::atomic::Ordering;
use std::sync::Arc;

mod editor;
pub mod lyrics;
pub mod performer;
pub mod shared;

use performer::{Mode, Performer, Tone};
use shared::{Shared, SCOPE_LEN};

pub const DEFAULT_LYRICS: &str = "Hello there, I am a voice made of air.";

#[derive(Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeParam {
    #[id = "sing"]
    #[name = "Sing lyrics"]
    Sing,
    #[id = "vowel"]
    #[name = "Vowel pad"]
    Vowel,
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
    #[persist = "lyrics"]
    pub lyrics: Arc<RwLock<String>>,

    #[id = "mode"]
    pub mode: EnumParam<ModeParam>,
    #[id = "body"]
    pub body: FloatParam,
    #[id = "octave"]
    pub octave: IntParam,

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

    #[id = "eco"]
    pub eco: BoolParam,
    #[id = "restart"]
    pub restart_on_play: BoolParam,
    #[id = "gain"]
    pub gain: FloatParam,
}

fn percent(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_unit(" %")
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

impl Default for ChimeraParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(editor::WIDTH, editor::HEIGHT),
            lyrics: Arc::new(RwLock::new(DEFAULT_LYRICS.to_string())),

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

            eco: BoolParam::new("Eco Quality", false).non_automatable(),
            restart_on_play: BoolParam::new("Restart Lyrics On Play", true),
            gain: FloatParam::new("Volume", 0.0, FloatRange::Linear { min: -30.0, max: 12.0 })
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_rounded(1))
                .with_smoother(SmoothingStyle::Linear(30.0)),
        }
    }
}

pub enum Task {
    /// build an engine for this body, off the audio thread
    BuildEngine { sample_rate: f64, body: BodyMix, eco: bool },
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
    /// the voice (body, eco) last asked for, and for how many samples that has stood still
    body_seen: (BodyMix, bool),
    body_still: usize,
    /// the voice the engine is running, and whether another is being built in the background
    voice_now: (BodyMix, bool),
    body_building: bool,
    was_playing: bool,
    bend: f64,
    mod_wheel: f64,
}

impl Default for ChimeraVoice {
    fn default() -> Self {
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
            body_seen: (BodyMix::MALE, false),
            body_still: 0,
            voice_now: (BodyMix::MALE, false),
            body_building: false,
            was_playing: false,
            bend: 0.0,
            mod_wheel: 0.0,
        }
    }
}

impl ChimeraVoice {
    fn tone(&self) -> Tone {
        let p = &self.params;
        Tone {
            pressure: p.breath.value() as f64,
            tension: p.pressed.value() as f64,
            brightness: p.bright.value() as f64,
            breathiness: p.breathy.value() as f64,
            roughness: p.rough.value() as f64,
            fry: p.fry.value() as f64,
            vibrato: (p.vibrato.value() as f64 + 80.0 * self.mod_wheel).min(160.0),
            vibrato_rate: p.vibrato_rate.value() as f64,
            glide: p.glide.value() as f64 / 1000.0,
            drift: p.drift.value() as f64,
            whisper: p.whisper.value(),
        }
    }

    fn handle_event(&mut self, event: PluginNoteEvent<Self>) {
        let Some(perf) = self.performer.as_mut() else { return };
        match event {
            NoteEvent::NoteOn { note, velocity, .. } => perf.note_on(note, velocity as f64),
            NoteEvent::NoteOff { note, .. } | NoteEvent::Choke { note, .. } => perf.note_off(note),
            NoteEvent::MidiPitchBend { value, .. } => {
                self.bend = (value as f64 * 2.0 - 1.0) * self.params.bend_range.value() as f64;
                perf.set_bend(self.bend);
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
            Task::BuildEngine { sample_rate, body, eco } => {
                let engine: Engine = Performer::build_engine(sample_rate, body, eco);
                let mut b = shared.engine_box.lock();
                b.outgoing = None;
                b.incoming = Some((body, eco, engine));
                drop(b);
                shared.engine_ready.store(true, Ordering::Release);
            }
        })
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(self.params.clone(), self.shared.clone())
    }

    fn initialize(&mut self, _layout: &AudioIOLayout, config: &BufferConfig, _context: &mut impl InitContext<Self>) -> bool {
        self.sample_rate = config.sample_rate as f64;
        let body = body_for(self.params.body.value());
        let eco = self.params.eco.value();
        let mut perf = Performer::with_engine(Performer::build_engine(self.sample_rate, body, eco), body);
        let syllables = lyrics::parse(&self.params.lyrics.read());
        *self.shared.chips.lock() = syllables.iter().map(lyrics::written).collect();
        self.shared.chips_version.fetch_add(1, Ordering::Relaxed);
        perf.set_lyrics(syllables);
        // a first snapshot sizes the telemetry's buffers, so the audio thread never grows them
        perf.telemetry(&mut self.tele);
        self.performer = Some(perf);
        self.mono = vec![0.0; (config.max_buffer_size as usize).max(64)];
        self.body_seen = (body, eco);
        self.voice_now = (body, eco);
        self.body_still = 0;
        self.body_building = false;
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
        let tone = self.tone();
        let shared = self.shared.clone();
        let playing = context.transport().playing;

        // ---- what the editor and the background task sent
        {
            let perf = self.performer.as_mut().unwrap();
            if shared.lyrics_ready.load(Ordering::Acquire) {
                if let Some(mut b) = shared.lyrics_box.try_lock() {
                    if let Some(new) = b.incoming.take() {
                        b.outgoing = Some(perf.set_lyrics(new));
                    }
                    shared.lyrics_ready.store(false, Ordering::Release);
                }
            }
            if shared.engine_ready.load(Ordering::Acquire) {
                if let Some(mut b) = shared.engine_box.try_lock() {
                    if let Some((body, eco, engine)) = b.incoming.take() {
                        let cursor = perf.cursor();
                        let old = perf.swap_engine(engine, body);
                        perf.set_cursor(cursor);
                        b.outgoing = Some((body, eco, old));
                        self.voice_now = (body, eco);
                    }
                    shared.engine_ready.store(false, Ordering::Release);
                    self.body_building = false;
                }
            }
            let req = shared.cursor_request.swap(-1, Ordering::AcqRel);
            if req >= 0 {
                perf.set_cursor(req as usize);
            }
            if shared.panic.swap(false, Ordering::AcqRel) {
                perf.all_notes_off();
            }
            if playing && !self.was_playing && self.params.restart_on_play.value() {
                perf.set_cursor(0);
            }
            self.was_playing = playing;

            perf.set_mode(match self.params.mode.value() {
                ModeParam::Sing => Mode::Sing,
                ModeParam::Vowel => Mode::Vowel,
            });
            perf.set_tone(&tone);
            perf.set_transpose(12.0 * self.params.octave.value() as f64);
            perf.set_pad(self.params.vowel_x.value() as f64, self.params.vowel_y.value() as f64);
        }

        // ---- the body: built in the background once the slider has stood still for a moment
        let want = (body_for(self.params.body.value()), self.params.eco.value());
        if want != self.body_seen {
            self.body_seen = want;
            self.body_still = 0;
        } else {
            self.body_still = self.body_still.saturating_add(n);
        }
        if want != self.voice_now && !self.body_building && self.body_still as f64 > 0.12 * self.sample_rate {
            self.body_building = true;
            context.execute_background(Task::BuildEngine { sample_rate: self.sample_rate, body: want.0, eco: want.1 });
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
        let perf = self.performer.as_mut().unwrap();
        shared.cursor.store(perf.cursor() as i64, Ordering::Relaxed);
        shared.sounding.store(perf.sounding_index().map_or(-1, |i| i as i64), Ordering::Relaxed);
        self.tele_wait += n;
        if self.params.editor_state.is_open() && self.tele_wait as f64 >= self.sample_rate / 60.0 {
            if let Some(mut viz) = shared.viz.try_lock() {
                self.tele_wait = 0;
                perf.telemetry(&mut self.tele);
                for (d, s) in viz.artic.iter_mut().zip(self.tele.artic.iter()) {
                    *d = *s;
                }
                viz.body = perf.body();
                viz.f0 = self.tele.f0 as f32;
                viz.voicing = self.tele.voicing as f32;
                viz.level = self.tele.out_rms as f32;
                for k in 0..SCOPE_LEN {
                    viz.scope[k] = self.scope[(self.scope_pos + k) % SCOPE_LEN];
                }
                viz.active.clear();
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

        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for ChimeraVoice {
    const CLAP_ID: &'static str = "com.chimera-voice.chimera-voice";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("A physically modelled singing voice: type lyrics, play notes");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::Instrument, ClapFeature::Synthesizer, ClapFeature::Stereo, ClapFeature::Mono];
}

nih_export_clap!(ChimeraVoice);
