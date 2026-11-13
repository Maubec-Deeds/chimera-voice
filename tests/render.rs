//! Headless checks of the performer: it sings in tune, both modes sound, and it runs faster
//! than real time. Each test also leaves a WAV in target/renders to listen to.

use chimera::anatomy::human::BodyMix;
use chimera_voice::lyrics;
use chimera_voice::performer::{Mode, Performer, Tone};
use std::time::Instant;

const FS: f64 = 48000.0;

fn render(p: &mut Performer, out: &mut Vec<f32>, seconds: f64) {
    let n = (seconds * FS) as usize;
    let start = out.len();
    out.resize(start + n, 0.0);
    // odd block sizes, as a host might hand out
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
    // refine between samples with a parabola through the peak
    let (a, b, c) = (corr(best.1 - 1), best.0, corr(best.1 + 1));
    let d = 0.5 * (a - c) / (a - 2.0 * b + c);
    FS / (best.1 as f64 + d)
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

fn cents(a: f64, b: f64) -> f64 {
    1200.0 * (a / b).log2()
}

fn hz(key: u8) -> f64 {
    440.0 * 2f64.powf((key as f64 - 69.0) / 12.0)
}

#[test]
fn sings_the_lyrics_in_tune() {
    let mut p = Performer::new(FS);
    let syl = lyrics::parse("Hello there, I am a voice made of air.");
    assert_eq!(syl.len(), 10, "{:?}", syl.iter().map(|s| s.text.clone()).collect::<Vec<_>>());
    p.set_lyrics(syl);
    p.set_tone(&Tone { vibrato: 0.0, drift: 0.0, ..Tone::default() });
    let mut out = Vec::new();
    let melody = [45u8, 47, 49, 50, 52, 50, 49, 47, 45, 52];
    let started = Instant::now();
    for (i, key) in melody.iter().enumerate() {
        p.note_on(*key, 0.75);
        assert_eq!(p.sounding_index(), Some(i));
        let at = out.len();
        render(&mut p, &mut out, 0.5);
        let held = &out[at + (0.3 * FS) as usize..];
        let got = pitch_near(held, hz(*key));
        assert!(cents(got, hz(*key)).abs() < 25.0, "note {i}: {got:.1} Hz, wanted {:.1}", hz(*key));
        assert!(rms(held) > 0.02, "note {i} is too quiet: {}", rms(held));
        p.note_off(*key);
        render(&mut p, &mut out, 0.12);
    }
    render(&mut p, &mut out, 0.8);
    let took = started.elapsed().as_secs_f64();
    let seconds = out.len() as f64 / FS;
    println!("sing: {:.2} s of audio in {:.2} s ({:.0}% of real time)", seconds, took, 100.0 * took / seconds);
    assert!(out.iter().all(|v| v.is_finite()));
    assert!(rms(&out[out.len() - (0.3 * FS) as usize..]) < 0.003, "it does not fall silent");
    assert!(took < seconds, "slower than real time");
    assert_eq!(p.cursor(), 0, "the lyrics wrap round");
    save("sing.wav", &out);
}

#[test]
fn legato_and_melisma_keep_the_voice_going() {
    let mut p = Performer::new(FS);
    p.set_lyrics(lyrics::parse("glo - - ri a"));
    assert_eq!(p.syllable_count(), 5);
    p.set_tone(&Tone { vibrato: 0.0, drift: 0.0, ..Tone::default() });
    let mut out = Vec::new();
    p.note_on(57, 0.8);
    render(&mut p, &mut out, 0.5);
    // overlapping keys: the melisma glides without a gap
    p.note_on(59, 0.8);
    p.note_off(57);
    render(&mut p, &mut out, 0.4);
    assert_eq!(p.sounding_index(), Some(0), "a melisma keeps its syllable");
    let at = out.len();
    p.note_on(61, 0.8);
    p.note_off(59);
    render(&mut p, &mut out, 0.5);
    assert!(rms(&out[at..at + (0.1 * FS) as usize]) > 0.02, "the glide broke the vowel");
    let got = pitch_near(&out[at + (0.3 * FS) as usize..], hz(61));
    assert!(cents(got, hz(61)).abs() < 25.0, "{got}");
    p.note_on(62, 0.8);
    p.note_off(61);
    render(&mut p, &mut out, 0.4);
    p.note_on(64, 0.8);
    p.note_off(62);
    render(&mut p, &mut out, 0.6);
    p.note_off(64);
    render(&mut p, &mut out, 0.6);
    assert!(out.iter().all(|v| v.is_finite()));
    save("melisma.wav", &out);
}

#[test]
fn the_vowel_pad_plays_and_changes_the_sound() {
    let mut out = Vec::new();
    let mut edge = Vec::new();
    for body in [BodyMix::MALE, BodyMix::FEMALE, BodyMix::CHILD, BodyMix::new(0.5, 0.0)] {
        let mut p = Performer::new(FS);
        let t = Instant::now();
        p.set_body(body);
        println!("body {:?} built in {:.1} ms", body, t.elapsed().as_secs_f64() * 1e3);
        p.set_mode(Mode::Vowel);
        p.set_tone(&Tone { vibrato: 0.0, drift: 0.0, ..Tone::default() });
        let key = if body == BodyMix::MALE { 48 } else { 60 };
        for (x, y) in [(0.0, 0.0), (0.85, 1.0), (1.0, 0.0)] {
            p.set_pad(x, y);
            p.note_on(key, 0.8);
            let at = out.len();
            render(&mut p, &mut out, 0.6);
            let held = &out[at + (0.35 * FS) as usize..];
            let got = pitch_near(held, hz(key));
            assert!(cents(got, hz(key)).abs() < 25.0, "{:?} {x},{y}: {got} Hz, wanted {}", body, hz(key));
            assert!(rms(held) > 0.02);
            // how much of the energy changes from sample to sample: high for a bright vowel
            let diff: f64 = held.windows(2).map(|w| ((w[1] - w[0]) as f64).powi(2)).sum::<f64>().sqrt();
            edge.push(diff / (rms(held) * (held.len() as f64).sqrt()));
            p.note_off(key);
            render(&mut p, &mut out, 0.3);
            assert!(rms(&out[out.len() - (0.05 * FS) as usize..]) < 0.01, "the note does not stop");
        }
    }
    println!("edge (ee, ah, oo per body): {:.3?}", edge);
    // the three corners of the pad sound different, in the same order for every body
    for b in edge.chunks(3) {
        assert!(b[1] > b[0] * 1.1 && b[0] > b[2] * 1.05, "ee {} ah {} oo {}", b[0], b[1], b[2]);
    }
    assert!(out.iter().all(|v| v.is_finite()));
    save("vowels.wav", &out);
}

#[test]
fn bend_sustain_and_panic() {
    let mut p = Performer::new(FS);
    p.set_lyrics(lyrics::parse("la la la"));
    p.set_tone(&Tone { vibrato: 0.0, drift: 0.0, ..Tone::default() });
    let mut out = Vec::new();
    p.note_on(57, 0.8);
    render(&mut p, &mut out, 0.4);
    p.set_bend(2.0);
    let at = out.len();
    render(&mut p, &mut out, 0.6);
    let got = pitch_near(&out[at + (0.35 * FS) as usize..], hz(59));
    assert!(cents(got, hz(59)).abs() < 25.0, "bent to {got}");
    p.set_bend(0.0);
    p.set_sustain(true);
    p.note_off(57);
    let at = out.len();
    render(&mut p, &mut out, 0.5);
    assert!(rms(&out[at + (0.3 * FS) as usize..]) > 0.02, "the pedal holds the note");
    p.set_sustain(false);
    render(&mut p, &mut out, 0.6);
    assert!(rms(&out[out.len() - (0.1 * FS) as usize..]) < 0.003);
    p.note_on(60, 0.8);
    render(&mut p, &mut out, 0.3);
    p.all_notes_off();
    render(&mut p, &mut out, 0.5);
    assert!(rms(&out[out.len() - (0.1 * FS) as usize..]) < 0.003, "silence after a panic");
    save("bend.wav", &out);
}

#[test]
fn eco_quality_holds_and_costs_less() {
    let mut cost = Vec::new();
    for eco in [false, true] {
        let mut p = Performer::with_engine(Performer::build_engine(FS, BodyMix::FEMALE, eco), BodyMix::FEMALE);
        p.set_lyrics(lyrics::parse("Hello there, I am a voice made of air."));
        p.set_tone(&Tone { vibrato: 0.0, drift: 0.0, ..Tone::default() });
        let mut out = Vec::new();
        let t = Instant::now();
        for key in [57u8, 59, 61, 62, 64, 62, 61, 59, 57, 64] {
            p.note_on(key, 0.8);
            let at = out.len();
            render(&mut p, &mut out, 0.5);
            let held = &out[at + (0.3 * FS) as usize..];
            let got = pitch_near(held, hz(key));
            assert!(cents(got, hz(key)).abs() < 25.0, "eco {eco}: {got} Hz, wanted {}", hz(key));
            assert!(rms(held) > 0.02 && rms(held) < 0.7, "eco {eco}: level {}", rms(held));
            p.note_off(key);
            render(&mut p, &mut out, 0.12);
        }
        render(&mut p, &mut out, 0.6);
        cost.push(t.elapsed().as_secs_f64() / (out.len() as f64 / FS));
        assert!(out.iter().all(|v| v.is_finite()));
        assert!(rms(&out[out.len() - (0.2 * FS) as usize..]) < 0.003);
        save(if eco { "eco.wav" } else { "full.wav" }, &out);
    }
    println!("full {:.0}% of real time, eco {:.0}%", 100.0 * cost[0], 100.0 * cost[1]);
    assert!(cost[1] < 0.75 * cost[0]);
}
