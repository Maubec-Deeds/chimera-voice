//! Lyrics text → sung syllables, with the CMU Pronouncing Dictionary built in (one copy per
//! process, shared by every instance of the plugin).
//!
//! Plain words are read as English (also Hangul and Russian). Two more ways to write a sound:
//! - `{HH AH L OW}`: the dictionary's own sound codes (ARPAbet), for a keyboard without phonetic
//!   symbols. One word per run of codes; a `/` starts the next word.
//! - `[həˈloʊ]`: IPA. Spaces separate words.

use chimera::phonetics::english::{arpabet_to_segments, EnglishG2P};
use chimera::phonetics::ipa::{skipped_ipa, tokenize_ipa};
use chimera::phonetics::syllables::{lyrics_to_syllables, syllabify_segments};
use chimera::phonetics::{Segment, Syllable};
use std::io::Read;
use std::sync::OnceLock;

static DICT_GZ: &[u8] = include_bytes!("../vendor/chimera-vocal-synth/public/data/cmudict.txt.gz");

/// The English pronouncer. The first call unpacks the dictionary (about a tenth of a second):
/// never call it first from the audio thread.
pub fn g2p() -> &'static EnglishG2P {
    static G2P: OnceLock<EnglishG2P> = OnceLock::new();
    G2P.get_or_init(|| {
        let mut g = EnglishG2P::new();
        let mut text = String::new();
        // without the dictionary the letter-to-sound rules still read every word
        if flate2::read::GzDecoder::new(DICT_GZ).read_to_string(&mut text).is_ok() {
            g.load_dict(&text);
        }
        g
    })
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Words,
    Codes,
    Ipa,
}

/// The text cut into runs of plain words, `{codes}` and `[IPA]`. An unclosed bracket runs to the
/// end of the text.
fn runs(text: &str) -> Vec<(Kind, &str)> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        match rest.find(['{', '[']) {
            None => {
                out.push((Kind::Words, rest));
                break;
            }
            Some(open) => {
                if open > 0 {
                    out.push((Kind::Words, &rest[..open]));
                }
                let (kind, close) = if rest.as_bytes()[open] == b'{' { (Kind::Codes, '}') } else { (Kind::Ipa, ']') };
                let body = &rest[open + 1..];
                let end = body.find(close).unwrap_or(body.len());
                out.push((kind, &body[..end]));
                rest = body.get(end + 1..).unwrap_or("");
            }
        }
    }
    out
}

fn is_punct(c: char) -> bool {
    matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | '\n')
}

/// One phonetic word's sounds → its syllables, appended to `out`.
fn push_word(segs: Vec<Segment>, out: &mut Vec<Syllable>) {
    let segs: Vec<Segment> = segs.into_iter().filter(|s| s.ipa != "_").collect();
    let parts = syllabify_segments(&segs);
    let n = parts.len();
    for (i, p) in parts.into_iter().enumerate() {
        let text: String = p.onset.iter().chain(std::iter::once(&p.nucleus)).chain(p.coda.iter()).map(|s| s.ipa.as_str()).collect();
        out.push(Syllable { text, onset: p.onset, nucleus: p.nucleus, coda: p.coda, word_end: i + 1 == n, ..Default::default() });
    }
}

fn end_phrase(out: &mut [Syllable]) {
    if let Some(last) = out.last_mut() {
        last.phrase_end = true;
    }
}

/// A lone - or ~: the previous vowel held onto the next note.
fn push_melisma(out: &mut Vec<Syllable>) {
    if let Some(prev) = out.last() {
        let s = Syllable { text: "~".into(), nucleus: prev.nucleus.clone(), melisma: Some(true), word_end: prev.word_end, ..Default::default() };
        out.push(s);
    }
}

fn is_hold(tok: &str) -> bool {
    tok == "-" || tok == "~"
}

/// A run of IPA → syllables: spaces separate the words.
fn ipa_run(run: &str, out: &mut Vec<Syllable>) {
    for line in run.split('\n') {
        for tok in line.split_whitespace() {
            if is_hold(tok) {
                push_melisma(out);
                continue;
            }
            push_word(tokenize_ipa(tok.trim_end_matches(is_punct)), out);
            if tok.ends_with(is_punct) {
                end_phrase(out);
            }
        }
        if run.contains('\n') {
            end_phrase(out);
        }
    }
}

/// A run of sound codes → syllables: spaces separate the codes, a `/` the words.
fn code_run(run: &str, out: &mut Vec<Syllable>) {
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut Vec<Syllable>| {
        if !word.is_empty() {
            push_word(arpabet_to_segments(&word.to_uppercase()), out);
            word.clear();
        }
    };
    for line in run.split('\n') {
        for tok in line.split_whitespace() {
            if tok == "/" {
                flush(&mut word, out);
            } else if is_hold(tok) {
                flush(&mut word, out);
                push_melisma(out);
            } else {
                word.push_str(tok.trim_end_matches(is_punct));
                word.push(' ');
                if tok.ends_with(is_punct) {
                    flush(&mut word, out);
                    end_phrase(out);
                }
            }
        }
        flush(&mut word, out);
        if run.contains('\n') {
            end_phrase(out);
        }
    }
}

/// `_` counts as a space everywhere, for hosts that keep the space bar for themselves (Bitwig
/// starts playback with it).
fn spaced(text: &str) -> String {
    text.replace('_', " ")
}

/// Lyrics → syllables. A lone `-` or `~` holds the vowel onto the next note; punctuation and line
/// breaks end a phrase.
pub fn parse(text: &str) -> Vec<Syllable> {
    let text = spaced(text);
    let mut out = Vec::new();
    for (kind, run) in runs(&text) {
        match kind {
            Kind::Words => out.extend(lyrics_to_syllables(run, g2p())),
            Kind::Codes => code_run(run, &mut out),
            Kind::Ipa => ipa_run(run, &mut out),
        }
    }
    out
}

/// What in the phonetic parts of the lyrics the voice will not say, one line per kind (empty when
/// everything is sung).
pub fn problems(text: &str) -> Vec<String> {
    let text = spaced(text);
    let mut out = Vec::new();
    for (kind, run) in runs(&text) {
        match kind {
            Kind::Words => {}
            Kind::Ipa => out.extend(skipped_ipa(run).warnings()),
            Kind::Codes => {
                let bad: Vec<&str> = run
                    .split(|c: char| c.is_whitespace() || c == '/')
                    .filter(|t| !t.is_empty() && *t != "-" && *t != "~")
                    .map(|t| t.trim_end_matches(is_punct))
                    .filter(|t| !t.is_empty() && arpabet_to_segments(&t.to_uppercase()).is_empty())
                    .collect();
                if !bad.is_empty() {
                    out.push(format!("not a sound code: {}", bad.join(" ")));
                }
            }
        }
    }
    out
}

/// What a syllable looks like on its chip: a hyphen when its word goes on.
pub fn written(s: &Syllable) -> String {
    if s.melisma == Some(true) {
        "~".to_string()
    } else if s.word_end {
        s.text.clone()
    } else {
        format!("{}-", s.text)
    }
}

/// The sound codes, with a word that has each sound (for the editor's help).
pub const CODES: &[(&str, &str)] = &[
    ("AA", "father"), ("AE", "cat"), ("AH", "cut"), ("AO", "law"), ("AW", "cow"), ("AY", "my"), ("EH", "bed"), ("ER", "bird"),
    ("EY", "say"), ("IH", "sit"), ("IY", "see"), ("OW", "go"), ("OY", "boy"), ("UH", "book"), ("UW", "too"),
    ("B", "bee"), ("CH", "chin"), ("D", "day"), ("DH", "this"), ("F", "fee"), ("G", "go"), ("HH", "he"), ("JH", "joy"),
    ("K", "key"), ("L", "lay"), ("M", "me"), ("N", "no"), ("NG", "sing"), ("P", "pay"), ("R", "ray"), ("S", "see"),
    ("SH", "she"), ("T", "tea"), ("TH", "thin"), ("V", "vow"), ("W", "we"), ("Y", "yes"), ("Z", "zoo"), ("ZH", "vision"),
];
