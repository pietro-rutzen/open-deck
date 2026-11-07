# Open Deck

Open Deck is a macOS DJ app for a local, DRM-free party set. The normal layout is two decks. Four decks are available when you want them. It is meant to be direct to use: load a track, cue it, and mix. This checkout is a prototype. It does not claim a measured startup time, a measured end-to-end latency, or a complete controller certification.

The audio engine is MIT-licensed [Compás](https://github.com/sergiogallegos/compas), pinned in this tree at commit [`102d2d3b6fd049efcd670c5a11bcc304ee726612`](https://github.com/sergiogallegos/compas/commit/102d2d3b6fd049efcd670c5a11bcc304ee726612). Copyright (c) 2026 Sergio Gallegos. That license is [LICENSE](LICENSE). The window title is still `Open Deck — Compás prototype`. The title bar and status bar say Open Deck.

## Party use on a DDJ-FLX4

The owner reports playing a full party on a Pioneer DDJ-FLX4. That is owner-reported use, not an independent hardware test.

A later session lost hardware LOAD while drag and drop still worked. The exact failure was not reproduced. The owner then confirmed that unplugging and reconnecting the controller, browsing, and LOAD worked without restarting the app. Those reports are not an all-night soak, a firmware check, or a pass for every mapped control.

## What you can do

- Play, cue, tempo, sync, key lock, and key shift.
- Three-band EQ, filter, channel faders, and a crossfader.
- Loops, hot cues, and beat jump.
- Per-deck echo, flanger, reverb, and bit-crusher. These are Open Deck inserts, not another DJ app's effect set.
- A library of local files, with folders and crates, and recording of the master mix to WAV.
- Optional oscilloscope, stereo image, and spectrum on each deck. A live comparison against MiniMeters has not been run. Do not read the meters as matched to MiniMeters.

The folder scanner accepts `mp3`, `flac`, `wav`, `m4a`, `aac`, `ogg`, `oga`, `aiff`, and `aif`. Decode uses Symphonia. Generated fixtures confirm uncompressed PCM WAV, uncompressed PCM AIFF (`.aiff` and `.aif`), AAC-LC in MPEG-4 ADTS (`.aac` sync `0xFFF1`), and AAC-LC inside an MPEG-4 `.m4a`, each at the fixture's 44.1 kHz stereo layout. MP3, FLAC, and Ogg Vorbis stay enabled. MPEG-2 ADTS (sync `0xFFF9`, including the file macOS `afconvert` writes), ALAC, HE-AAC, USAC, and compressed AIFC are not covered. A listed extension is not a promise that every file of that type opens. Spotify mixing is not available. See [docs/SPOTIFY_FEASIBILITY.md](docs/SPOTIFY_FEASIBILITY.md).

## DDJ-FLX4

Connect the controller by USB. Pick the speaker output before launch. Open **Controller profiles**, choose **Pioneer DDJ-FLX4**, select the MIDI input, connect, and activate. Activate again after a restart.

The bundled profile includes play, cue, sync, tempo, trim, three-band EQ, filter, channel volume, crossfader, headphone cue, and jog scratch and bend, plus loops, hot cues, and beat jump. A hardware move applies immediately. Soft takeover is off, so start with the level down.

Jog wheels and channel faders are implemented. They are not unsupported. This repository also does not contain an independent physical sign-off for them. Headphones can use channels 3/4 on the same device as the master, or a separate output. The owner confirmed headphone cue on the FLX4 during setup. Clock recovery and every failure path are not fully qualified.

Pad FX holds this app's echo, flanger, reverb, and bit-crusher on that deck. Pads 1–4 use the panel parameters. Pads 5–8, the shifted Pad FX layer, and Pad FX2 use presets of those same inserts, and release restores the panel settings. Keyboard mode selects a hot cue, then plays it at +4, +5, +6, +7, ±0, +1, +2, and +3 semitones. That is not a stem mode, and it is not a rekordbox effect preset. Sampler play, one-shot retrigger, and stop cover eight shared slots. Sampler slots 9–16, sampler banks, browser loading, and Release FX are not implemented. Key shift keeps the semitone scale already in the app, which is not the manual's keyboard scale. None of this has a physical FLX4 test. The written control list is [docs/testing/FLX4_FUNCTIONALITY_AUDIT.md](docs/testing/FLX4_FUNCTIONALITY_AUDIT.md).

## Run it on this Mac

From the repository root:

- `./scripts/opendeck-dev.sh` starts a development session.
- `./scripts/opendeck-build.sh` builds a debug app and installs `/Applications/Open Deck.app`.
- `./scripts/opendeck-build.sh --build-only` builds without replacing the app you have open.

[pietro-rutzen/open-deck](https://github.com/pietro-rutzen/open-deck) has no GitHub release. The tag `v0.1.0` is the older upstream release [Compás DJ v0.1.0](https://github.com/sergiogallegos/compas/releases/tag/v0.1.0) (24 June 2026, 08:50 Europe/Helsinki). That is not this prototype. CI compiles the engine on Linux, Windows, and macOS and builds the macOS app. That is not a Windows or Linux product test.

Practice tracks: `samples/compas-test-120bpm.wav` and `samples/compas-test-128bpm.wav`. Regenerate them with `node scripts/make-test-audio.mjs`.

## Development

Read [AGENTS.md](AGENTS.md), [docs/PROJECT_PROGRESS.md](docs/PROJECT_PROGRESS.md), and [docs/AGENT_WORKFLOW.md](docs/AGENT_WORKFLOW.md). Source is `apps/compas-dj/frontend`, `apps/compas-dj/src-tauri`, and `crates/compas-*`. AI tools are for development only. The app does not include a model or a chat feature.
