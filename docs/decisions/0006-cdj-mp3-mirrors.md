# ADR-0006 — LAME 320 CBR mirrors for older CDJs

- Status: accepted for the local prototype
- Date: 2026-11-12
- Author / accountable role: the session that implemented the MP3 mirror
- Decision authority: the maintainer approved the automatic MP3 converter for older CDJs that cannot read FLAC and the other files the library already decodes
- Related: `docs/MP3_MIRRORS.md`
- Supersedes / superseded by: none

## Context and constraints

Older Pioneer CDJs and XDJs that play MP3 and not FLAC are picky about the MP3 they will seek. The compatible file is MPEG-1 Layer III, 44.1 kHz, constant bitrate, with an ID3v2.3 tag. Variable bitrate, free-format frames, ID3v2.4, and sample rates other than 44.1 kHz are the combinations those players mis-seek or refuse. 320 kbps is the rate those units advertise and the rate rekordbox uses for a CDJ-oriented MP3.

The conversion is a background library job. It must not run on the audio callback, and a pulled USB stick must not be left with a half-written file.

LAME is LGPL-2.0. LAME's own LICENSE says a commercial program can use it if it is linked as a separate library (`libmp3lame`), the use is acknowledged, and any modifications are released under the LGPL. Static linking would require shipping the object files so a recipient can relink. That is a worse fit for an app bundle.

## Options

| Choice | Why it was not the default |
| --- | --- |
| LAME V0 or another VBR setting | Higher quality on paper, and the setting older CDJs seek incorrectly. |
| Fraunhofer | Not freely distributable. |
| shine or another small encoder | Lower quality, and no compatibility advantage over LAME CBR. |
| LAME `-q 2` | LAME's own header calls 2 "near-best" and the recommended speed/quality point. It is not more compatible. |
| Static `libmp3lame.a` | LGPL-legal only if the objects and a relink path ship with the app. Harder to replace. |

Nothing in current practice beats LAME 320 kbps CBR for this compatibility target. The quality switch does not change the frame format the CDJ sees.

## Decision and rationale

OpenDeck encodes with unmodified LAME 3.100, loaded at runtime with `dlopen` from `libmp3lame.0.dylib`. `scripts/build-lame-dylib.sh` downloads the upstream tarball and builds `--enable-shared --disable-static` for the `rustc` host architecture, so a Rosetta shell does not produce an x86_64 library that an arm64 app cannot load. One build fix is applied: `lame_init_old` is compiled as a file-local function on current clang but is still named in the 3.100 export list, so that single obsolete export is removed. The encoder sources are not modified. The dylib is copied into `Contents/Frameworks` and COPYING plus LICENSE into `Contents/Resources`, so the library can be replaced. Upstream is https://lame.sourceforge.io/ and https://www.mp3dev.org/.

The stream is 320 kbps CBR, 44.1 kHz, joint stereo, quality 0, VBR off, and the Xing/VBR tag rewrite disabled so the file is one forward write. Quality 0 is LAME's slowest and best psychoacoustic model (`lame.h`: "0=best (very slow)"). This job is offline and already yields while a deck is playing, so the extra CPU is acceptable. `-q 2` would be the right choice for a live encoder. It is not more compatible.

Sources that are not already 44.1 kHz are resampled with rubato's `SincFixedIn`: 256-tap sinc, Blackman-Harris window, 256× oversampling, cutoff 0.95, linear interpolation of the sinc table. That is rubato's high-quality windowed-sinc setting. Integer sources at 16 bits or below are rounded to the nearest 16-bit code. Wider integer sources and float sources get triangular (TPDF) dither of one 16-bit LSB before `lame_encode_buffer_interleaved`.

Tags are written as ID3v2.3 after the audio bytes and before the file is renamed into place: title, artist, album, genre, comment, a JPEG cover no larger than 800 px on the long edge, TBPM, and TKEY. A positive BPM and key already stored by library analysis replace the file's own BPM and key. The key name is preferred, then the Camelot code.

## Consequences and validation

The dylib is not in git. A build or test machine needs `scripts/build-lame-dylib.sh` first; `scripts/opendeck-build.sh` runs it. Replacing `Contents/Frameworks/libmp3lame.0.dylib` with another LAME 3.100 shared library is the LGPL substitution path. Reconsider this if a CDJ generation appears that prefers a different MP3 profile, or if LAME's license terms change. The operating rules for paths, the manifest, and the worker are in `docs/MP3_MIRRORS.md`.
