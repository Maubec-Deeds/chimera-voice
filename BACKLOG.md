# Backlog

Ideas not scheduled. Newest first.

## Done in 0.4.0 (2026-11-17)

These were the findings and requests from the first Bitwig test of v0.3.0:

- `_` counts as a space in lyrics (Bitwig keeps the space bar).
- The tracker reads the jaw from the chin's drop and OPEN from the lip gap alone, so they no
  longer move as one.
- Lip on teeth has its own calibration pose ("ff") and is scored against it.
- Releases carry one zip (plugin, face tracker, Install.cmd, licences), built on GitHub for each
  version tag.
- Face routing table (MAP): any part of the face to any of ten settings, with an amount.
- Knobs show what the face adds as a lighter arc.

## Still open

- Whether other keys type normally in the lyric box in Bitwig (only the space bar is known to be
  taken).
- Face routing to the vowel pad, the jaw and the lips themselves (now fixed in the mouth's own
  behaviour), or to the next syllable in Sing mode.
- The tracker's thresholds for the new jaw and "ff" readings are tuned on made-up faces only;
  check them with a real face and adjust.
