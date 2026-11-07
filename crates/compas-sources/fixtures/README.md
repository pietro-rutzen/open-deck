# Format fixtures

`tone.wav` is a 200 ms, 44.1 kHz, stereo, 16-bit PCM sine at 440 Hz. The left channel is louder than the right. It was synthesized by `generate.mjs` in this directory. It is not a copied recording.

On macOS, the same script converts that WAV with the system `afconvert` tool:

| File | Container | Data |
| --- | --- | --- |
| `tone.aiff`, `tone.aif` | AIFF | big-endian 16-bit PCM |
| `tone.m4a` | MPEG-4 audio (`m4af`) | AAC-LC |
| `tone.aac` | MPEG-4 AAC ADTS | AAC-LC |
| `tone-mpeg2.aac` | MPEG-2 AAC ADTS, unmodified `afconvert` output | AAC-LC |

`afconvert` is part of macOS. This script does not download an encoder or any audio file. Regenerate with:

```sh
node crates/compas-sources/fixtures/generate.mjs
```

The AAC files are lossy conversions of the same tone, so their decoded samples will not match the WAV sample for sample. `afconvert` writes MPEG-2 ADTS, whose frames start with `0xFFF9`. That file is `tone-mpeg2.aac`. Symphonia 0.5.5's ADTS reader only accepts the MPEG-4 sync word `0xFFF1`, so the generator also writes `tone.aac` by clearing that ID bit on each frame and leaving the AAC payload unchanged. The decode test expects `tone.aac` to decode and `tone-mpeg2.aac` to be rejected. The committed bytes are the fixtures the decode test checks.

These files are test data created for this repository. The repository license is MIT. AAC decode still uses the existing Symphonia AAC decoder; this fixture set does not add a new codec implementation. ALAC, HE-AAC, USAC, and compressed AIFC are not in this set.
