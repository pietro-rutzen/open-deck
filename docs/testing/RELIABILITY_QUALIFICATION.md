# Reliability qualification

Current status: local prototype; no production release qualification. Offline, synthetic tests and actual device tests are separate evidence.

## Automatic regressions

From the repository root:

```sh
npm test --prefix apps/compas-dj/frontend
npm run typecheck --prefix apps/compas-dj/frontend
CARGO_HOME="$PWD/.build-cache/cargo" cargo test --locked -p compas-audio --lib
CARGO_HOME="$PWD/.build-cache/cargo" cargo test --locked -p compas-dj --lib
```

The frontend job in CI runs the selection and marker regressions. The engine job runs ordinary mixer tests; the macOS desktop job explicitly tests the native app. A local Trash-permission test has previously failed in the restricted development environment. Report that exclusion explicitly if used; CI retains the complete test.

Library tests exercise stable selection through 10,000 refresh/reorder cycles; they do not generate physical MIDI or exercise the full WebView. Waveform tests check source-frame placement and shared colors; visual checks remain separate.

## Offline mixer stress

T027 adds generated in-memory fixtures and direct commands through the real bounded mixer command ring. It advances audio frames faster than wall time, with no audio device or external music. It must assert behavior, not just avoid a panic. The default test renders 60 seconds with two deck replacements each second. The explicit hour renders 172,800,000 frames and 7,200 deck replacements:

```sh
CARGO_HOME="$PWD/.build-cache/cargo" cargo test --locked -p compas-audio --lib offline_hour_load_cue_transport_session -- --ignored --nocapture
```

See the T027 task/handoff for results and outstanding review. Strict local audio Clippy currently fails on existing warnings; hosted CI is not yet qualified.

An offline hour means one hour of rendered sample time, not one hour under macOS scheduling, USB, audio-driver or UI load. It does not measure callback deadlines, audible dropouts, physical latency, disk decode memory or the app's upstream control queue.

## Physical release gate

- [ ] Record exact build revision, macOS/CPU, controller firmware, sample rate and buffer settings.
- [ ] Mix for at least 60 wall-clock minutes with two decks, track changes, cue toggles, loops, sampler, effects, library analysis and UI activity.
- [ ] Record starting/ending xrun, command pressure, cue starvation and recording-drop counters, and any audible artifacts.
- [ ] Test FLX4 master/headphones and external-master + FLX4-headphones separately.
- [ ] Repeat controller unplug/replug, device loss and sleep/wake with explicit recovery observations.
- [ ] Measure physical loopback latency with a documented setup; software MIDI receipt-to-callback delay is a separate metric.

A production claim requires a passing report for each supported configuration and resolution of release blockers. User-confirmed reconnect/browse/LOAD on 2026-11-04 is a useful single-case pass, not this entire gate.

## Next engineering gates

1. T028 bounds deck decode concurrency with one worker and one latest pending slot per deck. T029 caps actual deck PCM at 512 MiB and cancels superseded requests between source reads. Codec-internal allocation, analysis memory and interruption during blocking reads remain outside these bounds.
2. Bound the upstream engine-command channel, with explicit continuous-control coalescing and reliable transport/load admission. The bounded audio ring alone does not bound the whole pipeline.
3. Reconcile track/effect/recording state after stream failure; prove recovery and avoid silent stale UI state.
4. Complete callback/lifetime audit, hosted checks, dependency/license review, then signed/notarized release-candidate installation on a clean Mac.
