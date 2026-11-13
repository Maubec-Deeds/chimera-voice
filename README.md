# Chimera Voice

A CLAP instrument for Bitwig (Windows) built on [CHIMERA](https://github.com/jamesondh/chimera-vocal-synth),
Jameson Hodge's physical-modelling voice engine. Every sound comes from a simulated airway (lungs,
vocal folds, throat, mouth, lips, nose); there are no samples.

The engine is used unchanged, as a git submodule in `vendor/chimera-vocal-synth`. This project adds
the plugin around it: note handling, parameters and the editor.

## Playing it

It is one voice (monophonic).

- **Sing lyrics**: type words; each note sings the next syllable. Overlap two notes to sing them
  legato. A lone `-` in the lyrics holds the vowel onto the next note. A typed hyphen splits a word
  yourself (`ti-ger`). Commas, full stops and line breaks end a phrase. Click a syllable to start
  from it. English uses the CMU Pronouncing Dictionary; Korean (Hangul) and Russian also work.
- **Vowel pad**: notes sing the vowel under the pad's handle; drag it (or automate *Vowel
  Front-Back* / *Vowel Close-Open*) to move the mouth.
- **Body**: man, woman, child, or anywhere between. **Octave** shifts the keyboard.
- **Voice knobs**: Breath (lung pressure), Pressed, Bright, Breathy, Rough, Fry, Whisper.
- **Pitch knobs**: Vibrato depth and rate, Glide, Drift (human pitch wander), Bend range.
- Velocity sets how hard the voice breathes. Pitch bend, mod wheel (more vibrato) and the sustain
  pedal work. **Silence** stops everything.
- **Restart when play starts** sends the lyrics back to the first syllable each time the
  transport starts.
- **Eco** halves the processor use for a slightly duller voice.

Knobs: drag up and down, hold Shift for fine steps, double-click for the default.

## Processor use

The airway simulation is heavy: about 60% of one performance core of an i7-13700H at 48 kHz, 28%
with Eco on. Use one instance per singing part and bounce or freeze tracks when a project grows.

## Building

Needs Rust (`rustup`, stable, MSVC toolchain).

```
git submodule update --init
./install.ps1        # builds and copies "Chimera Voice.clap" to %LOCALAPPDATA%\Programs\Common\CLAP
cargo test --release # sings test phrases; WAVs land in target/renders
```

`target/release/chimera_voice_standalone.exe` runs the plugin on its own with the system's audio
and MIDI devices.

## Layout

- `src/performer.rs`: notes → engine commands (a port of the web app's live Sing controller, plus
  the vowel pad). No plugin code: the tests drive it directly.
- `src/lib.rs`: the plugin (parameters, MIDI, audio callback).
- `src/editor.rs`: the editor (egui).
- `src/lyrics.rs`: text → syllables, with the dictionary built in.
- `src/shared.rs`: what the audio thread, the editor and the background task share.

## Licences

This project is MIT-licensed (see `LICENSE`). It builds on:

- CHIMERA, © 2026 Jameson Hodge, MIT (`vendor/chimera-vocal-synth/LICENSE`).
- The CMU Pronouncing Dictionary (`vendor/chimera-vocal-synth/public/data/CMUDICT-LICENSE.txt`).
- AVHershey Simplex by Stewart C. Russell, CC0 / WTFPL.
- [NIH-plug](https://github.com/robbert-vdh/nih-plug), ISC, and [egui](https://github.com/emilk/egui), MIT / Apache-2.0.
