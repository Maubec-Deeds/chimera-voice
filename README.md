# Chimera Voice

A CLAP instrument for Bitwig (Windows) built on [CHIMERA](https://github.com/jamesondh/chimera-vocal-synth),
Jameson Hodge's physical-modelling voice engine. Every sound comes from a simulated airway (lungs,
vocal folds, throat, mouth, lips, nose); there are no samples.

The engine is used unchanged, as a git submodule in `vendor/chimera-vocal-synth`. This project adds
the plugin around it: note handling, parameters and the editor.

## Playing it

It is one voice (monophonic), with four modes.

### Sing

Each note sings the next syllable of the lyrics. Overlap two notes to sing them legato. Click a
syllable to start from it. **Restart when play starts** sends the lyrics back to the first
syllable each time the transport starts.

Ways to get lyrics in:

- **Type** in the box. English uses the CMU Pronouncing Dictionary; Korean (Hangul) and Russian
  also work.
- **Paste** takes the clipboard's text; **Load file** reads a text file. Both work even where the
  host keeps keystrokes from the plugin window.
- **Eight phrases** (the numbered buttons), saved with the project. Pick the one to sing by
  clicking, by automating *Phrase*, or by playing a keyswitch: C0 to G0 (MIDI notes 24 to 31)
  pick phrases 1 to 8 and start them from the top.
- **Sounds instead of spelling**, for invented words or exact pronunciation:
  - `{HH AH L OW / W ER L D}` uses the dictionary's sound codes; a `/` starts the next word.
    **Help** lists the codes.
  - `[həloʊ wɝld]` uses phonetic symbols (IPA); spaces separate words.
  - A sound the voice cannot make is named in red under the lyrics.

In any of them: a lone `-` holds the vowel onto the next note, a typed hyphen splits a word
yourself (`ti-ger`), and commas, full stops and line breaks end a phrase (the singer breathes).

### Vowel

Notes sing the vowel under the pad's handle; drag it (or automate *Vowel Front-Back* / *Vowel
Close-Open*) to move the mouth.

### Creature

A songbird, a Weddell seal, an alligator or an elephant, each with its own airway and knobs.
**The keys play** either a held note (the creature breathes at the key's pitch until you let go)
or one of its calls (each key plays the whole call at that pitch). The keyboard is moved by octaves
to sit in the creature's range. The eight *Creature Knob* parameters are whatever the chosen
creature's knobs are.

### Beatbox

The keys from C1 play the mouth's drum sounds, laid out like a General MIDI kit:

| Key | Sound | Key | Sound |
|---|---|---|---|
| C1 | Kick | F#1, G#1 | Hi-hat |
| C#1 | Rimshot | A#1 | Open hat |
| D1 | Snare | B1 | Crash |
| E1 | Inward snare | D#1, A1 | Lip pop |
| F1, G1 | Tongue click | any other key | Hum (a held bass note) |

Click a pad to hear it.

### Face control (experimental)

A webcam watches your mouth and Chimera follows it. Lips and jaw are all a camera can see, so
the words still come from the lyrics; the face supplies timing and mouth shape.

1. Turn on **FACE** in the top bar.
2. Double-click `tools/facetrack/Start face tracker.cmd`. The first run needs Python 3.12
   (python.org) and sets itself up, which takes a few minutes and about 300 MB.
3. Calibrate once in the tracker's window: press **C** with lips relaxed and shut, **O** on a wide
   "aah", **U** on an "oo", **E** on an "ee". This is saved for next time.

What the mouth does:

- **Sing**: each time the mouth opens it sings the next syllable; opening again after closing
  part way sings the next one legato; shutting the mouth ends the note.
- **Vowel**: the mouth moves the pad (a spread mouth is "ee", rounded is "oo", the opening is how
  open the vowel is) and opening it sounds the note.
- **Creature**: opening the mouth plays a held note.

The face sings on the last key you played. The dot on the FACE button is green while a face is
tracked, amber when the tracker runs but sees no face, blue when nothing is sending, and red
when another Chimera already has face control (only one can listen at a time).

Expect the voice to trail the mouth by about a tenth of a second. Mouthing silently while
listening to Chimera works better than speaking aloud with it, which sounds like an echo. A
dim room makes the camera slow down and adds delay, so light your face. The tracker sends to
127.0.0.1 only; nothing leaves the computer, and no images are saved.

### Everywhere

- **Body**: man, woman, child, or anywhere between. **Octave** shifts the keyboard.
- **Voice knobs**: Breath (lung pressure), Pressed, Bright, Breathy, Rough, Fry, Whisper.
- **Pitch knobs**: Vibrato depth and rate, Glide, Drift (human pitch wander), Bend range.
- Velocity sets how hard the voice breathes. Pitch bend, mod wheel (more vibrato) and the sustain
  pedal work. **Silence** stops everything.
- **Eco** halves the human voice's processor use for a slightly duller sound.

Knobs: drag up and down, hold Shift for fine steps, double-click for the default.

Changing the body, the creature, Eco, or going into or out of Creature mode builds a new airway in
the background; the voice is silent for about a third of a second while it does.

## Processor use

The airway simulation is heavy: about 60% of one performance core of an i7-13700H at 48 kHz, 28%
with Eco on. Use one instance per part and bounce or freeze tracks when a project grows.

## Building

Needs Rust (`rustup`, stable, MSVC toolchain).

```
git submodule update --init
./install.ps1        # builds and copies "Chimera Voice.clap" to %LOCALAPPDATA%\Programs\Common\CLAP
cargo test --release # plays test phrases, creatures, drums and made-up face frames; WAVs land in target/renders
```

`target/release/chimera_voice_standalone.exe` runs the plugin on its own with the system's audio
and MIDI devices.

## Layout

- `src/performer.rs`: notes → engine commands for the four modes (Sing is a port of the web app's
  live Sing controller). No plugin code: the tests drive it directly.
- `src/lyrics.rs`: text → syllables (words, sound codes, IPA), with the dictionary built in.
- `src/lib.rs`: the plugin (parameters, MIDI, audio callback).
- `src/editor.rs`: the editor (egui).
- `src/face.rs`: the face tracker's packets, the listener, and how frames become notes.
- `src/shared.rs`: what the audio thread, the editor and the background task share.
- `tools/facetrack/`: the webcam tracker (Python, MediaPipe).

## Licences

This project is MIT-licensed (see `LICENSE`). It builds on:

- CHIMERA, © 2026 Jameson Hodge, MIT (`vendor/chimera-vocal-synth/LICENSE`).
- The CMU Pronouncing Dictionary (`vendor/chimera-vocal-synth/public/data/CMUDICT-LICENSE.txt`).
- AVHershey Simplex by Stewart C. Russell, CC0 / WTFPL.
- [NIH-plug](https://github.com/robbert-vdh/nih-plug), ISC; [egui](https://github.com/emilk/egui),
  [copypasta](https://github.com/alacritty/copypasta) and [rfd](https://github.com/PolyMeilex/rfd),
  MIT / Apache-2.0.
- The face tracker uses [MediaPipe](https://github.com/google-ai-edge/mediapipe) and its face
  landmarker model (Apache-2.0, downloaded on first run) and [OpenCV](https://opencv.org) (Apache-2.0).
