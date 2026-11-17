# Chimera Voice

A CLAP instrument for Bitwig (Windows) built on [CHIMERA](https://github.com/jamesondh/chimera-vocal-synth),
Jameson Hodge's physical-modelling voice engine. Every sound comes from a simulated airway (lungs,
vocal folds, throat, mouth, lips, nose); there are no samples.

The engine is used unchanged, as a git submodule in `vendor/chimera-vocal-synth`. This project adds
the plugin around it: note handling, parameters and the editor.

## Download and install

1. On the [Releases page](../../releases/latest), download `ChimeraVoice-<version>-windows-x64.zip`
   and unzip it.
2. Close Bitwig, then double-click **Install.cmd** in the unzipped folder. It puts the plugin where
   CLAP hosts look and the face tracker in your Start menu ("Chimera face tracker").
3. Open Bitwig, rescan plugins, and add **Chimera Voice** to an instrument track. Press F1 in its
   window for a quick guide.

The face tracker also needs Python 3.12 from [python.org](https://www.python.org/downloads/) the
first time it starts. To update, run a newer zip's Install.cmd; the face calibration is kept.
`READ ME FIRST.txt` in the zip says the same, plus how to uninstall. Nothing needs building.

## Playing it

It is one voice (monophonic), with four modes.

### Sing

Each note sings the next syllable of the lyrics. Overlap two notes to sing them legato. Click a
syllable to start from it. **Restart when play starts** sends the lyrics back to the first
syllable each time the transport starts.

Ways to get lyrics in:

- **Type**: click the box and type. While you type the plugin holds the keyboard (the box has a
  green edge); click anywhere outside the box to hand it back to the host, so the space bar starts
  playback again. Some hosts (Bitwig among them) keep the space bar even then: type `_` instead,
  which counts as a space everywhere in the lyrics (`hello_there`). English uses the CMU
  Pronouncing Dictionary; Korean (Hangul) and Russian also work.
- **Paste** takes the clipboard's text; **Load file** reads a text file.
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

A webcam watches your face and Chimera follows it: the mouth in detail, plus eyebrows, head tilt
and smile. A camera cannot see the tongue, so sounds made with it (t, d, s, l, k) cannot be read
from the face, and in Sing mode the words still come from the lyrics.

1. Turn on **FACE** in the top bar.
2. Start the tracker: Start menu > **Chimera face tracker** (after installing from the zip), or
   double-click `tools/facetrack/Start face tracker.cmd` in a copy of this project. The first run
   needs Python 3.12 (python.org) and sets itself up, which takes a few minutes and about 300 MB.
3. Calibrate once: press **Space** in the tracker's window and follow the five poses it asks for
   (rest, "aah", "oo", "ee", and "ff" with the lower lip under the upper teeth), pressing Space to
   capture each. This is saved for next time. The window shows which poses are done; **R**, **A**,
   **U**, **E**, **F** redo one. The jaw is read from how far the chin drops (so it moves apart from
   the lips once rest and "aah" are calibrated), and "lip on teeth" from how near the face is to
   the "ff" pose.
4. Play a key once: the face sings on the last key you played.

What the face does:

- **Vowel**: your jaw and lips drive the model's jaw and lips directly, and the pad only places
  the tongue. Parting your lips starts the note, and the wider you open the louder it sings; a
  mouth that snaps open attacks harder. Rounded lips sing "oo", spread lips "ee", and a smile
  moves the vowel forward. Shut your lips on a note and it hums "m"; a quick shut-and-open gives
  "b"; lower lip on the teeth gives "v" (or "f" with Whisper on). Lips kept shut for about half a
  second end the note.
- **Sing**: each time the mouth opens it sings the next syllable; opening again after closing
  part way sings the next one legato; shutting the mouth ends the note. A wider mouth sings louder.
- **Creature**: opening the mouth plays a held note, louder the wider it is. Eyebrows and smile
  lean on the creature's first two knobs that are not its pitch.
- **Everywhere**, through the face routing below: tilting your head bends the pitch by up to two
  semitones either way (nothing within about four degrees of upright); raised eyebrows brighten and
  tighten the voice, lowered ones darken it; a smile brightens it.

**Face routing.** The **MAP** button next to FACE opens a table: one row for each part of the face
(open, jaw, round, spread, lip on teeth, smile, eyebrows, head tilt) and one column for each
setting (pitch, Breath, Pressed, Bright, Breathy, Rough, Fry, Vibrato, and the creature's first two
free knobs). Each cell is how much that part of the face moves that setting: drag it up or down
(Shift for fine steps; below zero turns the setting the other way), click it to switch it off or
back on, double-click for its default. 100% moves a knob across its whole range (pitch: an octave;
vibrato: 120 cents). **DEFAULTS** brings back the routing described above, **ALL OFF** clears it.
Each row shows what that part of the face reads now. The table is saved with the project. The
mouth's own work (starting and ending notes, how loud, and in Vowel mode the vowel and the lips) is
always on and is not in the table.

While the face moves a knob, the knob shows it: a lighter arc from the knob's own value to the
value in use, with a dot at the end.

In the tracker's window, **1**, **2** and **3** switch eyebrows, head tilt and smile off and on,
**V** hides the video to save processor time, and **Q** quits. The meters show everything being
sent.

The dot on the FACE button is green while a face is tracked, amber when the tracker runs but sees
no face, blue when nothing is sending, and red when another Chimera already has face control
(only one can listen at a time).

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
- **Help**: the **?** button, or **F1** while the plugin's window is the active one, opens a
  one-page guide; **Full help** from there (or **Help** in the lyrics panel) is the whole manual.
  F1 or Esc closes it.
- **Window size**: drag the bottom-right corner. Everything scales with the window, and the size
  is saved with the project. (The host cannot resize this kind of window by its edges.)

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
cargo test --release # plays test phrases, creatures, drums and made-up face frames, and clicks
                     # through the editor without a window; WAVs land in target/renders
```

`target/release/chimera_voice_standalone.exe` runs the plugin on its own with the system's audio
and MIDI devices. For checking the layout, the environment variables `CHIMERA_UI_SCALE` (0.6 to
2.5) and `CHIMERA_UI_HELP` (`quick`, `full` or `routing`) open it at another size or on a help or
routing screen.

`./package.ps1` builds the release zip in `dist/`. Pushing a version tag (`git tag v0.4.0`,
`git push origin v0.4.0`) has GitHub build the same zip and attach it to that tag's release; it can
also be run by hand from the repository's Actions tab.

## Layout

- `src/performer.rs`: notes → engine commands for the four modes (Sing is a port of the web app's
  live Sing controller). No plugin code: the tests drive it directly.
- `src/lyrics.rs`: text → syllables (words, sound codes, IPA), with the dictionary built in.
- `src/lib.rs`: the plugin (parameters, MIDI, audio callback).
- `src/editor.rs`: the editor (egui), drawn for 960 x 640 on a layer scaled to the window; the
  help screens; keyboard handling.
- `src/face.rs`: the face tracker's packets, the listener, and how frames become notes, mouth
  shapes and the signals the routing uses.
- `src/routes.rs`: the face routing table and what it does to each setting.
- `src/shared.rs`: what the audio thread, the editor and the background task share.
- `tools/facetrack/`: the webcam tracker (Python, MediaPipe).
- `packaging/`, `package.ps1`: the release zip and its installer.
- `.github/workflows/release.yml`: builds the zip on GitHub for each version tag.

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
