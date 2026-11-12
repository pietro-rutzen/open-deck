# Open Deck

Open Deck is an open-source DJ app for macOS. You point it at a folder of music you own, plug in a controller, and mix. There is no account, no subscription, and no license tied to your hardware.

It started from a simple itch: DJ software tends to come bundled with a store, a cloud, or a login, and a controller often only unlocks the app it shipped with. Open Deck goes the other way. It plays local, DRM-free files, it keeps your library as plain folders on disk, and its first controller profile is the Pioneer DDJ-FLX4.

This is a working prototype. It is good enough to play a set on, and it is honest about what has and has not been measured. Where something is implemented but not yet verified on real hardware, this page says so.

## What it does today

**Decks and mixer.** Two decks by default, four when you want them. Play, cue, tempo, sync, key lock, and key shift. Three-band EQ, filter, channel faders, and a crossfader.

**Performance.** Loops, hot cues, beat jump, and a sampler with four banks of 16 slots. Each deck has echo, flanger, reverb, and bit-crusher inserts. These are Open Deck's own effects, not a copy of another app's effect set.

**Library.** Watched folders, crates, search, drag-to-reorder, and BPM and key analysis with a beatgrid you can nudge. The folder on disk is the library, so nothing is locked inside a database.

**Seeing the sound.** Scrolling waveforms on every deck, plus an optional oscilloscope, stereo image, and spectrum.

**Recording.** The master mix records to WAV.

**MP3 copies for older players.** Each watched folder can keep an `mp3/` folder beside your files, with a 320 kbps CBR copy of every track, tagged and named so an older CDJ lists them in your order. Copy that folder to a USB stick and you are done. Details are in [docs/MP3_MIRRORS.md](docs/MP3_MIRRORS.md).

## File formats

| Format | Status |
| --- | --- |
| MP3, FLAC, WAV, AIFF, Ogg Vorbis | Plays |
| AAC in `.m4a` (iTunes Store purchases) and `.aac` | Plays (AAC-LC) |
| Apple Lossless, HE-AAC, compressed AIFC | Not supported yet |
| Protected files (`.m4p`) and streaming services | Not supported |

Decoding is done by [Symphonia](https://github.com/pdeljanov/Symphonia). A listed extension is not a promise that every file of that type opens: unusual variants exist, and the ones known to fail are in the table. Spotify mixing is not available, and [docs/SPOTIFY_FEASIBILITY.md](docs/SPOTIFY_FEASIBILITY.md) explains why.

## DDJ-FLX4 coverage

The bundled profile was built from AlphaTheta's official MIDI message list for the FLX4, and it maps every control on that list.

| Area | What is mapped |
| --- | --- |
| Transport | Play, cue, sync, tempo fader, jog scratch and pitch bend |
| Mixer | Trim, three-band EQ, filter, channel faders, crossfader, master level |
| Monitoring | Headphone cue per channel, on outputs 3/4 of the controller or on a separate device |
| Browsing | Rotary browse and LOAD to either deck |
| Pads | Hot Cue, Pad FX 1 and 2, Beat Jump, Beat Loop, Sampler, Keyboard, Key Shift |
| Effects | Beat FX section and Release FX |
| LEDs | Play, cue, sync, pad modes, and track-loaded feedback |

A few things work differently from rekordbox, on purpose or by necessity:

- Pad FX and Beat FX drive Open Deck's four inserts. The pad presets are Open Deck's own, not rekordbox presets.
- Keyboard mode plays a hot cue at different pitches. It is not a stem mode.
- Stem pads and Smart CFX / Smart Fader are not implemented.
- Soft takeover is off, so a hardware move applies immediately. Start with the levels down.

**How well tested is it?** Everyday mixing has been used hands-on with a real FLX4: playback, the mixer, jog scratching, headphone cue, and the cue LEDs. The pad modes and effect layers are covered by automated tests against the MIDI list, and are still waiting on a full hands-on pass. Latency, long-session stability, and firmware differences have not been measured. The control-by-control list is in [docs/testing/FLX4_FUNCTIONALITY_AUDIT.md](docs/testing/FLX4_FUNCTIONALITY_AUDIT.md).

### Connecting the controller

1. Connect the FLX4 by USB and pick your speaker output before launching.
2. Open **Controller profiles** and choose **Pioneer DDJ-FLX4**.
3. Select the MIDI input, connect, and activate.

Activate the profile again after restarting the app.

Other controllers are on the list in [docs/CONTROLLERS.md](docs/CONTROLLERS.md). The FLX4 is the only profile that ships today.

## Running it

You need a Mac with Rust and Node.js installed. From the repository root:

```sh
./scripts/opendeck-dev.sh                 # development session with live reload
./scripts/opendeck-build.sh               # build and install /Applications/Open Deck.app
./scripts/opendeck-build.sh --build-only  # build without replacing the installed app
```

There is no packaged release yet, so building from source is the way in. Two practice tracks live in `samples/` (120 and 128 BPM) if you want something to load right away.

CI compiles the engine on Linux, Windows, and macOS, and builds the macOS app. Only macOS is a supported platform for the app itself.

## How it is built

- **Engine:** Rust. All audio timing lives here, on a real-time thread that never allocates, locks, or waits on the UI.
- **Shell:** [Tauri 2](https://tauri.app).
- **Interface:** React and TypeScript. The UI shows what the engine reports and never drives timing.
- **MP3 encoding:** [LAME](https://lame.sourceforge.io) 3.100, loaded as a separate, replaceable library.

The engine is [Compás](https://github.com/sergiogallegos/compas) by Sergio Gallegos, MIT-licensed, pinned at commit [`102d2d3`](https://github.com/sergiogallegos/compas/commit/102d2d3b6fd049efcd670c5a11bcc304ee726612). Open Deck builds on it, and the upstream [LICENSE](LICENSE) is kept as is. You will still see "Compás prototype" in the window title.

[ARCHITECTURE.md](ARCHITECTURE.md) has the longer tour, and [ROADMAP.md](ROADMAP.md) shows where things are headed.

## What is not here

- No AI features. The app contains no model, no chat, and no network calls to one. AI tools are used to help write the code, and that is where it stops.
- No telemetry and no account.
- No measured latency or startup figures yet. When those numbers exist, they will be published with the method used to get them.

## Contributing

Issues and pull requests are welcome. Start with [CONTRIBUTING.md](CONTRIBUTING.md). If you want to work on the audio path, [AGENTS.md](AGENTS.md) lists the real-time rules every change has to follow, and they apply to people as much as to coding agents. Hands-on reports from FLX4 owners are especially useful: name your firmware, macOS version, and what you pressed.

Source lives in `apps/compas-dj/frontend`, `apps/compas-dj/src-tauri`, and `crates/compas-*`.

## License

MIT. See [LICENSE](LICENSE). LAME is LGPL and ships as a separate library with its own license files.
