# Saturday setup and rehearsal

Target: DDJ-FLX4 with local files. These procedures are physical acceptance gates, not claims that listening passed.

## Choose the output before loading decks

| Setup | Settings → Speakers | Headphones |
| --- | --- | --- |
| Speaker connected to FLX4 RCA MASTER | Explicitly select DDJ-FLX4 | FLX4, channels 3/4; same stream as master 1/2 |
| Speaker connected to Audient or another interface | Select that actual output device | FLX4, channels 3/4; separate output stream |

Use an explicit device choice for rehearsal. “System output” follows macOS and may select a different device. Switching speakers restarts playback and clears decks/sampler pads; choose first, then load tracks. Do not change audio devices during a live transition.

## One rehearsal for each setup

Start at low speaker/headphone volume. Use the actual cables and speaker for the set.

- [ ] Confirm the named speaker device in Settings; load two local tracks.
- [ ] Start A, exercise its channel fader and crossfader. Repeat B. Verify expected channel assignment and no unexpected silence.
- [ ] With A’s channel fader down, select its mixer CUE and turn headphone mix toward CUE. Hear A in headphones, not speakers. CUE stays lit after release. Repeat B.
- [ ] Turn headphone mix toward MASTER and verify the master mix; return to the preferred position.
- [ ] Use rim nudge, touch/scratch/release, Play/Cue, hot cue, loop in/out, loop exit and sync. Check release behavior as well as presses.
- [ ] Engage and disengage each effect and key shift. Load a different track and confirm no previous effect stays unexpectedly active.
- [ ] Play a loaded sample; stop/clear it and verify no unexpected repeat.
- [ ] Mix for at least the planned set duration with the actual library. Note dropouts or unexpected level changes. Check Audio health before/after; increasing headphone buffer shortages or discarded frames need investigation.
- [ ] If the speakers lag the headphones, raise Settings → Headphone delay until they line up. Leave it at 0 when they already match. This delays the headphones; it does not make the speakers earlier.
- [ ] Stop playback, switch to the other output setup, reload tracks and repeat the routing checks.

Keep a tested backup player and local copies of the set available. This build has not been qualified for unattended device recovery; an audio restart requires reloading/checking routes, and relaunching if state is inconsistent.

## Diagnostics interpretation

Settings → Audio health reports software MIDI receipt-to-callback delay for supported native controls. It excludes USB input, physical speaker/DAC delay, jog fast path and headphone controls. It is not comparable to a physical rekordbox latency measurement. Save the Diagnostics bundle if a problem occurs; it contains state, not audio.

Separate devices have independent clocks. Bounded buffering/resampling is present, but this is not seamless adaptive clock synchronization. Single-device FLX4 routing avoids that separate-clock path.

## Evidence to record

Build revision, macOS version, FLX4 firmware, actual devices, sample rate, buffer settings, cabling, duration, commands exercised, observed glitches and before/after counters. Leave boxes unchecked until heard/tested on hardware.
