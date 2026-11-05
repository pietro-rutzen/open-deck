# OpenDeck prototype

Open-source macOS DJ prototype based on MIT-licensed Compás commit `102d2d3b6fd049efcd670c5a11bcc304ee726612`. Original copyright and LICENSE retained. UI still contains upstream branding.

## Build and run

From repository root, run `./scripts/opendeck-dev.sh` for development or `./scripts/opendeck-build.sh` to build and install the local debug app. After verifying the installed bundle, the script quits any running Open Deck and opens the copy in Applications. Use `./scripts/opendeck-build.sh --build-only` to build while keeping your current session running. Dependencies use project-local caches. First native build takes several minutes.

Generated test tracks: `samples/compas-test-120bpm.wav` and `samples/compas-test-128bpm.wav`. Regenerate with `node scripts/make-test-audio.mjs`.

## Controller setup

Connect FLX4 by USB. Choose the master output in macOS Sound settings before launching. Open top-bar **Controller profiles (mapping & learn)**; select **Pioneer DDJ-FLX4**, choose MIDI input, **Connect**, then **Activate**. Repeat activation after restart. Keep the separate legacy MIDI Mapping panel empty.

Mapping covers play/cue/sync, gain/EQ/filter/tempo and crossfader. Hardware validation is pending. Jog wheels, channel faders and built-in FLX4 headphone channel routing are not ready. Soft takeover is disabled: hardware movement applies immediately, so start output volume low. Cue monitoring currently uses a separate output-device path.

Spotify mixing needs partner access unavailable to this project; see [feasibility](docs/SPOTIFY_FEASIBILITY.md). Use local files or generated samples.

## Development workflow

Read [AGENTS.md](AGENTS.md), [progress](docs/PROJECT_PROGRESS.md) and [workflow](docs/AGENT_WORKFLOW.md). AI coding agents develop OpenDeck; AI features are excluded from the DJ product. Actual source paths: `apps/compas-dj/frontend`, `apps/compas-dj/src-tauri`, `crates/compas-*`. Original conflicting docs are preserved in `docs/upstream-baseline/`.
