//! Lyrics text → sung syllables, with the CMU Pronouncing Dictionary built in (one copy per
//! process, shared by every instance of the plugin).

use chimera::phonetics::english::EnglishG2P;
use chimera::phonetics::syllables::lyrics_to_syllables;
use chimera::phonetics::Syllable;
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

/// English, Korean (Hangul) and Russian lyrics → syllables. A lone `-` or `~` holds the vowel
/// onto the next note; punctuation and line breaks end a phrase.
pub fn parse(text: &str) -> Vec<Syllable> {
    lyrics_to_syllables(text, g2p())
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
