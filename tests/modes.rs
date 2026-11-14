//! Headless checks of the creature and beatbox modes, the phonetic ways of writing lyrics, and
//! the phrase keys. Each sounding test leaves a WAV in target/renders to listen to.

use chimera_voice::lyrics;
use chimera_voice::performer::{self, KeyAction, Mode, Performer, Tone};

const FS: f64 = 48000.0;

fn render(p: &mut Performer, out: &mut Vec<f32>, seconds: f64) {
    let n = (seconds * FS) as usize;
    let start = out.len();
    out.resize(start + n, 0.0);
    for chunk in out[start..].chunks_mut(441) {
        p.render(chunk);
    }
}

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

/// Fundamental by autocorrelation (Hz), looked for within half an octave of `near`.
fn pitch_near(x: &[f32], near: f64) -> f64 {
    let (lo, hi) = ((FS / (near * 1.41)) as usize, (FS / (near / 1.41)) as usize);
    let n = x.len() - hi;
    let corr = |lag: usize| (0..n).map(|i| x[i] as f64 * x[i + lag] as f64).sum::<f64>();
    let mut best = (f64::MIN, lo);
    for lag in lo..=hi {
        let c = corr(lag);
        if c > best.0 {
            best = (c, lag);
        }
    }
    FS / best.1 as f64
}

fn hz(key: u8) -> f64 {
    440.0 * 2f64.powf((key as f64 - 69.0) / 12.0)
}

fn cents(a: f64, b: f64) -> f64 {
    1200.0 * (a / b).log2()
}

fn save(name: &str, x: &[f32]) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("renders");
    std::fs::create_dir_all(&dir).unwrap();
    let mut b = Vec::with_capacity(44 + x.len() * 2);
    let len = (x.len() * 2) as u32;
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&(FS as u32).to_le_bytes());
    b.extend_from_slice(&(FS as u32 * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&len.to_le_bytes());
    for v in x {
        b.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(dir.join(name), b).unwrap();
}

#[test]
fn every_creature_sounds_held_and_called() {
    let mut out = Vec::new();
    for c in 0..4 {
        let def = performer::creature(c);
        let mut p = Performer::new(FS);
        p.set_mode(Mode::Creature);
        // nothing sounds while the human engine is still in place
        p.note_on(60, 0.8);
        render(&mut p, &mut out, 0.1);
        assert!(rms(&out[out.len() - (0.1 * FS) as usize..]) < 0.003, "a key sounded before the creature was there");
        p.note_off(60);
        p.set_creature(Some(c));
        p.set_knobs(&performer::creature_defaults(def));
        // a held key breathes until it is let go
        p.note_on(60, 0.8);
        let at = out.len();
        render(&mut p, &mut out, 1.0);
        let held = rms(&out[at + (0.4 * FS) as usize..]);
        assert!(held > 0.004, "{}: a held note is silent ({held})", def.id);
        assert!(p.creature_telemetry().is_some_and(|t| t.id == def.id && !t.tubes.is_empty()));
        p.note_off(60);
        render(&mut p, &mut out, 1.2);
        let after = rms(&out[out.len() - (0.2 * FS) as usize..]);
        assert!(after < 0.25 * held, "{}: it keeps sounding after the key ({after} vs {held})", def.id);
        // every call makes a sound of its own
        for call in 0..def.calls.len() {
            p.set_key_action(KeyAction::Call(call));
            p.note_on(64, 0.8);
            p.note_off(64);
            let at = out.len();
            render(&mut p, &mut out, 1.5);
            assert!(rms(&out[at..]) > 0.003, "{} {}: silent", def.id, def.calls[call].id);
        }
        p.all_notes_off();
        render(&mut p, &mut out, 0.5);
        assert!(out.iter().all(|v| v.is_finite()), "{}", def.id);
        println!("{}: held {held:.3}, {} calls", def.id, def.calls.len());
    }
    save("creatures.wav", &out);
}

#[test]
fn the_keys_play_the_drums_and_hum_a_bass() {
    let mut p = Performer::new(FS);
    p.set_mode(Mode::Beatbox);
    let mut out = Vec::new();
    render(&mut p, &mut out, 0.3);
    for key in 36u8..48 {
        let sound = performer::beat_for_key(key).unwrap();
        p.note_on(key, 0.8);
        p.note_off(key);
        let at = out.len();
        // the crash and the open hat hiss for a while
        render(&mut p, &mut out, 1.2);
        let peak = out[at..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak > 0.02, "key {key} (sound {sound}) is silent: peak {peak}");
        assert_eq!(p.last_beat().0, sound);
        assert!(rms(&out[out.len() - (0.08 * FS) as usize..]) < 0.01, "key {key} rings on");
    }
    // any other key hums its note for as long as it is held
    p.set_tone(&Tone { vibrato: 0.0, drift: 0.0, ..Tone::default() });
    p.note_on(52, 0.8);
    let at = out.len();
    render(&mut p, &mut out, 1.2);
    let held = &out[at + (0.6 * FS) as usize..];
    let got = pitch_near(held, hz(52));
    assert!(cents(got, hz(52)).abs() < 30.0, "hum at {got} Hz, wanted {}", hz(52));
    assert!(rms(held) > 0.01, "the hum is too quiet: {}", rms(held));
    p.note_off(52);
    render(&mut p, &mut out, 0.6);
    assert!(rms(&out[out.len() - (0.1 * FS) as usize..]) < 0.003, "the hum does not stop");
    // a pad click
    p.beat(0, 0.75);
    render(&mut p, &mut out, 0.4);
    assert!(out.iter().all(|v| v.is_finite()));
    save("beatbox.wav", &out);
}

#[test]
fn lyrics_can_be_spelled_in_sounds() {
    let text = |s: &str| lyrics::parse(s).iter().map(lyrics::written).collect::<Vec<_>>().join(" ");
    // sound codes: one word per group, "/" between words, a lone "-" holds the vowel
    let codes = lyrics::parse("{HH AH L OW / W ER L D} again");
    assert_eq!(codes.len(), 5, "{}", text("{HH AH L OW / W ER L D} again"));
    assert_eq!(codes[0].nucleus.ipa, "ə");
    assert_eq!(codes[1].nucleus.ipa, "oʊ");
    assert!(codes[1].word_end && !codes[0].word_end);
    assert_eq!(codes[2].coda.len(), 2);
    assert_eq!(lyrics::parse("{hh ah l ow}").len(), 2, "lower case codes work too");
    let held = lyrics::parse("{G L OW - - R IY}");
    assert_eq!(held.iter().filter(|s| s.melisma == Some(true)).count(), 2, "{}", text("{G L OW - - R IY}"));
    // IPA: spaces between words
    let ipa = lyrics::parse("[həloʊ wɝld]");
    assert_eq!(ipa.len(), 3, "{}", text("[həloʊ wɝld]"));
    assert_eq!(ipa[2].nucleus.ipa, "ɝ");
    // the same word spelled three ways sings the same vowel
    let plain = lyrics::parse("hello");
    assert_eq!(plain.len(), 2);
    assert_eq!(plain[1].nucleus.ipa, codes[1].nucleus.ipa);
    // what cannot be sung is reported, and what can is not
    assert!(lyrics::problems("hello {HH AH L OW} [həloʊ]").is_empty(), "{:?}", lyrics::problems("hello {HH AH L OW} [həloʊ]"));
    assert!(!lyrics::problems("{HH QQ}").is_empty());
    // an unclosed bracket does not lose the words before it
    assert_eq!(lyrics::parse("la la {L AA").len(), 3);

    // and it sings
    let mut p = Performer::new(FS);
    p.set_lyrics(lyrics::parse("{S T R EY N JH} [wɝld]"));
    let mut out = Vec::new();
    for key in [50u8, 52] {
        p.note_on(key, 0.8);
        let at = out.len();
        render(&mut p, &mut out, 0.6);
        assert!(rms(&out[at + (0.3 * FS) as usize..]) > 0.02);
        p.note_off(key);
        render(&mut p, &mut out, 0.3);
    }
    save("phonetic.wav", &out);
}

#[test]
fn low_keys_pick_the_phrase() {
    let mut p = Performer::new(FS);
    p.set_phrases(vec![lyrics::parse("one two three"), lyrics::parse("la la"), Vec::new()]);
    let mut out = Vec::new();
    p.note_on(57, 0.8);
    assert_eq!((p.phrase(), p.sounding_index(), p.cursor()), (0, Some(0), 1));
    p.note_off(57);
    render(&mut p, &mut out, 0.3);
    // the second keyswitch: phrase 2 from its start, and the key itself sings nothing
    p.note_on(performer::PHRASE_KEY + 1, 0.8);
    p.note_off(performer::PHRASE_KEY + 1);
    assert_eq!((p.phrase(), p.cursor(), p.sounding_index()), (1, 0, None));
    assert_eq!(p.syllable_count(), 2);
    p.note_on(57, 0.8);
    assert_eq!(p.sounding_index(), Some(0));
    p.note_off(57);
    render(&mut p, &mut out, 0.3);
    // an empty phrase sings "ah"
    p.select_phrase(2);
    p.note_on(57, 0.8);
    let at = out.len();
    render(&mut p, &mut out, 0.5);
    assert!(rms(&out[at + (0.2 * FS) as usize..]) > 0.02);
    // in the other modes those keys are ordinary notes
    p.set_mode(Mode::Vowel);
    p.note_on(performer::PHRASE_KEY, 0.8);
    assert_eq!(p.phrase(), 2);
}
