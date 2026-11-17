//! Face routing: which part of the face changes which sound setting, and by how much.
//!
//! A table of amounts, one per (face signal, setting) pair, from −1 to 1 (0: not connected). The
//! face's signals run 0..1 (eyebrows and head tilt −1..1), and an amount of 1 with the signal
//! full moves the setting across its knob's whole range (pitch: an octave; the creature knobs:
//! their whole range). The table is saved with the project.
//!
//! The mouth's own work (starting and ending notes, loudness, the vowel, the lips) is not in the
//! table: it is always on.

/// The face's signals, in the table's row order.
pub const SOURCES: [&str; 8] = ["OPEN", "JAW", "ROUND", "SPREAD", "LIP ON TEETH", "SMILE", "EYEBROWS", "HEAD TILT"];
/// What each signal is, for the table's tooltips.
pub const SOURCE_TIPS: [&str; 8] = [
    "How far your lips are apart (0 shut, 1 as wide as the calibrated \"aah\").",
    "How far your jaw has dropped (0 at rest, 1 as in the calibrated \"aah\").",
    "How pushed forward and rounded your lips are, as for \"oo\".",
    "How wide your lips are spread, as for \"ee\".",
    "Your lower lip on your upper teeth, as for \"f\" and \"v\".",
    "How much you smile.",
    "Eyebrows: raised is positive, lowered (frowning) is negative.",
    "Head tilt: to your right is positive, to your left negative.",
];
pub const OPEN: usize = 0;
pub const JAW: usize = 1;
pub const ROUND: usize = 2;
pub const SPREAD: usize = 3;
pub const TEETH: usize = 4;
pub const SMILE: usize = 5;
pub const BROWS: usize = 6;
pub const TILT: usize = 7;
pub const NSRC: usize = SOURCES.len();

/// The settings, in the table's column order.
pub const DESTS: [&str; 10] = ["PITCH", "BREATH", "PRESSED", "BRIGHT", "BREATHY", "ROUGH", "FRY", "VIBRATO", "CREATURE 1", "CREATURE 2"];
pub const PITCH: usize = 0;
pub const BREATH: usize = 1;
pub const PRESSED: usize = 2;
pub const BRIGHT: usize = 3;
pub const BREATHY: usize = 4;
pub const ROUGH: usize = 5;
pub const FRY: usize = 6;
pub const VIBRATO: usize = 7;
pub const CREATURE_1: usize = 8;
pub const CREATURE_2: usize = 9;
pub const NDST: usize = DESTS.len();

/// What an amount of 1 does with the signal full, in the setting's own units.
pub const FULL: [f64; NDST] = [12.0, 20.0, 1.0, 1.0, 1.0, 1.0, 1.0, 120.0, 1.0, 1.0];
/// Those units, for the tooltips.
pub const UNITS: [&str; NDST] = ["semitones", "cmH2O", "", "", "", "", "", "cents", "", ""];

pub type Routes = [[f32; NDST]; NSRC];

/// What the face did before the table existed: head tilt bends the pitch by up to two
/// semitones, the eyebrows brighten and tighten the voice, the smile brightens it, and the
/// eyebrows and smile lean on the creature's first two free knobs.
pub fn defaults() -> Routes {
    let mut r = [[0.0; NDST]; NSRC];
    r[TILT][PITCH] = 2.0 / 12.0;
    r[BROWS][BRIGHT] = 0.35;
    r[BROWS][PRESSED] = 0.15;
    r[SMILE][BRIGHT] = 0.2;
    r[BROWS][CREATURE_1] = 0.3;
    r[SMILE][CREATURE_2] = 0.3;
    r
}

/// How much the face moves each setting, in its own units, for these signal values.
pub fn mods(routes: &Routes, sources: &[f64; NSRC]) -> [f64; NDST] {
    let mut out = [0.0; NDST];
    for (row, s) in routes.iter().zip(sources) {
        for (o, (amount, full)) in out.iter_mut().zip(row.iter().zip(FULL)) {
            *o += *amount as f64 * s * full;
        }
    }
    out
}

/// A cell's effect in words: "+2.0 semitones", "+35% of the knob".
pub fn describe(dest: usize, amount: f32) -> String {
    if UNITS[dest].is_empty() {
        format!("{:+.0}% of the knob's range", amount * 100.0)
    } else {
        let v = amount as f64 * FULL[dest];
        format!("{v:+.1} {}", UNITS[dest])
    }
}
