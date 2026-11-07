# FLX4 functionality audit — T019

This is an inventory, not a party-ready certification. User confirmed headphone PFL and latched CUE lights on T017, plus earlier playback/mixer/scratch/meters. T018 crossfader/master/rim-nudge and all T019 changes have offline regression evidence only; every new or re-routed control below still needs a physical FLX4 test (NOT RUN). No blanket controller compatibility claim.

Sources: Pioneer FLX4 MIDI E1 (search-indexed official table; direct PDF denied403), and Mixxx project FLX4 mapping/driver retrieved2026-10-26:
- https://www.pioneerdj.com/-/media/pioneerdj/software-info/controller/ddj-flx4/ddj-flx4_midi_message_list_e1.pdf
- https://github.com/mixxxdj/mixxx/blob/main/res/controllers/Pioneer-DDJ-FLX4.midi.xml
- https://github.com/mixxxdj/mixxx/blob/main/res/controllers/Pioneer-DDJ-FLX4-script.js

Address inventory below covers every control entry in that mapping, including shifted combinations. It does not establish proprietary rekordbox DSP parity, analog microphone/USB-mix hardware behavior, firmware compatibility or Bluetooth support. Those remain unverified.

## Important remaining gaps

Not implemented: Release FX, stem pads, and Smart CFX/Smart Fader. Release FX is a rekordbox host effect. The manual does not give a DSP formula, so it stays an open product choice and is not implemented. Pad FX uses Open Deck's echo, flanger, reverb, and bit-crusher. Pads 1–4 keep the panel parameters. Pads 5–8, the shifted layer, and Pad FX2 are presets of those same inserts, documented in the pad-mode matrix. That assignment is not a rekordbox preset, and it has no physical test. Keyboard mode plays a selected hot cue at the manual's default semitones. It is not a stem mode. SHIFT+PLAY censor, SHIFT+LOOP IN/OUT boundary adjust via jog, SHIFT beat-jump size ×16, tempo-range cycling (SHIFT+SYNC), deck SHIFT buttons, the cue-point LED, track-loaded LEDs and beat-loop shift-layer LEDs are implemented below; every one of them still needs a physical FLX4 test (NOT RUN). Beat FX drives OpenDeck's per-deck echo/flanger/reverb/crusher inserts, not rekordbox's Beat FX DSP or master-channel FX position. Sampler mode has four banks of 16 slots. Deck 1 plays slots 1–8 of the active bank and deck 2 plays slots 9–16. The bank is software state; the MIDI list has no bank message. SHIFT+pad stops a playing slot and, when the slot is idle, loads the browser-selected track through the existing decode path. A missing selection or a failed decode leaves the slot. An empty unshifted pad does nothing. That load replaces an idle slot. It is not a separate eject, and it does not copy rekordbox's refusal to load into an occupied slot. Hot cues follow OpenDeck set/seek semantics rather than rekordbox hold-to-preview. The second cue/play indicator address `90/91 47` stays dark because the same bytes are listed for both indicators. Unsupported controls must not be called working.

Hot cue, beat loop, loop in/out, beat jump, quantize, sync, sampler trigger/stop, key shift, Beat FX, Pad FX, and keyboard are applied on the controller thread at the press. Engine command acknowledgements are still absent. Physical timing against a reference player is NOT RUN.

## Pad-mode matrix

Checked against the AlphaTheta DDJ-FLX4 MIDI message list E1 and the English hardware manual DRI1804A on 7 Nov 2026. This is the software behavior. No physical FLX4 session has been run, so every row stays hardware-unverified.

Mode buttons are Note On channel `90` (deck 1) and `91` (deck 2). Press is `7F`, release is `00`, and the same note is the LED. One mode LED stays lit per deck. Performance pads are channel `97` for deck 1 and `99` for deck 2. SHIFT pads are channel `98` and `9A`. Note Off on those channels is normalized to a zero-velocity Note On before the mapping runs. A mapped toggle treats MIDI 64–127 as pressed and 0–63 as released.

| Mode | Button | Pad notes | What the software does |
| --- | --- | --- | --- |
| Hot Cue | `1B` | `00`–`07`, SHIFT deletes the same notes | Existing set/seek and delete. An empty pad stores the playhead heard at the press. A filled pad seeks there, quantized to the grid when quantize is on. This is not rekordbox hold-to-preview. |
| Keyboard | SHIFT+Hot Cue `69` | `40`–`47`, SHIFT `40`–`47` | See Keyboard below. Entering, leaving, or pressing `69` again clears the selection and restores a pitch a pad was holding. |
| Pad FX1 | `1E` | `10`–`17`, SHIFT `10`–`17` | Slots 0–7, and SHIFT slots 8–15. Leaving this mode releases that deck's held inserts. |
| Pad FX2 | SHIFT+Pad FX1 `6B` | `50`–`57`, SHIFT `50`–`57` | Slots 16–23, and SHIFT slots 24–31. Leaving this mode releases that deck's held inserts. |
| Beat Jump | `20` | `20`–`27` | −1, +1, −2, +2, −4, +4, −8, +8 beats, times the deck scale. Matches the manual's default sizes. |
| Beat Loop | SHIFT+Beat Jump `6D` | `60`–`67` | 1/4, 1/2, 1, 2, 4, 8, 16, 32 beats. The loop stays after release. The same pad cancels it. |
| Sampler | `22` | `30`–`37`, SHIFT stops or loads the same notes | Four banks of 16. See Sampler below. |
| Key Shift | SHIFT+Sampler `6F` | `70`–`77`, SHIFT `70`–`77` | Manual keyboard scale. SHIFT pads 7 and 8 move it. See Key shift below. |

SHIFT beat-jump pads 7 and 8 (`26`/`27` on the shift channel) multiply that deck's jump size by 1/16 or 16. Those addresses are also the track-loaded LEDs.

Leaving Pad FX1 or Pad FX2, deactivating the profile, or changing the output port releases held Pad FX inserts and restores the parameters saved at the first hold of that effect. Re-pressing the same Pad FX mode does not release. The SHIFT button itself does not release a held pad. A later pad of the same effect becomes the owner, so a following release of the earlier pad does not restore the saved parameters and does not leave a second insert on. That release is accepted as Note Off or as a Note On with velocity 0–63. Whether the hardware emits the original note-off when SHIFT is pressed in the middle of a hold was not tested.

Deck 2 uses the same note numbers on `91`, `99`, and `9A`.

### Pad FX presets

The manual's Effect A–AF letters are host assignments, not a fixed DSP list. Open Deck uses the four existing inserts: echo, flanger, reverb, and bit-crusher. There is no Release FX. Holding a pad turns its insert on. Releasing the pad that owns the insert turns it off, unless Beat FX had latched it, in which case the latch stays and the saved panel parameters return.

Pads 1–4 keep the panel parameters and hold the insert only while the pad is down. Release clears it. If that insert is already on, those four pads do not take ownership, so releasing them does not clear a Beat FX latch. Pads 5–8 and every shifted and Pad FX2 pad are presets. The first hold of an effect saves the panel parameters. A later preset of the same effect keeps that saved copy. Releasing a pad that does not own the insert does nothing. The last preset of an effect wins. `fx.next` still moves only latched effects.

Echo time is the preset's beat count times the deck's beat length (the default beat is 0.5 s, so 1 beat is 0.5 s). Echo feedback is `0.15 + depth × 0.7`. Echo mix is `depth × 0.5` while the insert is on. Flanger uses the same beat clock. Depth, size, mix, crush, and downsample stay inside 0–1.

| Slots | Layer | Notes | Pad order |
| --- | --- | --- | --- |
| 0–3 | Pad FX1 pads 1–4 | `10`–`13` | Panel echo, flanger, reverb, crusher |
| 4–7 | Pad FX1 pads 5–8 | `14`–`17` | Echo 1 beat / depth 0.75; flanger 1 / 0.8; reverb size 0.9 mix 0.55; crusher crush 0.85 down 0.6 |
| 8–15 | Pad FX1 + SHIFT | shift `10`–`17` | Echo 0.25 / 0.4; flanger 8 / 0.45; reverb 0.35 / 0.25; crusher 0.25 / 0.15; echo 2 / 0.65; flanger 2 / 0.7; reverb 0.75 / 0.7; crusher 0.7 / 0.85 |
| 16–23 | Pad FX2 | `50`–`57` | Echo 4 / 0.55; flanger 0.5 / 0.5; reverb 0.5 / 0.45; crusher 0.6 / 0.4; echo 1/8 / 0.8; flanger 16 / 0.35; reverb 1.0 / 0.4; crusher 0.95 / 0.2 |
| 24–31 | Pad FX2 + SHIFT | shift `50`–`57` | Echo 0.5 / 0.9; flanger 0.25 / 0.9; reverb 0.2 / 0.8; crusher 0.4 / 0.95; echo 8 / 0.35; flanger 6 / 0.55; reverb 0.85 / 0.2; crusher 0.15 / 0.5 |

Controls are `deck.0.pad_fx.0`–`31` and `deck.1.pad_fx.0`–`31`.

### Keyboard

This is pitched hot-cue playback, not stems. The old "Toggle Stem" label in this audit was wrong.

The first press selects a hot cue. An empty slot stores the heard playhead and mirrors `deck.N.hotcue.slot`. Later presses seek that cue, start playback, and set the pitch. A repeat Note On while that same pad is already held does nothing. Releasing a pad restores the pitch from before the hold only when no keyboard pad on that deck remains down. Playback keeps running after release. The manual does not define note-off; momentary pitch is an Open Deck choice. Loading a track clears the selection, and the load path sets pitch back to 0.

Default semitones, then plus the transpose, clamped to ±12: pad 1 +4, pad 2 +5, pad 3 +6, pad 4 +7, pad 5 ±0, pad 6 +1, pad 7 +2, pad 8 +3.

SHIFT pads 1–6 delete the selected hot cue. If nothing is selected, they delete that pad's hot cue. The UI mirror is `deck.N.clear_hotcue.slot`. SHIFT pad 7 raises the transpose by one semitone. SHIFT pad 8 lowers it by one. A pad that is already held updates immediately.

Controls are `deck.N.keyboard.0`–`7` and `deck.N.keyboard_shift.0`–`7`.

### Sampler

Four banks of 16 empty slots. Empty slots do not allocate PCM. The MIDI list has no bank message; the active bank is software state, cycled from the sampler panel. Deck 1 pads are slots 1–8 of that bank (`sampler.0`–`7`, then 16 per bank). Deck 2 pads are slots 9–16 (`sampler.8`–`15` on bank 0). Press plays the slot. A second press restarts a one-shot from the beginning. A looped slot still toggles. An empty unshifted pad does nothing. Note Off, a zero-velocity Note On, and velocity 1–63 do not play, stop, or load.

SHIFT+pad while a voice of that slot is playing stops it. SHIFT+pad while the slot is idle sends the selected library row through the existing non-realtime decode path and replaces the slot only after a successful decode. Nothing selected, a selected folder, or a failed decode leaves the previous sample and reports the existing library error. The stop press does not also load. This replaces an idle slot. It is not a distinct eject, and it does not copy rekordbox's optional refusal to load into an occupied slot.

The on-screen grid stays eight pads, deck 1's half of the active bank, plus a bank chip. Deck 2 slots are loaded from the controller and are not extra buttons. Pads light when a slot in the active bank is loaded. The bank chip has not been checked in a window. Physical FLX4 test pending.

### Key shift

The Mac/Windows manual (DRI1804A, page 65) sets these pads to the keyboard scale: +4, +5, +6, +7, ±0, +1, +2, +3 semitones. The sounding pitch is that semitone plus the current transpose, clamped to ±12. SHIFT pad 7 raises the transpose and the sounding pitch by one semitone. SHIFT pad 8 lowers them. SHIFT pads 1–6 are not assigned by that manual, so a press is consumed and does not change pitch. The MIDI message list's older absolute and relative labels are not what these pads do.

The lit pad is the last unshifted pad pressed. A transpose makes matching the pitch to one pad ambiguous, and rekordbox's on-screen scale display is not cloned. A note-off does not change the pitch. Loading a track clears the transpose and the lit pad, and the load path sets pitch back to 0. Setting the pitch from anywhere else clears the lit pad and leaves the transpose.

### Still absent

Smart CFX, Smart Fader, Release FX, and stem pads. The manual says a Release FX is assigned to a pad in advance, and pressing it while another Pad FX is held turns that effect off and adds the Release FX. SHIFT+Beat FX ON/OFF does the same for Beat FX. The algorithm is a rekordbox host effect, not a DSP formula, so it is not implemented. A mode button is lit only where the behavior above exists.

## Changed behavior (T019)

- Native control path: channel fader, trim, EQ low/mid/high, filter, tempo, PLAY, CUE and SHIFT+CUE (track start) are resolved in the controller thread and sent straight to the engine command queue. The UI only mirrors them through a coalesced `controller:batch` event (16 ms), so dropped or slow UI frames can no longer delay or drop these controls. EQ band state is shared between UI edits and controller edits so turning one knob never resets another band.
- UI traffic: profile-bound MIDI is no longer republished as raw `midi:cc`/`midi:note` events (raw input is only forwarded while a learn panel is open or for unbound messages), continuous UI-owned controls are coalesced per 16 ms frame, LED feedback only crosses IPC when a value changes, and master meter/engine-load events re-render only the title bar instead of the whole app. The library no longer re-renders on every telemetry tick.
- Timing: hot cue set, loop in/out and beat loop use the playhead heard at the moment of the press (playhead minus output latency); beat jump and quick jump use the press-time playhead. Previously they used a 30 Hz UI sample.
- PLAY LED is engine-owned: lit while playing, blinking at 500 ms while a loaded deck is paused, dark when empty.
- LOOP IN held for 500 ms starts a 4-beat loop at the press position.
- BEAT SYNC long press sets the deck as sync leader; SHIFT+RELOOP/EXIT is reloop-and-stop.
- Key shift pads use the manual's keyboard scale (+4, +5, +6, +7, ±0, +1, +2, +3). SHIFT pads 7 and 8 move that scale and the sounding pitch by one semitone, clamped to ±12. SHIFT pads 1–6 do not change pitch. The lit pad is the last unshifted pad.
- Sampler pads play four banks of 16 slots. Deck 1 is slots 1–8 of the active bank and deck 2 is slots 9–16. A second press restarts a one-shot. SHIFT stops a playing slot and loads the selected browser track when the slot is idle. Pads light when a slot is loaded. Sampler and key-shift pad-mode buttons light their mode LED. Physical test pending.
- Beat FX: FX SELECT cycles echo → flanger → reverb → crusher (SHIFT reverses, an active effect carries over), BEAT ◀/▶ steps echo/flanger beat length, CH SELECT picks deck 1 or 2, LEVEL/DEPTH sets the selected effect amount, ON/OFF toggles it with LED feedback, SHIFT+ON/OFF turns every insert off. The status bar shows the current Beat FX selection.
- BROWSE press toggles browsing between the track list and the sources tree (Local Library and folders); SHIFT+BROWSE rotate zooms the waveform.
- SHIFT+PLAY censors: the deck plays backward while the button is held, then returns to the position playback would have reached. A paused deck returns to the same point and stays paused.
- SHIFT+LOOP IN or SHIFT+LOOP OUT, held, turns the jog wheel into a loop-boundary trim (about 10 ms of audio per platter count). The wheel does not scratch while that trim is held.
- SHIFT beat-jump pads 7 and 8 scale that deck's beat-jump size by 1/16 or 16, clamped to that span. The on-screen jump chips show the scaled size.
- SHIFT+SYNC cycles the tempo-fader span ±8% → ±6% → ±10% → ±16% → ±100% → ±8%. The fader stays ±8% until that button is pressed. The deck label shows the active span.
- Deck SHIFT buttons light while held. The CUE button stays lit when the play-head is parked on the cue point or previewing from it. Beat-loop shift-layer pads mirror the active beat-loop length. Track-loaded pads `98/9A 26` and `98/9A 27` light while that deck has a track.

## Changed behavior (T018)

- Rim jog now uses a normal-playback tempo bend, preserves keylock and switches phase sync to tempo-only, keeping manual phase adjustment; fresh SYNC engagement restores phase sync.
- Crossfader applies natively, including14-bit resolution; speaker-master regression changes while PFL stays constant by design. No confirmed hardware crossfader failure cause yet; previous listening context was PFL.
- Added loop IN/OUT/reloop/scale, shifted beatjump, hotcue set/seek/delete, beatloop/beatjump pad layers, browse and stopped-deck load.
- Native status query/keep-alive from Mixxx driver, retained LED desired-state restoration, transport LED from engine telemetry, loop/hotcue/pad state feedback. No decorative lights to imply unimplemented features.
- Original saved built-in profile snapshots receive missing non-conflicting addresses. Custom address mappings and scripts are preserved.

| Physical function | MIDI status/data1 | OpenDeck target | Evidence/status |
|---|---|---|---|
| BROWSE - rotate - Scroll tracklist/tree view | `B6 40` | Library browse | Implemented; physical test pending |
| BROWSE +SHIFT - Zoom waveform | `B6 64` | waveform.zoom (relative) | Implemented; physical test pending |
| BROWSE - press - Move cursor between track list and tree view | `96 41` | library.focus (track list ↔ sources) | Implemented; physical test pending |
| BROWSE +SHIFT - press - Move cursor between track list and tree view | `96 42` | library.focus (track list ↔ sources) | Implemented; physical test pending |
| LOAD (DECK1) - press - Load a Track into Deck 1 | `96 46` | Library load (stopped deck only) | Implemented; physical test pending |
| LOAD (DECK2) - press - Load a Track into Deck 2 | `96 47` | Library load (stopped deck only) | Implemented; physical test pending |
| Shift (DECK1) | `90 3F` | Lights while held | Implemented; physical test pending |
| Shift (DECK2) | `91 3F` | Lights while held | Implemented; physical test pending |
| PLAY/PAUSE (DECK1) - press - Play/Pause | `90 0B` | deck.0.play (native) | Native engine path; physical test pending |
| PLAY/PAUSE +SHIFT (DECK1) - press - Reverse playback in Slip Mode while held (Censor) | `90 0E` | deck.0.censor | Implemented; physical test pending |
| PLAY/PAUSE (DECK2) - press - Play/Pause | `91 0B` | deck.1.play (native) | Native engine path; physical test pending |
| PLAY/PAUSE +SHIFT (DECK2) - press - Reverse playback in Slip Mode while held (Censor) | `91 0E` | deck.1.censor | Implemented; physical test pending |
| CUE (DECK1) - press - Set/Call Cue, Back Cue | `90 0C` | deck.0.cue (native) | Native engine path; physical test pending |
| CUE +SHIFT (DECK1) - press - Jump to track start | `90 48` | deck.0.start (native) | Native engine path; physical test pending |
| CUE (DECK2) - press - Set/Call Cue, Back Cue | `91 0C` | deck.1.cue (native) | Native engine path; physical test pending |
| CUE +SHIFT (DECK2) - press - Jump to track start | `91 48` | deck.1.start (native) | Native engine path; physical test pending |
| JOG DIAL PLATTER Vinyl mode On (DECK1) - rotate - Scratch | `B0 22` | Native jog | Scratch/bend regression tests; hardware feel pending |
| JOG DIAL PLATTER Vinyl mode Off (DECK1) - rotate - Pitch bend | `B0 23` | Native jog | Scratch/bend regression tests; hardware feel pending |
| JOG DIAL PLATTER +SHIFT (DECK1) - rotate - Search (Fast Pitch bend) | `B0 29` | Native jog | Scratch/bend regression tests; hardware feel pending |
| JOG DIAL PLATTER (DECK1) - touch - enable (on touch) / disable (on release) Scratching/Pitch bend | `90 36` | Native jog touch | Mapped; physical retest pending |
| JOG DIAL PLATTER +SHIFT (DECK1) - touch - enable (on touch) / disable (on release) highspeed Pitch bend | `90 67` | Native jog touch | Mapped; physical retest pending |
| JOG DIAL SIDE (DECK1) - rotate - Pitch bend | `B0 21` | Native jog | Scratch/bend regression tests; hardware feel pending |
| JOG DIAL PLATTER Vinyl mode On (DECK2) - rotate - Scratch | `B1 22` | Native jog | Scratch/bend regression tests; hardware feel pending |
| JOG DIAL PLATTER Vinyl mode Off (DECK2) - rotate - Pitch bend | `B1 23` | Native jog | Scratch/bend regression tests; hardware feel pending |
| JOG DIAL PLATTER +SHIFT (DECK2) - rotate - Search (Fast Pitch bend) | `B1 29` | Native jog | Scratch/bend regression tests; hardware feel pending |
| JOG DIAL PLATTER (DECK2) - touch - enable (on touch) / disable (on release) Scratching/Pitch bend | `91 36` | Native jog touch | Mapped; physical retest pending |
| JOG DIAL PLATTER +SHIFT (DECK2) - touch - enable (on touch) / disable (on release) highspeed Pitch bend | `91 67` | Native jog touch | Mapped; physical retest pending |
| JOG DIAL SIDE (DECK2) - rotate - Pitch bend | `B1 21` | Native jog | Scratch/bend regression tests; hardware feel pending |
| TEMPO (DECK1) - fader - Tempo control LSB | `B0 20` | deck.0.tempo LSB | 14-bit decoding tested |
| TEMPO (DECK1) - fader - Tempo control MSB | `B0 00` | deck.0.tempo (native) | Native engine path; physical test pending |
| TEMPO (DECK2) - fader - Tempo control LSB | `B1 20` | deck.1.tempo LSB | 14-bit decoding tested |
| TEMPO (DECK2) - fader - Tempo control MSB | `B1 00` | deck.1.tempo (native) | Native engine path; physical test pending |
| BEAT SYNC (DECK1) - press - Beat Sync to Master deck | `90 58` | deck.0.sync | Mapped; physical test pending |
| BEAT SYNC LONG PRESS (DECK1) - press - Set as Master deck | `90 5C` | deck.0.sync_leader | Mapped; physical test pending |
| BEAT SYNC +SHIFT (DECK1) - press - change Tempo range | `90 60` | deck.0 tempo span ±8/6/10/16/100% | Implemented; physical test pending |
| BEAT SYNC (DECK2) - press - Beat Sync to Master deck | `91 58` | deck.1.sync | Mapped; physical test pending |
| BEAT SYNC LONG PRESS (DECK2) - press - Set as Master deck | `91 5C` | deck.1.sync_leader | Mapped; physical test pending |
| BEAT SYNC +SHIFT (DECK2) - press - change Tempo range | `91 60` | deck.1 tempo span ±8/6/10/16/100% | Implemented; physical test pending |
| LOOP IN/4 BEAT (DECK1) - press - Set loop in | `90 10` | deck.0.loop.in; hold 500 ms = 4-beat loop | Implemented; physical test pending |
| LOOP IN/4 BEAT (DECK2) - press - Set loop in | `91 10` | deck.1.loop.in; hold 500 ms = 4-beat loop | Implemented; physical test pending |
| LOOP OUT (DECK1) - press - Set loop out | `90 11` | deck.0.loop.out | Mapped; physical test pending |
| LOOP OUT (DECK2) - press - Set loop out | `91 11` | deck.1.loop.out | Mapped; physical test pending |
| RELOOP/EXIT (DECK1) - press - (loop off) Reloop, (loop on) Loop exit | `90 4D` | deck.0.loop.toggle | Mapped; physical test pending |
| RELOOP/EXIT (DECK2) - press - (loop off) Reloop, (loop on) Loop exit | `91 4D` | deck.1.loop.toggle | Mapped; physical test pending |
| RELOOP/EXIT +SHIFT (DECK1) - press - Reloop and stop | `90 50` | deck.0.loop.reloop_stop | Mapped; physical test pending |
| RELOOP/EXIT +SHIFT (DECK2) - press - Reloop and stop | `91 50` | deck.1.loop.reloop_stop | Mapped; physical test pending |
| SHIFT + LOOP IN (DECK1) - Loop in adjust (using jog wheel) | `90 4C` | Jog trims deck.0 loop in | Implemented; physical test pending |
| SHIFT + LOOP IN (DECK2) - Loop in adjust (using jog wheel) | `91 4C` | Jog trims deck.1 loop in | Implemented; physical test pending |
| SHIFT + LOOP OUT (DECK1) - Loop out adjust (using jog wheel) | `90 4E` | Jog trims deck.0 loop out | Implemented; physical test pending |
| SHIFT + LOOP OUT (DECK2) - Loop out adjust (using jog wheel) | `91 4E` | Jog trims deck.1 loop out | Implemented; physical test pending |
| CUE/LOOP CALL LEFT (DECK1) - press - half active loop | `90 51` | deck.0.loop.half | Mapped; physical test pending |
| CUE/LOOP CALL LEFT + SHIFT (DECK1) - press - quick jump back | `90 3E` | deck.0.jump.back | Mapped; physical test pending |
| CUE/LOOP CALL LEFT (DECK2) - press - half active loop | `91 51` | deck.1.loop.half | Mapped; physical test pending |
| CUE/LOOP CALL LEFT + SHIFT (DECK2) - press - quick jump back | `91 3E` | deck.1.jump.back | Mapped; physical test pending |
| CUE/LOOP CALL LEFT (DECK1) - press - double active loop | `90 53` | deck.0.loop.double | Mapped; physical test pending |
| CUE/LOOP CALL RIGHT + SHIFT (DECK1) - press - quick jump forwards | `90 3D` | deck.0.jump.forward | Mapped; physical test pending |
| CUE/LOOP CALL LEFT (DECK2) - press - double active loop | `91 53` | deck.1.loop.double | Mapped; physical test pending |
| CUE/LOOP CALL RIGHT + SHIFT (DECK2) - press - quick jump forwards | `91 3D` | deck.1.jump.forward | Mapped; physical test pending |
| CROSSFADER - slider | `B6 1F` | mixer.crossfader | Mapped; physical test pending |
| CROSSFADER - slider | `B6 3F` | mixer.crossfader LSB | 14-bit decoding tested |
| CHANNELFADER - slider | `B0 33` | deck.0.volume LSB | 14-bit decoding tested |
| CHANNELFADER - slider | `B0 13` | deck.0.volume (native) | Native engine path; physical test pending |
| CHANNELFADER - slider | `B1 33` | deck.1.volume LSB | 14-bit decoding tested |
| CHANNELFADER - slider | `B1 13` | deck.1.volume (native) | Native engine path; physical test pending |
| Shift + Left CUE - Toggle quantize on/off (Deck1) | `90 68` | deck.0.quantize | Mapped; physical test pending |
| Shift + Right CUE - Toggle quantize on/off (Deck2) | `91 68` | deck.1.quantize | Mapped; physical test pending |
| TRIM - rotate | `B0 24` | deck.0.trim LSB | 14-bit decoding tested |
| TRIM - rotate | `B0 04` | deck.0.trim (native) | Native engine path; physical test pending |
| TRIM - rotate | `B1 24` | deck.1.trim LSB | 14-bit decoding tested |
| TRIM - rotate | `B1 04` | deck.1.trim (native) | Native engine path; physical test pending |
| EQ HI - rotate | `B0 27` | deck.0.eq.high LSB | 14-bit decoding tested |
| EQ MID - rotate | `B0 2B` | deck.0.eq.mid LSB | 14-bit decoding tested |
| EQ LOW - rotate | `B0 2F` | deck.0.eq.low LSB | 14-bit decoding tested |
| EQ HI - rotate | `B0 07` | deck.0.eq.high (native) | Native engine path; physical test pending |
| EQ MID - rotate | `B0 0B` | deck.0.eq.mid (native) | Native engine path; physical test pending |
| EQ LOW - rotate | `B0 0F` | deck.0.eq.low (native) | Native engine path; physical test pending |
| EQ HI - rotate | `B1 27` | deck.1.eq.high LSB | 14-bit decoding tested |
| EQ MID - rotate | `B1 2B` | deck.1.eq.mid LSB | 14-bit decoding tested |
| EQ LOW - rotate | `B1 2F` | deck.1.eq.low LSB | 14-bit decoding tested |
| EQ HI - rotate | `B1 07` | deck.1.eq.high (native) | Native engine path; physical test pending |
| EQ MID - rotate | `B1 0B` | deck.1.eq.mid (native) | Native engine path; physical test pending |
| EQ LOW - rotate | `B1 0F` | deck.1.eq.low (native) | Native engine path; physical test pending |
| CUE Channel - press - toggle Headphone Cue | `90 54` | deck.0.headphone_cue | Mapped; physical test pending |
| CUE Channel - press - toggle Headphone Cue | `91 54` | deck.1.headphone_cue | Mapped; physical test pending |
| HEADPHONES MIXING - rotate - Monitor Balance | `B6 2C` | mixer.cue_mix LSB | 14-bit decoding tested |
| HEADPHONES MIXING - rotate - Monitor Balance | `B6 0C` | mixer.cue_mix | Mapped; physical test pending |
| FILTER CH1 - rotate - Filter Effect Knob | `B6 17` | deck.0.filter (native) | Native engine path; physical test pending |
| FILTER CH1 - rotate - Filter Effect Knob | `B6 37` | deck.0.filter LSB | 14-bit decoding tested |
| FILTER CH2 - rotate - Filter Effect Knob | `B6 18` | deck.1.filter (native) | Native engine path; physical test pending |
| FILTER CH2 - rotate - Filter Effect Knob | `B6 38` | deck.1.filter LSB | 14-bit decoding tested |
| BEAT FX SELECT - press once - select next effect | `94 63` | fx.next | Mapped to OpenDeck per-deck inserts, not rekordbox Beat FX DSP; physical test pending |
| BEAT FX SELECT + shift - press once - select previous effect | `94 64` | fx.prev | Mapped to OpenDeck per-deck inserts, not rekordbox Beat FX DSP; physical test pending |
| BEAT LEFT - press - select previous effect unit | `94 4A` | fx.beat_down (echo/flanger beat length) | Mapped to OpenDeck per-deck inserts, not rekordbox Beat FX DSP; physical test pending |
| BEAT RIGHT - press - select next effect unit | `94 4B` | fx.beat_up (echo/flanger beat length) | Mapped to OpenDeck per-deck inserts, not rekordbox Beat FX DSP; physical test pending |
| BEAT FX CH SELECT CH1 - slide - Select FX on DECK 1 | `94 10` | fx.channel.0 | Mapped to OpenDeck per-deck inserts, not rekordbox Beat FX DSP; physical test pending |
| BEAT FX CH SELECT CH2 - slide - Select FX on DECK 2 | `95 11` | fx.channel.1 | Mapped to OpenDeck per-deck inserts, not rekordbox Beat FX DSP; physical test pending |
| BEAT FX LEVEL/DEPTH - rotate (MSB) - Adjust FX Level (mix) and BEAT FX Depth (meta) in the selected slot | `B4 02` | fx.level | Mapped to OpenDeck per-deck inserts, not rekordbox Beat FX DSP; physical test pending |
| BEAT FX ON/OFF - press - Toggle FX in the selected Slot (when on CHAN1/1+2) | `94 47` | fx.toggle.0 | Mapped to OpenDeck per-deck inserts, not rekordbox Beat FX DSP; physical test pending |
| BEAT FX ON/OFF - press - Toggle FX in the selected Slot (when on CHAN2) | `95 47` | fx.toggle.1 | Mapped to OpenDeck per-deck inserts, not rekordbox Beat FX DSP; physical test pending |
| BEAT FX ON/OFF +SHIFT - press - Disable all enabled Beat FX Slots (when on CHAN1/1+2) | `94 43` | fx.off_all | Mapped to OpenDeck per-deck inserts, not rekordbox Beat FX DSP; physical test pending |
| BEAT FX ON/OFF +SHIFT - press - Disable all enabled Beat FX Slots (when on CHAN12) | `95 43` | fx.off_all | Mapped to OpenDeck per-deck inserts, not rekordbox Beat FX DSP; physical test pending |
| HOT CUE MODE (DECK1) - press - set hotcue mode | `90 1B` | Supported pad-mode LED | Implemented; physical test pending |
| HOT CUE MODE (DECK2) - press - set hotcue mode | `91 1B` | Supported pad-mode LED | Implemented; physical test pending |
| KEYBOARD MODE (DECK1) - press - set keyboard mode | `90 69` | Keyboard selection reset and held-pitch restore | Implemented offline; physical test pending |
| KEYBOARD MODE (DECK2) - press - set keyboard mode | `91 69` | Keyboard selection reset and held-pitch restore | Implemented offline; physical test pending |
| PAD FX1 MODE (DECK1) - press - set pad fx1 mode | `90 1E` | Pad-mode latch; leaving Pad FX1 or Pad FX2 releases that deck's held inserts | Implemented offline; physical test pending |
| PAD FX1 MODE (DECK2) - press - set pad fx1 mode | `91 1E` | Pad-mode latch; leaving Pad FX1 or Pad FX2 releases that deck's held inserts | Implemented offline; physical test pending |
| PAD FX2 MODE (DECK1) - press - set pad fx2 mode | `90 6B` | Pad-mode latch; slots 16–31; leaving the mode releases held inserts | Implemented offline; physical test pending |
| PAD FX2 MODE (DECK2) - press - set pad fx2 mode | `91 6B` | Pad-mode latch; slots 16–31; leaving the mode releases held inserts | Implemented offline; physical test pending |
| PAD 1–4 (DECK1) PAD FX1 - hold - panel echo, flanger, reverb, bit-crusher | `97 10`–`97 13` | deck.0.pad_fx.0–3 | Hold enables, release disables. Does not steal a latched insert. Physical test pending |
| PAD 1–4 (DECK2) PAD FX1 - hold - panel echo, flanger, reverb, bit-crusher | `99 10`–`99 13` | deck.1.pad_fx.0–3 | Hold enables, release disables. Does not steal a latched insert. Physical test pending |
| PAD 5–8, SHIFT PAD FX1, PAD FX2, and SHIFT PAD FX2 | `97/99 14`–`17`, `98/9A 10`–`17`, `97/99 50`–`57`, `98/9A 50`–`57` | deck.0/1.pad_fx.4–31 | Open Deck presets in the pad-mode matrix. Release restores the saved panel parameters and any Beat FX latch. Physical test pending |
| BEAT JUMP MODE (DECK1) - press - set beat jump mode | `90 20` | Supported pad-mode LED | Implemented; physical test pending |
| BEAT JUMP MODE (DECK2) - press - set beat jump mode | `91 20` | Supported pad-mode LED | Implemented; physical test pending |
| BEAT LOOP MODE (DECK1) - press - set beat loop mode | `90 6D` | Supported pad-mode LED | Implemented; physical test pending |
| BEAT LOOP MODE (DECK2) - press - set beat loop mode | `91 6D` | Supported pad-mode LED | Implemented; physical test pending |
| SAMPLER MODE (DECK1) - press - set sampler mode | `90 22` | Supported pad-mode LED | Implemented; physical test pending |
| SAMPLER MODE (DECK2) - press - set sampler mode | `91 22` | Supported pad-mode LED | Implemented; physical test pending |
| KEY SHIFT MODE (DECK1) - press - set key shift mode | `90 6F` | Supported pad-mode LED | Implemented; physical test pending |
| KEY SHIFT MODE (DECK2) - press - set key shift mode | `91 6F` | Supported pad-mode LED | Implemented; physical test pending |
| PAD 1 (DECK1) HOT CUE MODE - press - set hotcue | `97 00` | deck.0.hotcue.0 | Mapped; physical test pending |
| PAD 1 +SHIFT (DECK1) HOT CUE MODE - press - delete hotcue | `98 00` | deck.0.clear_hotcue.0 | Mapped; physical test pending |
| PAD 1 (DECK2) HOT CUE MODE - press - set hotcue | `99 00` | deck.1.hotcue.0 | Mapped; physical test pending |
| PAD 1 +SHIFT (DECK2) HOT CUE MODE - press - delete hotcue | `9A 00` | deck.1.clear_hotcue.0 | Mapped; physical test pending |
| PAD 2 (DECK1) HOT CUE MODE - press - set hotcue | `97 01` | deck.0.hotcue.1 | Mapped; physical test pending |
| PAD 2 +SHIFT (DECK1) HOT CUE MODE - press - delete hotcue | `98 01` | deck.0.clear_hotcue.1 | Mapped; physical test pending |
| PAD 2 (DECK2) HOT CUE MODE - press - set hotcue | `99 01` | deck.1.hotcue.1 | Mapped; physical test pending |
| PAD 2 +SHIFT (DECK2) HOT CUE MODE - press - delete hotcue | `9A 01` | deck.1.clear_hotcue.1 | Mapped; physical test pending |
| PAD 3 (DECK1) HOT CUE MODE - press - set hotcue | `97 02` | deck.0.hotcue.2 | Mapped; physical test pending |
| PAD 3 +SHIFT (DECK1) HOT CUE MODE - press - delete hotcue | `98 02` | deck.0.clear_hotcue.2 | Mapped; physical test pending |
| PAD 3 (DECK2) HOT CUE MODE - press - set hotcue | `99 02` | deck.1.hotcue.2 | Mapped; physical test pending |
| PAD 3 +SHIFT (DECK2) HOT CUE MODE - press - delete hotcue | `9A 02` | deck.1.clear_hotcue.2 | Mapped; physical test pending |
| PAD 4 (DECK1) HOT CUE MODE - press - set hotcue | `97 03` | deck.0.hotcue.3 | Mapped; physical test pending |
| PAD 4 +SHIFT (DECK1) HOT CUE MODE - press - delete hotcue | `98 03` | deck.0.clear_hotcue.3 | Mapped; physical test pending |
| PAD 4 (DECK2) HOT CUE MODE - press - set hotcue | `99 03` | deck.1.hotcue.3 | Mapped; physical test pending |
| PAD 4 +SHIFT (DECK2) HOT CUE MODE - press - delete hotcue | `9A 03` | deck.1.clear_hotcue.3 | Mapped; physical test pending |
| PAD 5(DECK1) HOT CUE MODE - press - set hotcue | `97 04` | deck.0.hotcue.4 | Mapped; physical test pending |
| PAD 5 +SHIFT (DECK1) HOT CUE MODE - press - delete hotcue | `98 04` | deck.0.clear_hotcue.4 | Mapped; physical test pending |
| PAD 5 (DECK2) HOT CUE MODE - press - set hotcue | `99 04` | deck.1.hotcue.4 | Mapped; physical test pending |
| PAD 5 +SHIFT (DECK2) HOT CUE MODE - press - delete hotcue | `9A 04` | deck.1.clear_hotcue.4 | Mapped; physical test pending |
| PAD 6 (DECK1) HOT CUE MODE - press - set hotcue | `97 05` | deck.0.hotcue.5 | Mapped; physical test pending |
| PAD 6 +SHIFT (DECK1) HOT CUE MODE - press - delete hotcue | `98 05` | deck.0.clear_hotcue.5 | Mapped; physical test pending |
| PAD 6 (DECK2) HOT CUE MODE - press - set hotcue | `99 05` | deck.1.hotcue.5 | Mapped; physical test pending |
| PAD 6 +SHIFT (DECK2) HOT CUE MODE - press - delete hotcue | `9A 05` | deck.1.clear_hotcue.5 | Mapped; physical test pending |
| PAD 7 (DECK1) HOT CUE MODE - press - set hotcue | `97 06` | deck.0.hotcue.6 | Mapped; physical test pending |
| PAD 7 +SHIFT (DECK1) HOT CUE MODE - press - delete hotcue | `98 06` | deck.0.clear_hotcue.6 | Mapped; physical test pending |
| PAD 7 (DECK2) HOT CUE MODE - press - set hotcue | `99 06` | deck.1.hotcue.6 | Mapped; physical test pending |
| PAD 7 +SHIFT (DECK2) HOT CUE MODE - press - delete hotcue | `9A 06` | deck.1.clear_hotcue.6 | Mapped; physical test pending |
| PAD 8 (DECK1) HOT CUE MODE - press - set hotcue | `97 07` | deck.0.hotcue.7 | Mapped; physical test pending |
| PAD 8 +SHIFT (DECK1) HOT CUE MODE - press - delete hotcue | `98 07` | deck.0.clear_hotcue.7 | Mapped; physical test pending |
| PAD 8 (DECK2) HOT CUE MODE - press - set hotcue | `99 07` | deck.1.hotcue.7 | Mapped; physical test pending |
| PAD 8 +SHIFT (DECK2) HOT CUE MODE - press - delete hotcue | `9A 07` | deck.1.clear_hotcue.7 | Mapped; physical test pending |
| PAD 1–8 KEYBOARD - press - play the selected hot cue at +4,+5,+6,+7,±0,+1,+2,+3 semitones | `97/99 40`–`47` | deck.0/1.keyboard.0–7 | First press selects or stores the hot cue. Later presses seek, play, and pitch. Release restores pitch when the last pad is up. Playback continues. Physical test pending |
| PAD 1–6 +SHIFT KEYBOARD - press - delete the selected hot cue | `98/9A 40`–`45` | deck.0/1.keyboard_shift.0–5 | Deletes the selected cue, or that pad's hot cue when none is selected. Physical test pending |
| PAD 7 +SHIFT KEYBOARD - press - raise the scale one semitone | `98/9A 46` | deck.0/1.keyboard_shift.6 | Clamped to +12. A held pad updates immediately. Physical test pending |
| PAD 8 +SHIFT KEYBOARD - press - lower the scale one semitone | `98/9A 47` | deck.0/1.keyboard_shift.7 | Clamped to −12. A held pad updates immediately. Physical test pending |
| PAD 1 (DECK1) BEAT LOOP MODE - press - 1/4 Beatloop | `97 60` | deck.0.beatloop.0 | Mapped; physical test pending |
| PAD 1 (DECK2) BEAT LOOP MODE - press - 1/4 Beatloop | `99 60` | deck.1.beatloop.0 | Mapped; physical test pending |
| PAD 2 (DECK1) BEAT LOOP MODE - press - 1/2 Beatloop | `97 61` | deck.0.beatloop.1 | Mapped; physical test pending |
| PAD 2 (DECK2) BEAT LOOP MODE - press - 1/2 Beatloop | `99 61` | deck.1.beatloop.1 | Mapped; physical test pending |
| PAD 3 (DECK1) BEAT LOOP MODE - press - 1/1 Beatloop | `97 62` | deck.0.beatloop.2 | Mapped; physical test pending |
| PAD 3 (DECK2) BEAT LOOP MODE - press - 1/1 Beatloop | `99 62` | deck.1.beatloop.2 | Mapped; physical test pending |
| PAD 4 (DECK1) BEAT LOOP MODE - press - 2 Beatloop | `97 63` | deck.0.beatloop.3 | Mapped; physical test pending |
| PAD 4 (DECK2) BEAT LOOP MODE - press - 2 Beatloop | `99 63` | deck.1.beatloop.3 | Mapped; physical test pending |
| PAD 5 (DECK1) BEAT LOOP MODE - press - 4 Beatloop | `97 64` | deck.0.beatloop.4 | Mapped; physical test pending |
| PAD 5 (DECK2) BEAT LOOP MODE - press - 4 Beatloop | `99 64` | deck.1.beatloop.4 | Mapped; physical test pending |
| PAD 6 (DECK1) BEAT LOOP MODE - press - 8 Beatloop | `97 65` | deck.0.beatloop.5 | Mapped; physical test pending |
| PAD 6 (DECK2) BEAT LOOP MODE - press - 8 Beatloop | `99 65` | deck.1.beatloop.5 | Mapped; physical test pending |
| PAD 7 (DECK1) BEAT LOOP MODE - press - 16 Beatloop | `97 66` | deck.0.beatloop.6 | Mapped; physical test pending |
| PAD 7 (DECK2) BEAT LOOP MODE - press - 16 Beatloop | `99 66` | deck.1.beatloop.6 | Mapped; physical test pending |
| PAD 8 (DECK1) BEAT LOOP MODE - press - 32 Beatloop | `97 67` | deck.0.beatloop.7 | Mapped; physical test pending |
| PAD 8 (DECK2) BEAT LOOP MODE - press - 32 Beatloop | `99 67` | deck.1.beatloop.7 | Mapped; physical test pending |
| PAD 1 (DECK1) BEAT JUMP MODE - press - Jump 1 Beat backwards | `97 20` | deck.0.beatjump.0 | Mapped; physical test pending |
| PAD 1 (DECK2) BEAT JUMP MODE - press - Jump 1 Beat backwards | `99 20` | deck.1.beatjump.0 | Mapped; physical test pending |
| PAD 2 (DECK1) BEAT JUMP MODE - press - Jump 1 Beat forwards | `97 21` | deck.0.beatjump.1 | Mapped; physical test pending |
| PAD 2 (DECK2) BEAT JUMP MODE - press - Jump 1 Beat forwards | `99 21` | deck.1.beatjump.1 | Mapped; physical test pending |
| PAD 3 (DECK1) BEAT JUMP MODE - press - Jump 2 Beats backwards | `97 22` | deck.0.beatjump.2 | Mapped; physical test pending |
| PAD 3 (DECK2) BEAT JUMP MODE - press - Jump 2 Beats backwards | `99 22` | deck.1.beatjump.2 | Mapped; physical test pending |
| PAD 4 (DECK1) BEAT JUMP MODE - press - Jump 2 Beats forwards | `97 23` | deck.0.beatjump.3 | Mapped; physical test pending |
| PAD 4 (DECK2) BEAT JUMP MODE - press - Jump 2 Beats forwards | `99 23` | deck.1.beatjump.3 | Mapped; physical test pending |
| PAD 5 (DECK1) BEAT JUMP MODE - press - Jump 4 Beats backwards | `97 24` | deck.0.beatjump.4 | Mapped; physical test pending |
| PAD 5 (DECK2) BEAT JUMP MODE - press - Jump 4 Beats backwards | `99 24` | deck.1.beatjump.4 | Mapped; physical test pending |
| PAD 6 (DECK1) BEAT JUMP MODE - press - Jump 4 Beats forwards | `97 25` | deck.0.beatjump.5 | Mapped; physical test pending |
| PAD 6 (DECK2) BEAT JUMP MODE - press - Jump 4 Beats forwards | `99 25` | deck.1.beatjump.5 | Mapped; physical test pending |
| PAD 7 (DECK1) BEAT JUMP MODE - press - Jump 8 Beats backwards | `97 26` | deck.0.beatjump.6 | Mapped; physical test pending |
| PAD 7 (DECK2) BEAT JUMP MODE - press - Jump 8 Beats backwards | `99 26` | deck.1.beatjump.6 | Mapped; physical test pending |
| PAD 8 (DECK1) BEAT JUMP MODE - press - Jump 8 Beats forwards | `97 27` | deck.0.beatjump.7 | Mapped; physical test pending |
| PAD 8 (DECK2) BEAT JUMP MODE - press - Jump 8 Beats forwards | `99 27` | deck.1.beatjump.7 | Mapped; physical test pending |
| PAD 7 (DECK1) +SHift BEAT JUMP MODE - press - decrease Beatjump by a factor of 16 | `98 26` | deck.0.beatjump.scale_down | Implemented; physical test pending. Same address as track_loaded LED |
| PAD 7 (DECK2) +Shift BEAT JUMP MODE - press - decrease Beatjump by a factor of 16 | `9A 26` | deck.1.beatjump.scale_down | Implemented; physical test pending. Same address as track_loaded LED |
| PAD 8 (DECK1) +SHift BEAT JUMP MODE - press - increase Beatjump by a factor of 16 | `98 27` | deck.0.beatjump.scale_up | Implemented; physical test pending. Same address as track_loaded LED |
| PAD 8 (DECK2) +Shift BEAT JUMP MODE - press - increase Beatjump by a factor of 16 | `9A 27` | deck.1.beatjump.scale_up | Implemented; physical test pending. Same address as track_loaded LED |
| PAD 1 (LEFT) SAMPLE MODE - press - Play Sample or Load Track | `97 30` | sampler.0.trigger | Press plays that bank slot. A second press restarts a one-shot. An empty pad does nothing. Velocity 0–63 does not play. Physical test pending |
| PAD 1 (LEFT)+SHIFT SAMPLE MODE - press - Stop Playback or Eject Track | `98 30` | sampler.0.stop | SHIFT stops the slot while it is playing. While idle, SHIFT loads the selected browser track into it. A missing selection or failed decode leaves the slot. Velocity 0–63 does neither. Physical test pending |
| PAD 2 (LEFT) SAMPLE MODE - press - Play Sample or Load Track | `97 31` | sampler.1.trigger | Press plays that bank slot. A second press restarts a one-shot. An empty pad does nothing. Velocity 0–63 does not play. Physical test pending |
| PAD 2 (LEFT)+SHIFT SAMPLE MODE - press - Stop Playback or Eject Track | `98 31` | sampler.1.stop | SHIFT stops the slot while it is playing. While idle, SHIFT loads the selected browser track into it. A missing selection or failed decode leaves the slot. Velocity 0–63 does neither. Physical test pending |
| PAD 3 (LEFT) SAMPLE MODE - press - Play Sample or Load Track | `97 32` | sampler.2.trigger | Press plays that bank slot. A second press restarts a one-shot. An empty pad does nothing. Velocity 0–63 does not play. Physical test pending |
| PAD 3 (LEFT)+SHIFT SAMPLE MODE - press - Stop Playback or Eject Track | `98 32` | sampler.2.stop | SHIFT stops the slot while it is playing. While idle, SHIFT loads the selected browser track into it. A missing selection or failed decode leaves the slot. Velocity 0–63 does neither. Physical test pending |
| PAD 4 (LEFT) SAMPLE MODE - press - Play Sample or Load Track | `97 33` | sampler.3.trigger | Press plays that bank slot. A second press restarts a one-shot. An empty pad does nothing. Velocity 0–63 does not play. Physical test pending |
| PAD 4 (LEFT)+SHIFT SAMPLE MODE - press - Stop Playback or Eject Track | `98 33` | sampler.3.stop | SHIFT stops the slot while it is playing. While idle, SHIFT loads the selected browser track into it. A missing selection or failed decode leaves the slot. Velocity 0–63 does neither. Physical test pending |
| PAD 5 (LEFT) SAMPLE MODE - press - Play Sample or Load Track | `97 34` | sampler.4.trigger | Press plays that bank slot. A second press restarts a one-shot. An empty pad does nothing. Velocity 0–63 does not play. Physical test pending |
| PAD 5 (LEFT)+SHIFT SAMPLE MODE - press - Stop Playback or Eject Track | `98 34` | sampler.4.stop | SHIFT stops the slot while it is playing. While idle, SHIFT loads the selected browser track into it. A missing selection or failed decode leaves the slot. Velocity 0–63 does neither. Physical test pending |
| PAD 6 (LEFT) SAMPLE MODE - press - Play Sample or Load Track | `97 35` | sampler.5.trigger | Press plays that bank slot. A second press restarts a one-shot. An empty pad does nothing. Velocity 0–63 does not play. Physical test pending |
| PAD 6 (LEFT)+SHIFT SAMPLE MODE - press - Stop Playback or Eject Track | `98 35` | sampler.5.stop | SHIFT stops the slot while it is playing. While idle, SHIFT loads the selected browser track into it. A missing selection or failed decode leaves the slot. Velocity 0–63 does neither. Physical test pending |
| PAD 7 (LEFT) SAMPLE MODE - press - Play Sample or Load Track | `97 36` | sampler.6.trigger | Press plays that bank slot. A second press restarts a one-shot. An empty pad does nothing. Velocity 0–63 does not play. Physical test pending |
| PAD 7 (LEFT)+SHIFT SAMPLE MODE - press - Stop Playback or Eject Track | `98 36` | sampler.6.stop | SHIFT stops the slot while it is playing. While idle, SHIFT loads the selected browser track into it. A missing selection or failed decode leaves the slot. Velocity 0–63 does neither. Physical test pending |
| PAD 8 (LEFT) SAMPLE MODE - press - Play Sample or Load Track | `97 37` | sampler.7.trigger | Press plays that bank slot. A second press restarts a one-shot. An empty pad does nothing. Velocity 0–63 does not play. Physical test pending |
| PAD 8 (LEFT)+SHIFT SAMPLE MODE - press - Stop Playback or Eject Track | `98 37` | sampler.7.stop | SHIFT stops the slot while it is playing. While idle, SHIFT loads the selected browser track into it. A missing selection or failed decode leaves the slot. Velocity 0–63 does neither. Physical test pending |
| PAD 1 (RIGHT) SAMPLE MODE - press - Play Sample or Load Track | `99 30` | sampler.8.trigger | Press plays that bank slot. A second press restarts a one-shot. An empty pad does nothing. Velocity 0–63 does not play. Physical test pending |
| PAD 1 (Right)+SHIFT SAMPLE MODE - press - Stop Playback or Eject Track | `9A 30` | sampler.8.stop | SHIFT stops the slot while it is playing. While idle, SHIFT loads the selected browser track into it. A missing selection or failed decode leaves the slot. Velocity 0–63 does neither. Physical test pending |
| PAD 2 (RIGHT) SAMPLE MODE - press - Play Sample or Load Track | `99 31` | sampler.9.trigger | Press plays that bank slot. A second press restarts a one-shot. An empty pad does nothing. Velocity 0–63 does not play. Physical test pending |
| PAD 2 (Right)+SHIFT SAMPLE MODE - press - Stop Playback or Eject Track | `9A 31` | sampler.9.stop | SHIFT stops the slot while it is playing. While idle, SHIFT loads the selected browser track into it. A missing selection or failed decode leaves the slot. Velocity 0–63 does neither. Physical test pending |
| PAD 3 (RIGHT) SAMPLE MODE - press - Play Sample or Load Track | `99 32` | sampler.10.trigger | Press plays that bank slot. A second press restarts a one-shot. An empty pad does nothing. Velocity 0–63 does not play. Physical test pending |
| PAD 3 (Right)+SHIFT SAMPLE MODE - press - Stop Playback or Eject Track | `9A 32` | sampler.10.stop | SHIFT stops the slot while it is playing. While idle, SHIFT loads the selected browser track into it. A missing selection or failed decode leaves the slot. Velocity 0–63 does neither. Physical test pending |
| PAD 4 (RIGHT) SAMPLE MODE - press - Play Sample or Load Track | `99 33` | sampler.11.trigger | Press plays that bank slot. A second press restarts a one-shot. An empty pad does nothing. Velocity 0–63 does not play. Physical test pending |
| PAD 4 (Right)+SHIFT SAMPLE MODE - press - Stop Playback or Eject Track | `9A 33` | sampler.11.stop | SHIFT stops the slot while it is playing. While idle, SHIFT loads the selected browser track into it. A missing selection or failed decode leaves the slot. Velocity 0–63 does neither. Physical test pending |
| PAD 5 (RIGHT) SAMPLE MODE - press - Play Sample or Load Track | `99 34` | sampler.12.trigger | Press plays that bank slot. A second press restarts a one-shot. An empty pad does nothing. Velocity 0–63 does not play. Physical test pending |
| PAD 5 (Right)+SHIFT SAMPLE MODE - press - Stop Playback or Eject Track | `9A 34` | sampler.12.stop | SHIFT stops the slot while it is playing. While idle, SHIFT loads the selected browser track into it. A missing selection or failed decode leaves the slot. Velocity 0–63 does neither. Physical test pending |
| PAD 6 (RIGHT) SAMPLE MODE - press - Play Sample or Load Track | `99 35` | sampler.13.trigger | Press plays that bank slot. A second press restarts a one-shot. An empty pad does nothing. Velocity 0–63 does not play. Physical test pending |
| PAD 6 (Right)+SHIFT SAMPLE MODE - press - Stop Playback or Eject Track | `9A 35` | sampler.13.stop | SHIFT stops the slot while it is playing. While idle, SHIFT loads the selected browser track into it. A missing selection or failed decode leaves the slot. Velocity 0–63 does neither. Physical test pending |
| PAD 7 (RIGHT) SAMPLE MODE - press - Play Sample or Load Track | `99 36` | sampler.14.trigger | Press plays that bank slot. A second press restarts a one-shot. An empty pad does nothing. Velocity 0–63 does not play. Physical test pending |
| PAD 7 (Right)+SHIFT SAMPLE MODE - press - Stop Playback or Eject Track | `9A 36` | sampler.14.stop | SHIFT stops the slot while it is playing. While idle, SHIFT loads the selected browser track into it. A missing selection or failed decode leaves the slot. Velocity 0–63 does neither. Physical test pending |
| PAD 8 (RIGHT) SAMPLE MODE - press - Play Sample or Load Track | `99 37` | sampler.15.trigger | Press plays that bank slot. A second press restarts a one-shot. An empty pad does nothing. Velocity 0–63 does not play. Physical test pending |
| PAD 8 (Right)+SHIFT SAMPLE MODE - press - Stop Playback or Eject Track | `9A 37` | sampler.15.stop | SHIFT stops the slot while it is playing. While idle, SHIFT loads the selected browser track into it. A missing selection or failed decode leaves the slot. Velocity 0–63 does neither. Physical test pending |
| PAD 1 (DECK1) KEY SHIFT MODE - press - No pitch modification | `97 70` | deck.0.key_shift.0 | Sets the sounding pitch to +4 plus the current transpose, clamped to ±12. The MIDI-list label in column 1 is the older scale. Physical test pending |
| PAD 1 +SHIFT (DECK1) KEY SHIFT MODE - press - Increase the pitch by 1 semitone | `98 70` | deck.0.key_shift_step.0 | Consumed. The Mac manual does not assign SHIFT pads 1–6, so pitch stays put. The MIDI-list label in column 1 is not this behavior. Physical test pending |
| PAD 1 (DECK2) KEY SHIFT MODE - press - No pitch modification | `99 70` | deck.1.key_shift.0 | Sets the sounding pitch to +4 plus the current transpose, clamped to ±12. The MIDI-list label in column 1 is the older scale. Physical test pending |
| PAD 1 +SHIFT (DECK2) KEY SHIFT MODE - press - Increase the pitch by 1 semitone | `9A 70` | deck.1.key_shift_step.0 | Consumed. The Mac manual does not assign SHIFT pads 1–6, so pitch stays put. The MIDI-list label in column 1 is not this behavior. Physical test pending |
| PAD 2 (DECK1) KEY SHIFT MODE - press - Set pitch to one semitone up | `97 71` | deck.0.key_shift.1 | Sets the sounding pitch to +5 plus the current transpose, clamped to ±12. The MIDI-list label in column 1 is the older scale. Physical test pending |
| PAD 2 +SHIFT (DECK1) KEY SHIFT MODE - press - Increase the pitch by 2 semitones | `98 71` | deck.0.key_shift_step.1 | Consumed. The Mac manual does not assign SHIFT pads 1–6, so pitch stays put. The MIDI-list label in column 1 is not this behavior. Physical test pending |
| PAD 2 (DECK2) KEY SHIFT MODE - press - Set pitch to one semitone up | `99 71` | deck.1.key_shift.1 | Sets the sounding pitch to +5 plus the current transpose, clamped to ±12. The MIDI-list label in column 1 is the older scale. Physical test pending |
| PAD 2 +SHIFT (DECK2) KEY SHIFT MODE - press - Increase the pitch by 2 semitones | `9A 71` | deck.1.key_shift_step.1 | Consumed. The Mac manual does not assign SHIFT pads 1–6, so pitch stays put. The MIDI-list label in column 1 is not this behavior. Physical test pending |
| PAD 3 (DECK1) KEY SHIFT MODE - press - Set the pitch to 2 semitones up | `97 72` | deck.0.key_shift.2 | Sets the sounding pitch to +6 plus the current transpose, clamped to ±12. The MIDI-list label in column 1 is the older scale. Physical test pending |
| PAD 3 +SHIFT (DECK1) KEY SHIFT MODE - press - Increase the pitch by 3 semitones | `98 72` | deck.0.key_shift_step.2 | Consumed. The Mac manual does not assign SHIFT pads 1–6, so pitch stays put. The MIDI-list label in column 1 is not this behavior. Physical test pending |
| PAD 3 (DECK2) KEY SHIFT MODE - press - Set the pitch to 2 semitones up | `99 72` | deck.1.key_shift.2 | Sets the sounding pitch to +6 plus the current transpose, clamped to ±12. The MIDI-list label in column 1 is the older scale. Physical test pending |
| PAD 3 +SHIFT (DECK2) KEY SHIFT MODE - press - Increase the pitch by 3 semitones | `9A 72` | deck.1.key_shift_step.2 | Consumed. The Mac manual does not assign SHIFT pads 1–6, so pitch stays put. The MIDI-list label in column 1 is not this behavior. Physical test pending |
| PAD 4 (DECK1) KEY SHIFT MODE - press - Set the pitch to 3 semitones up | `97 73` | deck.0.key_shift.3 | Sets the sounding pitch to +7 plus the current transpose, clamped to ±12. The MIDI-list label in column 1 is the older scale. Physical test pending |
| PAD 4 +SHIFT (DECK1) KEY SHIFT MODE - press - Increase the pitch by 4 semitones | `98 73` | deck.0.key_shift_step.3 | Consumed. The Mac manual does not assign SHIFT pads 1–6, so pitch stays put. The MIDI-list label in column 1 is not this behavior. Physical test pending |
| PAD 4 (DECK2) KEY SHIFT MODE - press - Set the pitch to 3 semitones up | `99 73` | deck.1.key_shift.3 | Sets the sounding pitch to +7 plus the current transpose, clamped to ±12. The MIDI-list label in column 1 is the older scale. Physical test pending |
| PAD 4 +SHIFT (DECK2) KEY SHIFT MODE - press - Increase the pitch by 4 semitones | `9A 73` | deck.1.key_shift_step.3 | Consumed. The Mac manual does not assign SHIFT pads 1–6, so pitch stays put. The MIDI-list label in column 1 is not this behavior. Physical test pending |
| PAD 5 (DECK1) KEY SHIFT MODE - press - Set the pitch to 3 semitones down | `97 74` | deck.0.key_shift.4 | Sets the sounding pitch to ±0 plus the current transpose, clamped to ±12. The MIDI-list label in column 1 is the older scale. Physical test pending |
| PAD 5 +SHIFT (DECK1) KEY SHIFT MODE - press - Decrease the pitch by 1 semitons | `98 74` | deck.0.key_shift_step.4 | Consumed. The Mac manual does not assign SHIFT pads 1–6, so pitch stays put. The MIDI-list label in column 1 is not this behavior. Physical test pending |
| PAD 5 (DECK2) KEY SHIFT MODE - press - Set the pitch to 3 semitones down | `99 74` | deck.1.key_shift.4 | Sets the sounding pitch to ±0 plus the current transpose, clamped to ±12. The MIDI-list label in column 1 is the older scale. Physical test pending |
| PAD 5 +SHIFT (DECK2) KEY SHIFT MODE - press - Decrease the pitch by 1 semitone | `9A 74` | deck.1.key_shift_step.4 | Consumed. The Mac manual does not assign SHIFT pads 1–6, so pitch stays put. The MIDI-list label in column 1 is not this behavior. Physical test pending |
| PAD 6 (DECK1) KEY SHIFT MODE - press - Set the pitch to 2 semitones down | `97 75` | deck.0.key_shift.5 | Sets the sounding pitch to +1 plus the current transpose, clamped to ±12. The MIDI-list label in column 1 is the older scale. Physical test pending |
| PAD 6 +SHIFT (DECK1) KEY SHIFT MODE - press - Decrease the pitch by 2 semitones | `98 75` | deck.0.key_shift_step.5 | Consumed. The Mac manual does not assign SHIFT pads 1–6, so pitch stays put. The MIDI-list label in column 1 is not this behavior. Physical test pending |
| PAD 6 (DECK2) KEY SHIFT MODE - press - Set the pitch to 2 semitones down | `99 75` | deck.1.key_shift.5 | Sets the sounding pitch to +1 plus the current transpose, clamped to ±12. The MIDI-list label in column 1 is the older scale. Physical test pending |
| PAD 6 +SHIFT (DECK2) KEY SHIFT MODE - press - Decrease the pitch by 2 semitones | `9A 75` | deck.1.key_shift_step.5 | Consumed. The Mac manual does not assign SHIFT pads 1–6, so pitch stays put. The MIDI-list label in column 1 is not this behavior. Physical test pending |
| PAD 7 (DECK1) KEY SHIFT MODE - press - Set the pitch to 1 semitones down | `97 76` | deck.0.key_shift.6 | Sets the sounding pitch to +2 plus the current transpose, clamped to ±12. The MIDI-list label in column 1 is the older scale. Physical test pending |
| PAD 7 +SHIFT (DECK1) KEY SHIFT MODE - press - Decrease the pitch by 3 semitones | `98 76` | deck.0.key_shift_step.6 | Raises the sounding pitch and the scale by one semitone, clamped to ±12. Physical test pending |
| PAD 7 (DECK2) KEY SHIFT MODE - press - Set the pitch to 1 semitones down | `99 76` | deck.1.key_shift.6 | Sets the sounding pitch to +2 plus the current transpose, clamped to ±12. The MIDI-list label in column 1 is the older scale. Physical test pending |
| PAD 7 +SHIFT (DECK2) KEY SHIFT MODE - press - Decrease the pitch by 3 semitones | `9A 76` | deck.1.key_shift_step.6 | Raises the sounding pitch and the scale by one semitone, clamped to ±12. Physical test pending |
| PAD 8 (DECK1) KEY SHIFT MODE - press - No pitch modification | `97 77` | deck.0.key_shift.7 | Sets the sounding pitch to +3 plus the current transpose, clamped to ±12. The MIDI-list label in column 1 is the older scale. Physical test pending |
| PAD 8 +SHIFT (DECK1) KEY SHIFT MODE - press - Decrease the pitch by 4 semitones | `98 77` | deck.0.key_shift_step.7 | Lowers the sounding pitch and the scale by one semitone, clamped to ±12. Physical test pending |
| PAD 8 (DECK2) KEY SHIFT MODE - press - No pitch modification | `99 77` | deck.1.key_shift.7 | Sets the sounding pitch to +3 plus the current transpose, clamped to ±12. The MIDI-list label in column 1 is the older scale. Physical test pending |
| PAD 8 +SHIFT (DECK2) KEY SHIFT MODE - press - Decrease the pitch by 4 semitones | `9A 77` | deck.1.key_shift_step.7 | Lowers the sounding pitch and the scale by one semitone, clamped to ±12. Physical test pending |

## LED output inventory

112 output rules from the reference mapping, plus script-driven meters/mode indicators/lifecycle above. A matching input address alone is not proof of correct output semantics. Hardware LED timing/brightness/blink parity remains unverified.

| Reference state | MIDI status/data1 | Current coverage |
|---|---|---|
| [Channel1] play_indicator | `90 0B` | Native engine-owned: lit while playing, 500 ms blink while paused and loaded; physical parity pending |
| [Channel2] play_indicator | `91 0B` | Native engine-owned: lit while playing, 500 ms blink while paused and loaded; physical parity pending |
| [Channel1] play_indicator | `90 47` | Not implemented |
| [Channel2] play_indicator | `91 47` | Not implemented |
| [Channel1] cue_indicator | `90 0C` | Lit while parked on the cue point or cue-previewing; physical test pending |
| [Channel2] cue_indicator | `91 0C` | Lit while parked on the cue point or cue-previewing; physical test pending |
| [Channel1] cue_indicator | `90 47` | Not implemented |
| [Channel2] cue_indicator | `91 47` | Not implemented |
| [Channel1] sync_enabled | `90 58` | State feedback implemented; physical parity pending |
| [Channel2] sync_enabled | `91 58` | State feedback implemented; physical parity pending |
| [Channel1] pfl | `90 54` | State feedback implemented; physical parity pending |
| [Channel2] pfl | `91 54` | State feedback implemented; physical parity pending |
| [Channel1] hotcue_1_status | `97 00` | State feedback implemented; physical parity pending |
| [Channel2] hotcue_1_status | `99 00` | State feedback implemented; physical parity pending |
| [Channel1] hotcue_2_status | `97 01` | State feedback implemented; physical parity pending |
| [Channel2] hotcue_2_status | `99 01` | State feedback implemented; physical parity pending |
| [Channel1] hotcue_3_status | `97 02` | State feedback implemented; physical parity pending |
| [Channel2] hotcue_3_status | `99 02` | State feedback implemented; physical parity pending |
| [Channel1] hotcue_4_status | `97 03` | State feedback implemented; physical parity pending |
| [Channel2] hotcue_4_status | `99 03` | State feedback implemented; physical parity pending |
| [Channel1] hotcue_5_status | `97 04` | State feedback implemented; physical parity pending |
| [Channel2] hotcue_5_status | `99 04` | State feedback implemented; physical parity pending |
| [Channel1] hotcue_6_status | `97 05` | State feedback implemented; physical parity pending |
| [Channel2] hotcue_6_status | `99 05` | State feedback implemented; physical parity pending |
| [Channel1] hotcue_7_status | `97 06` | State feedback implemented; physical parity pending |
| [Channel2] hotcue_7_status | `99 06` | State feedback implemented; physical parity pending |
| [Channel1] hotcue_8_status | `97 07` | State feedback implemented; physical parity pending |
| [Channel2] hotcue_8_status | `99 07` | State feedback implemented; physical parity pending |
| [Channel1] hotcue_1_status | `98 00` | State feedback implemented; physical parity pending |
| [Channel2] hotcue_1_status | `9A 00` | State feedback implemented; physical parity pending |
| [Channel1] hotcue_2_status | `98 01` | State feedback implemented; physical parity pending |
| [Channel2] hotcue_2_status | `9A 01` | State feedback implemented; physical parity pending |
| [Channel1] hotcue_3_status | `98 02` | State feedback implemented; physical parity pending |
| [Channel2] hotcue_3_status | `9A 02` | State feedback implemented; physical parity pending |
| [Channel1] hotcue_4_status | `98 03` | State feedback implemented; physical parity pending |
| [Channel2] hotcue_4_status | `9A 03` | State feedback implemented; physical parity pending |
| [Channel1] hotcue_5_status | `98 04` | State feedback implemented; physical parity pending |
| [Channel2] hotcue_5_status | `9A 04` | State feedback implemented; physical parity pending |
| [Channel1] hotcue_6_status | `98 05` | State feedback implemented; physical parity pending |
| [Channel2] hotcue_6_status | `9A 05` | State feedback implemented; physical parity pending |
| [Channel1] hotcue_7_status | `98 06` | State feedback implemented; physical parity pending |
| [Channel2] hotcue_7_status | `9A 06` | State feedback implemented; physical parity pending |
| [Channel1] hotcue_8_status | `98 07` | State feedback implemented; physical parity pending |
| [Channel2] hotcue_8_status | `9A 07` | State feedback implemented; physical parity pending |
| [Channel1] beatloop_0.25_enabled | `97 60` | State feedback implemented; physical parity pending |
| [Channel2] beatloop_0.25_enabled | `99 60` | State feedback implemented; physical parity pending |
| [Channel1] beatloop_0.5_enabled | `97 61` | State feedback implemented; physical parity pending |
| [Channel2] beatloop_0.5_enabled | `99 61` | State feedback implemented; physical parity pending |
| [Channel1] beatloop_1_enabled | `97 62` | State feedback implemented; physical parity pending |
| [Channel2] beatloop_1_enabled | `99 62` | State feedback implemented; physical parity pending |
| [Channel1] beatloop_2_enabled | `97 63` | State feedback implemented; physical parity pending |
| [Channel2] beatloop_2_enabled | `99 63` | State feedback implemented; physical parity pending |
| [Channel1] beatloop_4_enabled | `97 64` | State feedback implemented; physical parity pending |
| [Channel2] beatloop_4_enabled | `99 64` | State feedback implemented; physical parity pending |
| [Channel1] beatloop_8_enabled | `97 65` | State feedback implemented; physical parity pending |
| [Channel2] beatloop_8_enabled | `99 65` | State feedback implemented; physical parity pending |
| [Channel1] beatloop_16_enabled | `97 66` | State feedback implemented; physical parity pending |
| [Channel2] beatloop_16_enabled | `99 66` | State feedback implemented; physical parity pending |
| [Channel1] beatloop_32_enabled | `97 67` | State feedback implemented; physical parity pending |
| [Channel2] beatloop_32_enabled | `99 67` | State feedback implemented; physical parity pending |
| [Channel1] beatloop_0.25_enabled | `98 60` | Mirrors the unshifted beat-loop LED; physical test pending |
| [Channel2] beatloop_0.25_enabled | `9A 60` | Mirrors the unshifted beat-loop LED; physical test pending |
| [Channel1] beatloop_0.5_enabled | `98 61` | Mirrors the unshifted beat-loop LED; physical test pending |
| [Channel2] beatloop_0.5_enabled | `9A 61` | Mirrors the unshifted beat-loop LED; physical test pending |
| [Channel1] beatloop_1_enabled | `98 62` | Mirrors the unshifted beat-loop LED; physical test pending |
| [Channel2] beatloop_1_enabled | `9A 62` | Mirrors the unshifted beat-loop LED; physical test pending |
| [Channel1] beatloop_2_enabled | `98 63` | Mirrors the unshifted beat-loop LED; physical test pending |
| [Channel2] beatloop_2_enabled | `9A 63` | Mirrors the unshifted beat-loop LED; physical test pending |
| [Channel1] beatloop_4_enabled | `98 64` | Mirrors the unshifted beat-loop LED; physical test pending |
| [Channel2] beatloop_4_enabled | `9A 64` | Mirrors the unshifted beat-loop LED; physical test pending |
| [Channel1] beatloop_8_enabled | `98 65` | Mirrors the unshifted beat-loop LED; physical test pending |
| [Channel2] beatloop_8_enabled | `9A 65` | Mirrors the unshifted beat-loop LED; physical test pending |
| [Channel1] beatloop_16_enabled | `98 66` | Mirrors the unshifted beat-loop LED; physical test pending |
| [Channel2] beatloop_16_enabled | `9A 66` | Mirrors the unshifted beat-loop LED; physical test pending |
| [Channel1] beatloop_32_enabled | `98 67` | Mirrors the unshifted beat-loop LED; physical test pending |
| [Channel2] beatloop_32_enabled | `9A 67` | Mirrors the unshifted beat-loop LED; physical test pending |
| [Channel1] track_loaded | `98 26` | Lit while deck 1 has a track; physical test pending. Same address as beat-jump scale |
| [Channel1] track_loaded | `98 27` | Lit while deck 1 has a track; physical test pending. Same address as beat-jump scale |
| [Channel2] track_loaded | `9A 26` | Lit while deck 2 has a track; physical test pending. Same address as beat-jump scale |
| [Channel2] track_loaded | `9A 27` | Lit while deck 2 has a track; physical test pending. Same address as beat-jump scale |
| [Sampler1] track_loaded | `97 30` | Lit while the shared sampler slot is loaded; physical parity pending |
| [Sampler2] track_loaded | `97 31` | Lit while the shared sampler slot is loaded; physical parity pending |
| [Sampler3] track_loaded | `97 32` | Lit while the shared sampler slot is loaded; physical parity pending |
| [Sampler4] track_loaded | `97 33` | Lit while the shared sampler slot is loaded; physical parity pending |
| [Sampler9] track_loaded | `97 34` | Lit while the shared sampler slot is loaded; physical parity pending |
| [Sampler10] track_loaded | `97 35` | Lit while the shared sampler slot is loaded; physical parity pending |
| [Sampler11] track_loaded | `97 36` | Lit while the shared sampler slot is loaded; physical parity pending |
| [Sampler12] track_loaded | `97 37` | Lit while the shared sampler slot is loaded; physical parity pending |
| [Sampler5] track_loaded | `99 30` | Lit while the shared sampler slot is loaded; physical parity pending |
| [Sampler6] track_loaded | `99 31` | Lit while the shared sampler slot is loaded; physical parity pending |
| [Sampler7] track_loaded | `99 32` | Lit while the shared sampler slot is loaded; physical parity pending |
| [Sampler8] track_loaded | `99 33` | Lit while the shared sampler slot is loaded; physical parity pending |
| [Sampler13] track_loaded | `99 34` | Lit while the shared sampler slot is loaded; physical parity pending |
| [Sampler14] track_loaded | `99 35` | Lit while the shared sampler slot is loaded; physical parity pending |
| [Sampler15] track_loaded | `99 36` | Lit while the shared sampler slot is loaded; physical parity pending |
| [Sampler16] track_loaded | `99 37` | Lit while the shared sampler slot is loaded; physical parity pending |
| [Sampler1] track_loaded | `98 30` | Not implemented |
| [Sampler2] track_loaded | `98 31` | Not implemented |
| [Sampler3] track_loaded | `98 32` | Not implemented |
| [Sampler4] track_loaded | `98 33` | Not implemented |
| [Sampler9] track_loaded | `98 34` | Not implemented |
| [Sampler10] track_loaded | `98 35` | Not implemented |
| [Sampler11] track_loaded | `98 36` | Not implemented |
| [Sampler12] track_loaded | `98 37` | Not implemented |
| [Sampler5] track_loaded | `9A 30` | Not implemented |
| [Sampler6] track_loaded | `9A 31` | Not implemented |
| [Sampler7] track_loaded | `9A 32` | Not implemented |
| [Sampler8] track_loaded | `9A 33` | Not implemented |
| [Sampler13] track_loaded | `9A 34` | Not implemented |
| [Sampler14] track_loaded | `9A 35` | Not implemented |
| [Sampler15] track_loaded | `9A 36` | Not implemented |
| [Sampler16] track_loaded | `9A 37` | Not implemented |
