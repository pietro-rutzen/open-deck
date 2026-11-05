# OpenDeck project progress

Updated: 2026-11-05 (Europe/Helsinki). This is the authoritative project resume index; task records contain detailed claims and evidence.

## Current status

Active development in `/Users/pietrorutzen/Projects/OpenDeck`, based on Compás commit `102d2d3b6fd049efcd670c5a11bcc304ee726612` (MIT; upstream LICENSE retained). User authorized starting the prototype and placing it in Projects. Workflow kit installed; conflicting upstream docs preserved in `docs/upstream-baseline/`.

Fast prototype strategy: reuse the complete existing Compás application, then adapt incrementally. Source layout is `apps/compas-dj/frontend`, `apps/compas-dj/src-tauri`, and `crates/compas-*`; the original architecture document is a proposal, not the actual paths. No AI product features are being added.

Spotify mixing is blocked on partner access: public Spotify developer tools prohibit mixing/overlap. No public-API workaround or Spotify audio extraction is implemented. Local-file/controller prototype proceeds. See `docs/SPOTIFY_FEASIBILITY.md`.

## Active work and next action

The local app has been used at a party. User reported a LOAD failure after extended use; T025 fixes identified selection races and a silent hardware-only playing gate, but the exact long-session incident has not been reproduced. T026 user-confirmed unplug/replug, browse and LOAD work without relaunch (2026-11-04). This is a limited hardware pass, not complete M5/M6 qualification.

Current source baseline: aefba51 plus T025–T031 reliability changes in this delivery. Native prototype supports local files, controller mapping/LEDs, master/cue routing and responsive library/decks. No production release has been qualified.

| Priority | Next gate | Status / evidence |
|---|---|---|
| 1 | Repeatable reliability regression checks | T027 in progress: real offline mixer stress plus library/marker CI tests |
| 2 | Bounded command/load admission and state recovery | Audit remaining unbounded control queue, decoder concurrency and device-restart state; fix with explicit failure behavior |
| 3 | Measured hardware endurance and latency | At least 60-minute real two-deck session with analysis/controller activity; capture dropout counters and separate physical loopback latency |
| 4 | Reproducible release candidate | Green hosted CI, toolchain/platform matrix, audit callback risks and dependency/license provenance |
| 5 | Distribution | Signed/notarized build, clean-Mac installation, tested recovery instructions, checksums matching tested revision |

Do not add features ahead of outstanding reliability failures. Full historical milestones below retain their strict acceptance criteria; implemented features alone do not close multi-part gates. Latest completed coding task: [T037](tasks/T-037.md); T027 audio-test review and lint gates remain open. Recent handoffs: [T025](handoffs/T-025-20261103-01.md), [T026](handoffs/T-026-20261104-01.md).

## Milestones and acceptance gates

Checkboxes mean acceptance passed on an integrated revision with linked evidence. Work in progress, blocked, mocked or untested remains unchecked. Listed owners are accountable roles; assignment happens in task records.

### M0 — Development workflow foundation

Owner: orchestrator. Reviewer: docs-release. Dependencies: none.

- [x] Root AGENTS.md, orchestrator and eight specialist role instructions authored.
- [x] Workflow, task/handoff/ADR/verification templates and checkbox plan authored.
- [x] Primary-source option comparison and recommended portable approach documented.
- [ ] Maintainer adopts ADR-0001 and merges kit into the actual OpenDeck repository.
- [ ] A pilot assignment demonstrates role loading, path claims, independent review and resume from a saved handoff in the chosen harness.

Acceptance: all items above; evidence must identify repository revision and pilot task/handoff. Documentation completion does not close M0 adoption.

### M1 — Engine and product feasibility

Owner: architect-planner. Reviewers: backend-rust + audio-dsp + macos-midi. Depends: M0 adoption; pilot may run using this investigation.

- [ ] Record exact Compás URL, revision, license/notice inventory, dependency/assets audit and upstream patch policy.
- [ ] Build pinned upstream on target macOS; record actual commands, CPU, OS and results.
- [ ] Audit callback call graph and reusable core boundaries; document extraction/fork decision with API evidence.
- [ ] Confirm two-deck MVP, minimum supported macOS/CPU targets and explicit non-goals in an accepted ADR.
- [ ] Define command/state/error schema, clocks, queues, ownership and resource lifetime contracts.
- [ ] Inventory FLX4 hardware/firmware availability and capture actual audio/MIDI capabilities or record an explicit blocker.

Acceptance: accepted engine strategy ADR, reproducible local-file playback spike, callback risk report and agreed interfaces. Do not equate an upstream README feature list with OpenDeck verification.

### M2 — Repository scaffold and reproducible checks

Owner: backend-rust. Reviewers: frontend + qa-test + docs-release. Depends: M1.

- [ ] Establish actual workspace layout, Rust/Node toolchain pins, lockfiles and documented setup commands.
- [ ] Build Tauri 2 shell with React/TypeScript and a typed Rust command/state round-trip.
- [ ] Add engine facade boundary and legal, reproducible test fixtures.
- [ ] CI runs Rust format/lint/tests, frontend type/lint/tests and macOS desktop build with required checks.
- [ ] Verify product manifests/bundle contain no AI runtime or model service functionality.

Acceptance: fresh-checkout documented build and green required CI on an integrated revision; exact commands now recorded in contributor docs. No claims that this kit already created CI.

### M3 — Audio vertical slice

Owner: audio-dsp. Reviewers: backend-rust + qa-test. Depends: M2.

- [ ] Load and play a local file; stop/seek and recover from invalid/unsupported files.
- [ ] Two independent decks, gain/EQ/crossfader and separate master/cue buses work against deterministic fixtures.
- [ ] Validate bounded queues, overflow behavior, preallocation and off-callback reclamation/shutdown.
- [ ] Record offline DSP tolerances plus callback audit and instrumented stress results.

Acceptance: reproducible engine integration tests, safe output on load failures, no known callback allocation/blocking path; physical device routing remains an M5 gate.

### M4 — Performance UI and library

Owner: frontend. Reviewers: ui-ux + qa-test. Depends: M2 contracts; completion depends on M3 real engine.

- [ ] UX specification covers deck focus, load/play/cue, waveform, library, routing and recovery states.
- [ ] UI uses engine state; pending/rejected commands and stale events reconcile correctly.
- [ ] Local library/import, waveform rendering and essential keyboard/controller-visible states work.
- [ ] Type/interaction tests, accessible labels/focus and actual Tauri engine flow pass.

Acceptance: a tester performs a two-deck mix from local-file import through cue/master monitoring using the integrated app; mocks alone cannot close M4.

### M5 — macOS routing and DDJ-FLX4

Owner: macos-midi. Reviewers: audio-dsp + qa-test. Depends: M3; end-to-end UI/feedback depends on M4.

- [ ] Record FLX4 model/firmware/mode, macOS, CPU, sample rate, buffer and channel enumeration.
- [ ] Prove independent headphone cue and master routing with physical listening/capture evidence.
- [ ] Capture/provenance/version mapping for transport, jog touch/rotation modes, pitch, faders, EQ, cue and LED feedback; document unsupported controls explicitly.
- [ ] Verify input validation, queue saturation handling and feedback without duplicate/stuck controls.
- [ ] Exercise unplug/replug, sleep/wake, device/rate change and safe silence/recovery.

Acceptance: independent hardware report and tested mapping coverage matrix; unavailable hardware means blocked, not done. MIDI success does not imply audio routing success.

### M6 — Reliability and performance qualification

Owner: qa-test. Reviewers: audio-dsp + macos-midi. Depends: M4 + M5.

- [ ] Accept a test matrix for supported macOS/CPU/device configurations; each claimed configuration has evidence.
- [ ] Run at least a 60-minute two-deck session with library analysis, UI activity and MIDI stress; record xruns, callback durations and audible artifacts.
- [ ] Proposed target: zero observed dropouts/xruns at 48 kHz/256 frames on the designated reference Mac; accept/revise this target in an ADR before qualification.
- [ ] Measure loopback latency separately; record method/distribution and establish an accepted latency budget before claiming performance.
- [ ] Test load/cancel races, invalid IPC/MIDI, device loss and shutdown; resolve release-blocking failures and rerun affected cases.

Acceptance: approved target matrix/budgets, reproducible passing reports on release candidate and explicit known limitations. A 60-minute pass is a qualification sample, not a guarantee of all-session reliability.

### M7 — Open-source macOS release

Owner: docs-release. Reviewers: qa-test + macos-midi. Depends: M6.

- [ ] Contributor/user docs, actual build/test instructions, controller mapping guide and handoff index are current.
- [ ] Engine pin/patch provenance, source license, third-party notices and fixture rights reviewed.
- [ ] Produce intended architecture packages; sign/notarize/staple where applicable and test clean-machine installation with normal Gatekeeper behavior.
- [ ] Match release artifact checksums to tested revision; publish tested support matrix, limitations, changelog and recovery instructions.
- [ ] Maintainer performs or authorizes release publication; archive release evidence and next-version backlog.

Acceptance: installable verified artifact and matching source/evidence. Never recommend disabling platform security as the normal install procedure. Signing credentials stay outside Git.

## Risks / blockers

| ID | Risk or unknown | Unblock owner | Next evidence |
|---|---|---|---|
| R1 | Compás core may require maintained extraction/fork | architect-planner | Pinned build, dependency and API audit in M1 |
| R2 | Upstream real-time claims are unverified here | audio-dsp | Callback audit plus instrumented target build |
| R3 | FLX4 channel layout, mapping and firmware behavior unverified | macos-midi | Actual enumeration, captures and physical tests |
| R4 | Release credentials and clean-machine installation not assessed | orchestrator | Inventory in actual repository/session |
| R5 | Parallel agents can conflict on contracts/shared files | orchestrator | Exclusive claims and sequential integration records |

## Decision log

| ID | Status | Decision / constraint | Authority / rationale | Record |
|---|---|---|---|---|
| C-001 | required | No AI functionality in DJ product | User scope | AGENTS.md |
| C-002 | required | macOS, Rust/Compás, Tauri 2, React/TypeScript, DDJ-FLX4 | User scope; feasibility still to verify | M1 |
| ADR-0001 | proposed | Portable Markdown roles; one orchestrator; existing coding harness | Recommended to minimize infrastructure and support handoffs | [ADR](decisions/0001-agent-workflow.md) |

## Handoff index and update rules

Product handoffs are in docs/handoffs; the active-work section and each task entry link the latest evidence. Historical notes below describe their original checkpoints.

Orchestrator is the single writer unless an exclusive docs-release assignment is recorded. Update after claim/status/review/integration/blocker changes and session end. Include owner/session, reviewer, revision, evidence and next action. Only check criteria with actual evidence. Reopen invalidated items with reason. Preserve stable IDs and historical failures. Detailed protocol: [workflow](AGENT_WORKFLOW.md); reusable records: [task](templates/TASK.md), [handoff](templates/HANDOFF.md), [verification](templates/VERIFICATION.md), [ADR](templates/ADR.md).

## Prototype decision update

[ADR-0002](decisions/0002-fast-prototype.md): accepted reversible full-app reuse for speed, based on actual pinned source. User authorized starting work in Projects. ADR-0001 workflow is being used; native runtime pilot and full milestone gates remain open.

Latest handoff: [T-001 prototype](handoffs/T-001-20261025-01.md). App is open; hardware acceptance remains pending.

## T-002 — Visual design implementation

Owner: orchestrator; frontend-library specialist and backend specialist supplied isolated changes; controller_audit independently reviewed. User requested clean macOS style, deck color identity, VirtualDJ browser/inspector, rekordbox waveform styling and list thumbnails.

- [x] Native window controls, system fonts, neutral graphite surfaces; blue A / orange B with letters.
- [x] Sidebar + selectable list + right artwork/metadata inspector; explicit deck load actions.
- [x] Real lazy waveform thumbnails; streamed background analysis and embedded artwork/tags.
- [x] Five backend preview tests pass; frontend builds and native bundle pass.
- [x] Live native view shows imported tracks, real M4A embedded art/album/artist and amplitude overview.
- [x] Independent code review; compact transport, byte-budget and inspector-priority findings addressed.
- [ ] Final layout interaction matrix across two/four decks and small window sizes fully qualified.

Design source: [direction](design/DESIGN_DIRECTION.md). Detailed work/evidence: [T-002](tasks/T-002.md). Hardware verification remains separate from this design task.

## T-003 — User feedback refinements

- [x] Strong saturated layered soundwave colors, real frequency-aware library previews and RMS/peak structure.
- [x] Faster native-rate90s background analysis + optimized dev DSP + selected-track priority + immediate cache refresh events.
- [x] Content-driven decks, explicit More/Less, square defaults; no silently cropped deck controls.
- [x] 35 native tests and frontend display checks pass; independent code review resolved.
- [x] Final native visual verification at 1440×900: square controls, full tempo/transport/mixer visibility, real colored library previews and deck waveform.
- [ ] Extended keyboard/More/Browse/quad interaction qualification remains open.

[Task](tasks/T-003.md), [timing evidence](testing/BPM_LATENCY.md). Measurements are microbenchmarks, not UI/IPC timing. T-001 hardware gates remain open.

Latest handoff: [T-003](handoffs/T-003-20261025-01.md). Updated prototype is open.

## T-004 — FLX4 physical controls

- [x] User confirms physical left PLAY starts transport and audible playback.
- [ ] Correct mixer mapping and add native jog support; hardware retest pending.

Owner: orchestrator; frontend and native specialists. [Task](tasks/T-004.md).

T004 update: corrected mixer mapping/native jog implemented,41 app tests + core/audio tests + frontend/native builds pass. Updated app reopened andFLX4 reconnected. Physical mixer/jog response and independent final review remain unchecked. [Handoff](handoffs/T-004-20261025-01.md).

## T-005 — Mixer LEDs and beat divisions

User reports most T004 controls work; individual qualification still pending. Native meter output and stronger four-beat grid in progress. Owner: orchestrator. [Task](tasks/T-005.md).

T005:42 app tests/frontend/native builds pass; stronger beat divisions visually verified; updated app reopened andFLX4 connected. Physical meter confirmation pending. [Handoff](handoffs/T-005-20261025-01.md).

## T006 — Physical-controller design reference

- [x] Clear trim/EQ/filter channel strips; extra routing/FX/auto/curve under Options.
- [x] Five-segment green/amber/red meters with dBFS labels match FLX4 thresholds.
- [x] Frontend/native builds and1440×900 visual/Options check pass; FLX4 reconnected.

User confirms T005 physical meters react. Future design should follow physical controller organization while retaining minimal macOS surfaces. [T006](tasks/T-006.md).

## T-007 — Vinyl jog audio refinement

- [ ] Smooth jog audio and silence stationary sample reads; owner root, DSP reviewer /root/scratch_review.
- [ ] FLX4 listening acceptance and high-speed resampling/sensitivity qualification.

[Task](tasks/T-007.md). Beat markers use four-beat cadence but do not yet detect the true musical bar start; rekordbox downbeat parity is not claimed.

T007 first increment: velocity smoothing and stationary/boundary silence implemented;76 engine/core tests, independent DSP review and native bundle pass. App restart/listening pending. Release transitions, high-speed anti-aliasing and measured jog calibration remain open. [Handoff](handoffs/T-007-20261026-01.md).

## T-008 — Deck space and waveform resize

- [x] Taller fulltrack waveforms, larger jog/BPM and primary transport, fullwidth hotcues and two loop rows; measured viewport fixes SVG label distortion.

Owner root; read-only UI reviewer /root/deck_layout_review. [Task](tasks/T-008.md).

T008 complete for requested layout: final native/frontend build22:21UTC, independent UI review and desktop visual checks pass.1440×900/1280×810 resize checked; Browse/More checked, extended quad matrix open. App reopened with both prior tracks paused and FLX4 reconnection/activation issued. This also loads T007 smoothing code, whose physical listening acceptance remains open. [Handoff](handoffs/T-008-20261026-01.md).

## T009 — Overview height regression / full controls

- [x] Fix overgrown overview after resize and make full controls default (user request).

T008 layout acceptance reopened by user screenshot: unbounded overview/intrinsic SVG sizing can push controls away. Owner root; read-only reviewer /root/deck_layout_review. [Task](tasks/T-009.md).

T009 corrected: full controls default, no More/Less, tempo settings placed alongside primary fader, overview intrinsic-layout feedback removed. Final90px overview (user requested30% smaller),56pxcompact/Browse. Frontend/nativebuild22:36UTC and loadedvisualcheckpass; independentreviewpasses, repeatresizeprior128pxvariantstable. Extendedquadmatrix remainsopen.

## T010 — Embedded artist/title tags

- [x] Shared local source reads real tags for deck/newimport; repair existing placeholder library metadata without changing cues/analysis/history.

Ownerroot; reviewer/root/metadata_review. [Task](tasks/T-010.md). UserreportsSerato/rekordboxreadtagswhiledeckshowsUnknown; identifiedhardcodedplaceholderinsourceloader.

T010 complete: shared embedded Artist/Title/Album tags + AlbumArtist fallback, safe persisted-placeholder repair.7source+43apptests,independentbackendreview,native/frontendbuild22:42UTC,diffcheckpass. RealStardust/JessieWaretracksverifiedindecks,waveformheadersandpersistedlibrary;9tracksretained. FLX4connectedand26bindingactivationconfirmed;deckspaused. [Handoff](handoffs/T-010-20261026-01.md). T00990pxfullcontrolslayoutincluded. Spotifyapprovalresearchremainsunsubmitted;userdefersstreamingandwilltestlocalfiles.

## T011 — Plug-and-play controller setup

- [x] Restore last controller/profile and auto-match known devices on startup.
- [ ] Qualify physical USB unplug/replug, sleep/wake and error recovery (native reconnect implemented, cable test not run).

Owner root; native selection and frontend specialists with disjoint files. [Task](tasks/T-011.md).

T011 startupverifiedonactualFLX4withLaunchkeyalsoconnected:firstautomaticmatch+26bindings,secondrememberedstartup,manualDisconnectpausedandmanualConnectretainedprofile.52apptests,frontendracechecks,native/frontendbuild22:56UTC,independentbackend/macOS-MIDI/frontendreviewsandformat/diffchecksPASS. AppleftopenFLX4connectedtwoprevioustrackspaused. Physicalhotplug/sleepwakequalification remainsunchecked. [Handoff](handoffs/T-011-20261026-01.md).

## T012 — Song-list sorting

- [x] Clickable columns with ascending/descending and remembered preference across library/search/crate views.

Owner root; frontend reviewer /root/deck_layout_review. [Task](tasks/T-012.md).


T012 complete: numeric/natural stable sorting, missing-last and path-safe keyboard focus; responsive right-aligned effects and mixer fader clearance. Frontend/native builds, helper edge assertions, independent UI review and diff check PASS. Actual Title toggles retained selection; saved descending sort survived final restart. Final app open with prior A/B tracks paused; user library20 retained. Wide-screen same-row effects branch code-reviewed, native wider-screen check NOT RUN. [Handoff](handoffs/T-012-20261026-01.md).


## T013 — Wrapped effects alignment

- [x] Align wrapped effects with the controls above while preserving right placement on a shared row.

Owner root; frontend reviewer /root/deck_layout_review. T012 narrow-layout acceptance reopened by user screenshot. [Task](tasks/T-013.md).


T013 complete: removed effect auto-margin that offset wrapped row. Native1440×900↔1280×810 check, frontend/nativebuild, independent UI review and diffcheck PASS. Shared-row right placement code-reviewed; larger native display NOT RUN. [Handoff](handoffs/T-013-20261026-01.md).


## T014 — Single workspace

- [x] Remove Browse/Performance tabs and retain full decks/mixer/library workspace.
- [x] Add persisted draggable/keyboard divider between decks/mixer and library, with independent scrolling.

Owner root; reviewer /root/deck_layout_review. [Task](tasks/T-014.md).


T014 complete: single workspace and persisted split divider. Native drag, keyboard/bounds/reset, independent scroll, window resize and restart persistence PASS; final frontend/native build, independent UI review and diffcheck PASS. App open with current tracks paused. [Handoff](handoffs/T-014-20261026-01.md).


## T015 — Track-list album art

- [x] Add album-art thumbnails and album names beside titles with missing-art fallback and existing lazy bounded loading.

Owner root; reviewer /root/deck_layout_review. [Task](tasks/T-015.md).


T015 complete:26px art plus album subtitle in existing34px rows. Final frontend/native build, independent UI review, diffcheck and native1440/1280 visuals/sorting/selection PASS. [Handoff](handoffs/T-015-20261026-01.md).


## T016 — Responsive control sizing

- [x] Scale controls with pane height and constrain split to keep normal A/B controls visible.

T014 scroll-only behavior superseded by user request. Owner root; reviewer /root/deck_layout_review. [Task](tasks/T-016.md).


T016 complete: bounded fluid control/waveform sizing and usable split limits, measured fader travel. Final build/review/diffcheck PASS; loaded native1280/1440 min/max checks show normalA/B controls visible without scroll,10px tempo drag on80pxtrack gives2.0%,reset0.0%. Quad/expandedFX/Options matrix NOT RUN. [Handoff](handoffs/T-016-20261026-01.md).


## T017 — Controller integration and audio routing

- [ ] Diagnose root causes and implement reusable capability-based master/headphone routing with clear status.
- [ ] Verify isolated headphone PFL and controller flow on actual hardware.

Owner root; backend/macOS-MIDI/DSP/frontend specialists. [Task](tasks/T-017.md).

T017 integrated: explicit device/channel headphone routing, verified FLX4 3/4 preset, true pre-fader monitor bus, native PFL state/LEDs and safe resource retirement. Audio67 PASS; final build/review/listening pending. Audient master + FLX4 phones is the actual target. Universal mapping/clock-drift/device-recovery qualification remains open.

T017 final build +121 Rust tests PASS. Native FLX4 channels3/4 output opened and ON; physical headphone isolation/LED listening result pending. [Handoff](handoffs/T-017-20261026-01.md).

## T018 — FLX4 workflow audit

- [x] User confirmed T017 isolated headphone Cue and latched CUE LEDs.
- [ ] Audit every FLX4 control/feedback path and close normal-DJ-flow gaps.
- [ ] Final integrated tests, independent review and physical qualification.

Owner root. [Task](tasks/T-018.md), [control inventory](testing/FLX4_FUNCTIONALITY_AUDIT.md). Rim bend, native high-resolution crossfade, added pads/loops/library, profile upgrade and LED lifecycle integrated pending final build. Proprietary/advanced modes and native migration still open; not party-certified.

T018 checkpoint:126 Rust tests and fullbuildPASS; launched07:24UTCnativebuild, FLX4autoactive108bindings verified, phones3/4ON, bothdecksrestoredpaused. Fullcoverage, physicalnewcontrols and independentreview remain open. [Handoff](handoffs/T-018-20261026-01.md).

## T019 — FLX4 plug-and-play and set performance

- [x] Native engine path for faders/trim/EQ/filter/tempo/PLAY/CUE/start; coalesced UI mirror; raw MIDI only when unbound or learning; diffed LED feedback; meter re-render isolation.
- [x] Press-time positions for hot cue/loop/jump; sync leader, reloop-stop, 4-beat hold, key shift, sampler, Beat FX, browse focus, waveform zoom (187 bindings).
- [x] Independent review findings M1–M3 and recommended minors fixed (2026-10-26). Nits left open. No second review of the fixes.
- [ ] Physical FLX4 qualification and full-length mix.

Owner root. [Task](tasks/T-019.md), [handoff](handoffs/T-019-20261026-01.md), [control inventory](testing/FLX4_FUNCTIONALITY_AUDIT.md). Follow-up: audio lib 69 PASS, app lib 68 PASS, `tsc --noEmit` PASS, clippy has the same pre-existing warnings, `./scripts/opendeck-build.sh` PASS. Hardware NOT RUN.


## T020 — Beginner hierarchy and responsive polish

- [x] Clearer primary transport, monitoring and secondary-control hierarchy.
- [x] Preserve preferred large waveform, yielding space only in constrained panes; consistent drag/keyboard/accessibility geometry.
- [x] Independent review, TypeScript/Vite/native build and native wide/short-pane inspection.
- [x] Reopen final build for local-file practice.

Owner root; reviewer deck_layout_review. [Task](tasks/T-020.md), [handoff](handoffs/T-020-20261028-01.md). No audio/controller logic changes; T019 physical qualification remains open.

## T-021 — Controller-thread cue, loop, jump, and sync

Hot cue, beat loop, manual loop, beat jump, quantize, and sync now enqueue engine commands on the controller thread, using the press-time playhead. Sampler trigger and stop do too, at the same velocity as an on-screen pad. The UI still owns mouse edits, key shift, and Beat FX. `compas-dj` lib tests cover the sampler press. Physical listening against a reference player is NOT RUN.


## T022 — macOS desktop CI coverage

- [x] Add explicit desktop app tests and a locked Tauri macOS bundle build to pull-request/main CI.
- [x] Frontend and native bundle builds pass locally; independent workflow review and YAML validation pass.
- [ ] Confirm hosted macOS run: local tests report 98 PASS, one Trash-permission failure; test remains enabled.

Owner root; reviewer deck_layout_review. [Task](tasks/T-022.md), [handoff](handoffs/T-022-20261029-01.md). No application edits or release publishing.


## T023 — FLX4 responsiveness and sampler loading

- [x] Audit native loop/sampler paths; fix NoteOff release handling and tiny-loop adjustment panic.
- [x] Off-UI bounded sampler loading, admission control before worker scheduling, and visible errors.
- [x] 103 app tests pass (1 known Trash-permission test excluded locally), frontend/native build and independent review pass; native valid/invalid WAV loading verified.
- [ ] Physical FLX4 latency, looping and sampler listening qualification.

Owner root; controller specialist controller_audit; reviewer deck_layout_review. [Task](tasks/T-023.md), [handoff](handoffs/T-023-20261029-01.md).

## T024 — Saturday speaker routing

- [x] Choose speakers in the app. The default follows the Mac’s current output. DDJ-FLX4 master 1/2 with headphones 3/4 is an explicit choice.
- [ ] Physical party rehearsal: FLX4 master plus headphones, and laptop-output playback. NOT RUN.

Owner root. [Task](tasks/T-024.md), [handoff](handoffs/T-024-20261030-01.md). No latency or hardware certification. Beat FX and the listening pass remain open.

## T025 — Post-party LOAD reliability and scrolling hot cues (2026-11-03)

User reports party mostly successful, but hardware LOAD stopped even with destination paused; drag/drop still worked. This is a real unresolved hardware incident, not a passed soak test.

- [x] Remove hardware-only silent playing lock; controller LOAD follows existing UI replacement behavior.
- [x] Replace stale browser indexes/conflicting selection identities with one synchronous stable key; protect against delayed focus reversal; visible selection/listener errors.
- [x] Add scrolling numbered cue markers sharing the overview/pad palette, with source-rate/tempo/resize-aware placement.
- [x] Selection regression script (10,000 refresh/reorder cycles), marker projection tests, frontend build and 126 native tests pass; one known local Trash-permission test excluded.
- [x] Final bundle and native marker visual verification; app reopened paused.
- [ ] Extended physical FLX4 load session (controller now connected; short reconnect/LOAD check passed).

Owner root; implementation load_failure_audit (library), root (native/markers); review load_fix_review. [Task](tasks/T-025.md). No claim of exact incident reproduction or long-session reliability from unit tests.

T025 [handoff](handoffs/T-025-20261103-01.md): existing saved cue visibly confirmed in scrolling waveform at 32s zoom. Exact incident reproduction remains open.

## T026 — Persistent MIDI discovery

- [x] Found and removed per-second MIDI client creation; initialize reusable discovery on main thread before MIDI workers.
- [x] Independent review, 127 native tests, bundle and native launch (one known local Trash test excluded).
- [x] User confirmed unplug/replug, browse and LOAD without relaunch on 2026-11-04 (limited hardware check).

Owner root; reviewer hotplug_audit. [Task](tasks/T-026.md). T025 user connected FLX4; restart visibly restored MIDI connection and headphones. That startup observation preceded the later user-confirmed unplug/replug pass.

T026 [handoff](handoffs/T-026-20261104-01.md): 2026-11-04 build opened; user subsequently confirmed unplug/replug, browsing and LOAD without restart.

## T027 — Repeatable reliability qualification

Owner root (took over audio tests after specialist usage limit); reviewer reliability_review approved CI wiring, audio review pending. [Task](tasks/T-027.md), [handoff](handoffs/T-027-20261104-01.md).

- [x] Offline hour PASS: 7,200 deck replacements in 125.25 seconds; synthetic sample time only.
- [x] Default offline mixer regression: repeated replacement, PFL/faders, pause, looping and buffer ownership.
- [x] Library selection and cue-marker tests wired into npm test and CI.
- [x] Frontend test/typecheck/build; 92 engine and 127 native tests passed (one known local Trash test excluded).
- [ ] Independent audio-test review; specialist capacity exhausted this session.
- [ ] Strict lint/hosted CI green: existing audio Clippy warnings remain blockers.
- [ ] Physical session/latency qualification remains open.

Next: clear strict lint blockers, independently review the harness, then bound simultaneous full-track decodes to protect memory during rapid LOAD use.

## T028 — Bound rapid deck loads

Owner root/backend; independent reviewer load_admission_review. [Task](tasks/T-028.md), [handoff](handoffs/T-028-20261104-01.md).

- [x] Single process-lifetime decoder, one latest pending request per deck, round-robin service.
- [x] Stale queued requests skip decoding; generation guards preserve latest-only publication.
- [x] Panic isolation and latest-only error handling keep later loads serviceable.
- [x] Backend independent review found no blockers.
- [x] Final integrated tests: 129 PASS (one known local Trash exclusion), desktop bundle PASS; see handoff.

This bounds simultaneous deck jobs, not individual track size or upstream engine messages. Current decode cannot be interrupted. No new hardware qualification claimed.

## T029 — Track PCM limit and cooperative cancellation

Owner root/backend; reviewer load_admission_review. [Task](tasks/T-029.md), [handoff](handoffs/T-029-20261104-01.md).

- [x] Actual decoded deck PCM capped at 512 MiB; no duration-based allocation.
- [x] Superseded loads stop between source reads; stale failures remain suppressed.
- [x] Geometric fallible allocation, rate/empty/stereo validation, additional analysis-stage generation checks.
- [x] Independent backend review PASS; 134 native tests PASS (one known local Trash exclusion).
- [x] Final desktop bundle PASS; evidence recorded in handoff.

Limit scope: PCM accumulator only; codec internals, analysis arrays, total RSS and uninterruptible I/O remain outside this guarantee. Next reliability gate: bounded upstream engine command admission and recoverable stream state; T027 independent audio-test review/strict lint and physical endurance remain open.

## T030 — Grouped track load commands

- [x] Submit track audio, replay gain and pitch reset in one channel message.
- [x] Record current and peak queue depth. Closing the receiver clears the reported live backlog.
- [x] Backend and DSP reviews, native tests and desktop build passed on 2026-11-05.
- [ ] Protect paired press/release commands and remaining multi-command operations before enforcing an upstream capacity limit.

[Task](tasks/T-030.md) · [Verification and handoff](handoffs/T-030-20261105-01.md)

## T031 — Held controls on controller disconnect

- [x] End native deck CUE/reverse holds when an input closes or a mapping changes.
- [x] Preserve new track playback if a gated cue release arrives after its old hold ended.
- [x] Controller/mixer regression tests and DSP review passed.
- [ ] Physical held-button disconnect check.

[Task](tasks/T-031.md) · [Handoff](handoffs/T-031-20261105-01.md)

## T032 — Loop roll across track replacement

- [x] Clear old loop bounds and slip state on deck load and eject.
- [x] Exercise both paths with reloop, seek, a new regular loop and late roll release.
- [x] Audio tests, DSP review and desktop build passed.

[Task and handoff](tasks/T-032.md)

## T033 — RGB frequency waveforms

- [x] Shared RGB coloring for previews and playing waveforms, including phase-safe stereo band analysis. [Task and handoff](tasks/T-033.md)

## T034 — Brighter CDJ-style waveform previews

- [x] Tune frequency hues against the supplied recording and lift quiet preview passages. [Task and handoff](tasks/T-034.md)

## T035 — Mixed frequencies in real-track previews

- [x] Corrected the display balance for lower-energy treble and blended bass/treble through violet and lavender. Checked on two local tracks. [Task and handoff](tasks/T-035.md)

## T036 — Waveform structure overlay

- [x] Preserve RGB timbre and show changes in measured band energy across all waveform views. [Task and handoff](tasks/T-036.md)

## T037 — Centered frequency core and desktop restart

- [x] Cap the centered bands at 30% of local waveform height. Build, install, then restart by default. [Task and handoff](tasks/T-037.md)
