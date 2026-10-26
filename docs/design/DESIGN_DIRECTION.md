# OpenDeck visual direction

Established 2026-10-25 from the user's three reference screenshots and explicit preferences. This is the ongoing UI direction for future agents.

## Intent

A calm macOS-style DJ workspace: neutral graphite surfaces, system typography, thin dividers, purposeful spacing, restrained controls. Square panels/fields/buttons are the default; radius is reserved for functional circular controls and native OS geometry. Favor readable information hierarchy over simulated hardware textures, neon glow, branding or ornament. Use native window controls. React/Tauri remains the implementation; do not claim the full interface is AppKit or Apple-made.

## Reference translation

- VirtualDJ performance browser: left collections/folders sidebar, dense track list, separate right inspector showing artwork and actual track information. Select a row without loading a deck. Explicit labeled deck loads and drag/drop remain available.
- rekordbox waveforms: high-contrast audio envelopes, beat/downbeat organization, clear current position and full-track overviews. Add real small waveform representations to library rows. Never draw random samples to simulate analysis.
- macOS conventions: system font stack, native titlebar buttons, segmented workspace choice, keyboard focus and familiar sidebar/list/inspector structure. Remove remote web-font dependencies.

## Color rules

Deck A: blue `--deck-a`; Deck B: orange `--deck-b`. C: amber; D: green. Always pair identity color with deck letters and stable placement. Deck color must be consistent across panel edge, label, mapped control and loaded-track row marker. Do not use generic app accent or streaming-provider colors as competing deck identities.

Audio frequency palette is separate: blue low, amber mid, pale high; show measured bands as separate saturated layers on near-black signal backgrounds. Both main/deck waveforms and library thumbnails now carry real low/mid/high analysis. Waveform amplitude uses bounded track-wide display gain and a gentle nonlinear display curve to expose structure without changing audio. Meter warning/clipping colors retain their signal meaning. Amber cue affordances and green playing state are paired with symbols/text.

Color supports recognition and grouping; avoid unsupported claims that colors have universal psychological effects or that all DJ apps use one standard. Test readability, contrast and non-color cues.

## Interaction and performance

Performance/Browse layouts change information density; play/cue and primary mix controls remain reachable. Deck height follows content; More/Less reveals secondary controls, with outer workspace scrolling when necessary rather than cropping controls. Library rows use single selection, arrows/Home/End, Enter to focus details, and explicit load buttons. Removing from library lives under Library options; right-click must not silently remove a track.

Previews decode on a background Rust worker, separate from audio callbacks. Requests are visible-row lazy, serialized, cached with an estimated 12 MiB/128-entry budget, and prioritize selected track. Visible rows retain artwork for thumbnails; offscreen row hooks release visual results. Row priority stays below the selected inspector. Native image bitmap memory is separate from this estimated string cache. Preview limits: regular library files up to 256 MiB, 20 minutes, 96 kHz/eight channels; embedded PNG/JPEG <=2 MiB. Processing budget is cooperative between decode calls, not a strict timeout. Unavailable artwork/analysis must be labeled honestly.

## Primary guidance

[Apple Color](https://developer.apple.com/design/human-interface-guidelines/color): purposeful restrained color and redundant cues.
[Apple Sidebars](https://developer.apple.com/design/human-interface-guidelines/sidebars): navigation hierarchy and familiar collections.

## Follow-up opportunities

Continue native accessibility (custom knobs/faders need keyboard slider semantics), user-resizable columns/inspector, persisted visual preview cache and richer metadata persistence. These remain future work; do not report them implemented from this styling pass.

## Beat-grid readability (T005)

Main waveforms show each beat above the signal, using neutral dashed lines and top/bottom ticks. Every fourth interval starts a bar: a stronger coral line, triangles and bar.beat label (`92.1`). These markers use the existing analyzed first-beat anchor plus manual offset, assume4/4 and do not imply automatic musical downbeat/phrase recognition. Keep frequency colors separate from temporal grid markers.


## Physical controller reference (T006)

User requests physical DJ-controller organization as an ongoing design reference. Use the FLX4 photo/layout for spatial hierarchy: separate channel strips, TRIM → HI → MID → LOW → FILTER, CUE, channel fader, common crossfader. Preserve minimal macOS surfaces and deck identity; do not add decorative hardware textures. Move routing/curve/auto-mix/extra FX into explicit Options. Channel meters use discrete green/amber/red LED segments with dBFS labels and a red0dBFS clipping-risk indication; the five thresholds match current FLX4 feedback. Red is signal risk, not deck identity. Overall deck area follows content and scrolls instead of squeezing controls together.

## Deck hierarchy and available space (T008)

Use available deck height for fulltrack waveform detail. Jog wheel and BPM should be prominent (120px platter/19px readout in default layout); Play/Cue are primary48px-high targets and tempo has112px travel. Hotcue pads fill the remaining control width in two rows of four, with36px height; keep loop commands below in exactly two rows of five. Default Performance grid lets tempo span waveform and transport rows, avoiding blank lower deck space. Expanded/Browse/quad layouts retain density/scroll behavior. Waveform drawing uses measured CSS pixel dimensions, including its beat-label geometry, rather than stretching a fixed SVG viewBox on resize.

## Full controls and bounded waveform (T009)

User wants the full deck controls always shown; remove More/Less. Keep a compact two-column tempo area, primary fader/transport settings alongside secondary sync/key settings. Avoid a tall single stack forcing blank space beside other controls. Fulltrack overview above controls is90px high (30% reduction from128px, rounded); Browse/compact56px. SVG must be absolutely positioned within its sized overview so intrinsic aspect ratio cannot feed back into layout on resize. Preserve outer workspace scrolling for smaller windows and expanded effects.

## Plug-and-play defaults (T011)

Prefer automatic setup over repeated profile/port selection. Restore last active controller snapshot and compatible port; otherwise match a single known controller by profile port hints. Keep the editor as an optional customization tool and show actual native session state. Respect manual Disconnect for the current run, and do not guess between ambiguous controllers or choose an unrelated first keyboard. Apply this low-setup principle to future product flows where reliable detection is possible.


## Responsive tools and mixer clearance (T012)

Place effects to the right of beat-jump/roll tools whenever the deck control column is wide enough; wrap the whole effects group to a second row aligned with the common left edge of hot cues, loops and beat tools only when necessary (T013 correction). Keep space between Cue buttons, fader handles at maximum, meter labels and crossfader. A larger window should spread functional groups naturally rather than leaving effects crammed on the left. Song-list headers sort in either direction with a visible arrow and remember the preference; original order remains available through #.


## Single resizable workspace (T014)

Remove Browse/Performance tabs: one workspace keeps decks, mixer and library together. A subtle9px horizontal divider with row-resize cursor reallocates vertical space; persist the proportion. Pointer drag, keyboard Up/Down/Home/End and double-click reset work. Retain full-size controls through independent upper/lower scrolling, rather than squeezing their geometry. Reserve240px upper/160px library where the window allows. Prior Browse styling is obsolete; compact deck variants remain for quad mode.


## Album references in the list (T015)

Show26px square embedded album cover alongside title and a subdued album-name subtitle when tagged. Retain34px dense rows and existing title sorting/column geometry. Missing or failed art uses a music-note tile; missing album omits subtitle. Reuse visible-row lazy shared preview pipeline and bounded cache, no independent per-row decode requests. Images are decorative and not separately draggable; row selection/drag stays unchanged.


## Adaptive controls supersede scroll-only resizing (T016)

Normal A/B controls must remain visible as the upper pane shrinks. Use container-height dimensions with lower/upper bounds for waveforms, jog, Play/Cue, hotcues, spacing and fader travel; retain readable text and primary target priority. Reserve500pxupper +160pxlibrary when available, rather than allowing unusably small performance panes. Extremely short windows and expanded extras/quad can still scroll. Use actual rendered fader length for pointer mapping. T014's fixed-size scroll-only normal deck behavior and240pxminimum are superseded. T00990pxoverview is now a cap, adaptive down to40px. Do not scale the whole interface as an image.
