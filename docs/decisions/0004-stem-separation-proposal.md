# ADR-0004 — Stem separation: offline proposal, no implementation

- Status: proposed. An offline classical prototype was later approved; see ADR-0005. This proposal's model gate is unchanged.
- Date: 2026-11-07
- Author / accountable role: Architect
- Decision authority / review evidence: Owner review required. This document does not accept a model, a runtime, or a product feature.
- Related task / milestone: Stems technical proposal after the file-format and Pad FX1 milestones. No task id is opened here.
- Supersedes / superseded by: none. It does not reverse the 2026-06-24 removal of the earlier Demucs / ONNX stem path recorded in `ROADMAP.md`.

This is an engineering proposal, not legal advice and not a measurement report. Sources were read on 7 November 2026 (Europe/Helsinki). Licenses change; the linked texts are the authority.

On 7 November 2026 at 11:29 Europe/Helsinki the owner approved an offline classical-DSP prototype with no model weights and no playback change. That work is ADR-0005. It does not lift the model gate below, and it does not make harmonic/percussive separation a product stem.

## Approval gate

Open Deck does not add an AI or ML product feature, inference dependency, model download, or stem runtime until the owner explicitly lifts that constraint. This proposal states the gate. It does not implement stems, download weights, add a dependency, change the UI, or run a benchmark.

## Context and constraints

A DJ stem control mutes or solos vocals, drums, bass, and the remaining instruments inside a track that is already playing. The current engine plays one decoded mix per deck. `docs/DECK-GRAPH.md` reserves a future immutable snapshot of several stem buffers, and its first-refactor non-goal still says a stem graph waits until the mono/stereo source path is split cleanly. `deck_loading.rs` refuses a decode above `MAX_PCM_BYTES` (512 MiB). Audio callbacks stay allocation-free, lock-free, and free of I/O and IPC. Library decode already cancels between chunks and discards a cancelled result.

The owner’s machine for a later measurement is an Apple M4 Pro with 24 GB of memory. This document has no run on that machine. Published speed and memory figures below stay attached to the machine in the source. They are not Open Deck results. There is no latency, dropout, or thermal evidence for stems.

The DDJ-FLX4 audit does not list a stem mode. Hot cue, Pad FX, beat jump, sampler, keyboard, beat loop, and key shift keep their current behavior. A stem control is a software control in this proposal.

## What traditional DSP can and cannot do

These methods use fixed signal processing. They are outside the AI approval gate. They also do not produce the four musical stems a DJ pad is expected to isolate.

| Method | What it separates | Limit for this product |
| --- | --- | --- |
| Mid/side vocal cancel | Energy common to left and right, which is often a centered vocal | A stereo vocal, a doubled vocal, or a centered bass fails the assumption. The result is not a vocal stem plus a residual band mix. |
| Harmonic/percussive median-filter split (HPSS) | Tonal energy versus transient energy | Drums are not only transients, and bass sits with vocals and synths on the harmonic side. The split is two streams, not vocals/drums/bass/other. |
| Unsupervised factorization (NMF and similar) | Repeating spectral patterns, if the user tunes rank and iterations | No stable four-stem vocabulary, no bounded product runtime, and no quality bar this repo has measured. |

Four-way music separation of the kind in MUSDB (vocals, drums, bass, other) is a learned-model problem in every system surveyed below. Shipping an HPSS button under a Stem label would misstate the audio. This proposal does not add that button.

## Model options

Code license and weight license are separate. An MIT `LICENSE` file for the repository is a grant for the source code in that repository. It becomes a grant for the weight files only when the publisher says those files are included.

| Option | Code | Weights | Maintenance | Role in this proposal |
| --- | --- | --- | --- | --- |
| Hybrid Transformer Demucs (`htdemucs`, `htdemucs_ft`) | MIT, stated in the repo `LICENSE` line of the README | Research / scientific use, stated by the author. A standard SPDX identifier was not found | Archived upstream repo; author says it is unmaintained | Quality reference only. Blocked for a redistributed app |
| Open-Unmix `umxhq` | MIT, repository license line | Weight archive tagged MIT on Zenodo. Training set is MUSDB18-HQ, which is educational / non-commercial | Reference implementation. README: simplicity over performance, a baseline. Torch 2 note of 16 April 2024 says results moved | Only measurement candidate, and only after the dataset-rights question below is decided. Not cleared for shipping |
| Open-Unmix `umxl` | MIT code | README: CC BY-NC-SA 4.0, non-commercial | Same project. Default model in the README | Out. Non-commercial weights and a share-alike term |
| Open-Unmix `umx` | MIT code | Trained on AAC-compressed MUSDB18, bandwidth limited to 16 kHz by the README | Same project | Out. Narrower audio than `umxhq` |
| Spleeter 2 / 4 / 5 stems | README: “The code of Spleeter is MIT-licensed.” | `paper.md` says source and pre-trained models are MIT. The README does not say that. Issues [#898](https://github.com/deezer/spleeter/issues/898) and [#957](https://github.com/deezer/spleeter/issues/957) ask for confirmation; the pages reviewed contain no maintainer grant | README documents Apple M1 TensorFlow trouble and a separate paid Spleeter Pro. Training data is Deezer-internal and unreleased | Unresolved weight grant. Out until a publisher states it in the repo, not only in the 2020 paper |
| BS-RoFormer / Mel-Band RoFormer | `lucidrains/BS-RoFormer` code is MIT. That is an implementation, not ByteDance weights | Official ByteDance weights were not found. Community vocal checkpoints have a moving license history (a GPL discussion, later MIT claims by redistributors). Training-data rights: unknown | Community checkpoints, not one maintained product | Out. Unknown training rights are not a redistributable grant |
| UVR and other model zoos | Not surveyed file by file | Per-checkpoint terms. This pass did not establish one license | Community GUI that can host Demucs and other checkpoints | Out. No single primary license |

### Primary sources

- Demucs code and status. README: the repository “is not maintained anymore,” with bug-fix-only follow-up at `github.com/adefossez/demucs`. License section: “Demucs is released under the MIT license as found in the LICENSE file.” <https://github.com/facebookresearch/demucs>
- Demucs weights. Alexandre Défossez, 23 May 2022, issue 327: “The model weights are not covered by the MIT license, and are provided only for scientific purposes.” Same author, 2 August 2023, issue 508: “Weights are provided for research purpose!” <https://github.com/facebookresearch/demucs/issues/327> <https://github.com/facebookresearch/demucs/issues/508>
- Third-party Hugging Face cards that label those weights CC-BY-NC 4.0 are not the author’s license text. This proposal does not adopt that SPDX tag.
- Demucs training mix. README: MUSDB HQ plus 800 extra songs. `htdemucs_ft` uses the same set and “will take 4 times more time.” Output of the documented command is four stereo files at 44.1 kHz: drums, bass, other, vocals. The README’s published score is 9.00 dB SDR on the MUSDB HQ test set for the non-sparse model, and 9.20 dB for a sparse model whose code “is not ready for release.”
- Demucs speed text, which is not an M4 Pro result. In the GPU section the README says CPU processing “should be roughly equal to 1.5 times the duration of the track,” and that GPU use wants at least 3 GB of GPU memory, about 7 GB with default arguments. The machine is unspecified. Open Deck has not reproduced either figure.
- Open-Unmix code, models, and the real-time limit. <https://github.com/sigsep/open-unmix-pytorch/blob/master/README.md>. The README says a bidirectional LSTM “cannot be used in an online/real-time manner.” `umxl` weights are “only licensed for non-commercial use (CC BY-NC-SA 4.0).” `umxhq` is trained on MUSDB18-HQ for “a full bandwidth of 22050 Hz.” Published median-of-median SDR on that page: `umxhq` vocals 6.25, bass 5.07, drums 6.04, other 4.28; `umxl` is higher and is the non-commercial model. The design section says the project “favored simplicity over performance” and is a baseline. Those scores are dataset scores, not an Open Deck listening test.
- `umxhq` weight record tagged MIT: Zenodo 10.5281/zenodo.3370489, four `.pth` files of 35.6 MB each. <https://zenodo.org/records/3370489>. The tag is the publisher’s mark on that archive. File size is not inference RAM.
- MUSDB18 and MUSDB18-HQ dataset terms: “provided for educational purposes only” and “should not be used for any commercial purpose without the express permission of the copyright holders.” <https://zenodo.org/records/1117372> <https://zenodo.org/records/3338373>. Whether an MIT tag on a weight file trained from that material is a commercial grant is unresolved here. A comment on the Demucs issue tracker is not the dataset license.
- Spleeter code: <https://github.com/deezer/spleeter>. Spleeter paper text in the repo: “source code and pre-trained models are available on github and distributed under a MIT license.” The same paper’s speed example is a GeForce RTX 2080 plus two Xeon Gold 6134 CPUs: the MUSDB18 test set (about 3 hours 27 minutes) separated to four stems in less than 2 minutes, and “100 seconds of stereo audio in less than 1 second.” The summary’s “100 times faster than real-time” line is that class of GPU setup, and it also says the model needs buffering so it is not a real-time process. <https://github.com/deezer/spleeter/blob/master/paper.md>
- ONNX Runtime code is MIT: <https://github.com/microsoft/onnxruntime/blob/main/LICENSE>. That license covers the runtime source, not any model loaded into it.

## On device, offline

No cloud separator is selected. A cloud call would send the user’s audio off the machine and would require an account or spend. Both are out.

An on-device job keeps the file local. It still costs CPU, memory, battery, heat, and disk, and it can stall the UI if it runs on the thread that owns the window or the audio callback. The architecture below is the constraint for any later runtime. The runtime itself is not chosen. Python, TensorFlow, PyTorch, ONNX Runtime, Core ML, and MLX are all unmeasured here. Adding any of them is an AI dependency and stays behind the approval gate. The 2026-06-24 removal dropped an ONNX Runtime plus `htdemucs` integration for that reason, and because Demucs was already archived.

## Performance evidence versus estimates

| Quantity | Status on the M4 Pro 24 GB target |
| --- | --- |
| Real-time factor | UNMEASURED. No Open Deck run |
| Time to first stem, end-to-end latency, dropout count | UNMEASURED. No playback session was run for this proposal |
| Peak resident memory, GPU/ANE use, thermal or fan behavior | UNMEASURED |
| Weight-file size of `umxhq` | 4 × 35.6 MB on the Zenodo record. This is download size, not RAM |
| Demucs README CPU “1.5× duration” and 3–7 GB GPU figures | Upstream claims for an unnamed machine. Not copied forward as local evidence |
| Spleeter RTX 2080 timing | Upstream claim for that GPU and those CPUs. Not copied forward |

Storage arithmetic, from PCM layout, not from a model run. Stereo f32 is `sample_rate × 8` bytes per second.

| Layout | One minute, one stereo stem | One minute, four stems | Five minutes, four stems |
| --- | --- | --- | --- |
| 44.1 kHz f32 | 20.19 MiB | 80.76 MiB | 403.8 MiB |
| 44.1 kHz i16 | 10.09 MiB | 40.38 MiB | 201.9 MiB |
| 48 kHz f32 | 21.97 MiB | 87.89 MiB | 439.5 MiB |

A mix already at the 512 MiB decode cap cannot also retain four f32 copies. The first experiment, when one is approved, stores stems as i16 on disk and installs one audible sum into the existing deck buffer. Peak RAM of the model session remains UNMEASURED and is a stop condition if it approaches the machine’s 24 GB.

## Recommended architecture

Recommended approach, confidence medium on the shape and low on any particular model: one cancellable offline job per track, then the existing single-buffer deck. Confidence is high that the audio callback cannot host the model. Open-Unmix states the bidirectional model is not online. Demucs and Spleeter both document chunked, non-realtime processing. The callback rules forbid allocation, locks, I/O, and unbounded work in any case.

1. The user asks for stems on a library track, or on the track loaded in a deck. Import does not start a job by itself.
2. A worker outside the audio callback decodes with the existing decoder, checks cancellation between chunks, and stops without installing a result when cancelled. The same pattern already exists for deck loading.
3. The worker resamples to the model rate off the callback. Surveyed Demucs output is 44.1 kHz stereo. Content above 22.05 kHz is absent from stems made at that rate.
4. One job runs at a time, at utility QoS, with one thread of model work. The Demucs `-j` flag multiplies RAM; this design does not use it. The job yields between chunks so file scanning, waveform preview, and the UI keep their existing budgets.
5. The job writes i16 stems to a temp directory. Commit is an atomic rename after the last chunk and a hash check. Cancel, process exit, and a failed chunk delete the temp directory. A partial file is never loaded.
6. Cache identity is the file content hash, the model id, the model file hash, and the stem rate and channel count. A changed file or a changed model is a miss. The old entry is removed when a new one commits. Deleting the library track deletes the cache. A library-wide byte cap is an owner setting; it is unset here.
7. Playback uses the original mix while every stem is enabled. That preserves the full-rate decode, including a 96 kHz source, and it avoids presenting the model’s reconstruction as the mix. Muting or soloing builds one new mix on the worker: the enabled i16 stems are summed, the existing resampler path takes that buffer to the device rate, and the result is installed through the current deck-buffer handoff. The callback still reads one buffer. Old buffers retire on the control thread, as `docs/DECK-GRAPH.md` already requires for `Arc` deck buffers.
8. A mute change during playback replaces that one buffer. It is not sample-accurate, and it does not run inside the callback. Rapid toggles keep the latest requested mask; earlier bakes are dropped. A short control-thread crossfade can be added later if a measurement shows a click. That crossfade is also off the callback.
9. Sample-accurate four-buffer reading inside the callback is a later graph change. It waits on the deck-graph split and on a measured callback budget. It is not part of the first slice.

No UI or audio stall is an acceptance condition, not a claim that the design already meets it. The worker must not take the audio device lock, the MIDI lock, or a lock the callback waits on.

## Formats, rates, and channels

The folder scanner accepts `mp3`, `flac`, `wav`, `m4a`, `aac`, `ogg`, `oga`, `aiff`, and `aif`. A stem job accepts a file only after that decoder has produced finite, non-empty stereo PCM. Generated fixtures already cover 44.1 kHz stereo PCM WAV, PCM AIFF (`.aiff` and `.aif`), AAC-LC in MPEG-4 ADTS, and AAC-LC in `.m4a`. MP3, FLAC, and Ogg Vorbis stay on the existing decoder. MPEG-2 ADTS, ALAC, HE-AAC, USAC, and compressed AIFC are unsupported at decode, so they are unsupported for stems.

First-slice channel and rate rules, if the feature is later approved:

- Stereo only. Mono and more than two channels fail the job with an explicit reason. No silent downmix.
- Model I/O is 44.1 kHz stereo for the Demucs command documented above. Other decoded rates are resampled offline for the job. Stored stems follow the model rate. Device-rate conversion stays on the existing playback resampler.
- Empty, non-finite, or truncated decodes fail the job. The loaded mix, if any, keeps playing.
- Duration is bounded by the 512 MiB mix cap and by the unset stem cache cap. A file that decodes and still cannot reserve the i16 stem files fails before inference.

## UI touchpoints, proposed only

Match the current workspace: one library, deck inspector, square controls, deck letter plus deck color, hover labels. No new browser, no hardware texture, no stem waveform until a real stem buffer exists.

- Library row and inspector: a text state, `Stems ready`, `Separating`, `Cancelled`, or the failure reason. Absence of stems shows nothing that looks like a result.
- Deck inspector, only for a loaded track that has a committed cache: four square latch buttons, `Vocal`, `Drums`, `Bass`, `Other`, in the deck color, with the deck letter. All four on means the original mix. A mute requests the bake in step 8. Hover labels name the stem and the deck.
- Transport stays available on the original mix during the job. The job does not disable Play, and it does not take Pad FX, hot cue, beat jump, sampler, or keyboard pads.
- FLX4 mapping waits for a cited stem note in the controller list. This proposal does not add one.

## Failure and fallback

| Condition | Behavior |
| --- | --- |
| Feature gate still closed | No control is shown. This document does not add a disabled stub. |
| Model file missing, hash mismatch, or license not accepted by the owner | Job does not start. Mix plays. |
| Unsupported codec, channel count, or empty decode | Job fails with that reason. Mix plays. |
| Cancel, quit, or mode change away from the track | Temp files deleted. Installed audio stays the original mix unless a completed bake was already installed. |
| Worker crash or non-finite output | Temp files deleted. Deck returns to the original mix on the next control-thread tick. The failure is visible on the track. |
| Cache over the owner cap | Oldest stem sets are removed. A deck that was using a removed set returns to the original mix and shows the stems as absent. |
| All four stems enabled | Original mix buffer, not the sum of the model outputs. |

A stem mute is a reduced mix, with bleed and holes. The UI uses the stem names as estimates. It does not say the residual is a perfect instrumental.

## Phased acceptance

Nothing below has been run.

1. Owner decisions in the last section. No dependency and no weight download before that.
2. License file check for the exact weight bytes to be used, including training-data terms. A MIT code license in the same repo does not pass this step.
3. Offline routing test on a generated tone mix: cancel before inference, failed decode, and a committed fixture of four i16 stems. Assert channel count, rate, finite samples, and duration. Synthetic tones prove the pipeline. They do not prove musical quality.
4. Measurement session on the stated M4 Pro, one stereo 44.1 kHz track, one model, one job: wall time, peak resident memory, disk bytes, and whether the UI and an already playing deck keep their current behavior. Record the build, macOS version, and power source. Pass/fail numbers are chosen from that run, not from this document. Missing hardware or a skipped run is NOT RUN.
5. Cache tests: content-hash miss, model-id miss, delete-on-track-delete, partial-file cleanup, cap eviction.
6. Playback tests: all-on uses the original buffer; a mute installs one replacement buffer; a failed bake leaves the previous buffer; the audio-thread test suite still shows no allocation on the callback.
7. UI tests for the labels and the four buttons. A controller test is in scope only after a protocol citation.
8. A live latency and dropout session is a separate hardware gate. This proposal has no such proof.

## Privacy, security, and resources to measure later

- Audio stays on the machine. No account, no upload, no product telemetry of the file.
- Stem files are derivatives of the user’s audio. They leave with the track delete and with cache eviction.
- A later downloader, if the owner allows one, pins the URL, the byte size, and the hash before use. A mismatched hash is deleted. Remote code execution and unpinned weight URLs are out.
- Model parsers are native code. The first runtime, if any, loads one pinned local file under a size cap. Python plugin loading is out.
- Budgets to fill in after step 4, and not before: seconds per minute of audio, peak RSS, disk per track, library cache cap, and the longest track accepted. Thermal impact is a note from that same session, not a guess.

## Maintainability

Demucs is the strongest published quality reference in this survey and a poor dependency: the author has archived the Meta repository, limited the fork to important bug fixes, and kept the weights for research. Spleeter’s code is still public, its README and its paper disagree on the weight grant, its README already records Apple Silicon TensorFlow trouble, and the open confirmation issues have no maintainer answer in the pages reviewed. Open-Unmix is honest about being a 2019-era baseline; `umxl` is non-commercial; `umxhq` is MIT-tagged at the weight archive and still tied to a non-commercial dataset. Community RoFormer checkpoints move and relicense, and their training data is unknown. None of these projects is a supported audio engine for a DJ application. Owning a frozen, hash-pinned weight file is the maintainable shape. Tracking an upstream Python package is not.

## Decision and rationale

Proposed, not accepted: do not implement stem separation.

The architecture to use after an explicit owner approval is the offline job and single-buffer bake above. The measurement candidate, and only a candidate, is Open-Unmix `umxhq`, because its weight archive is the only one in this survey with a publisher MIT tag on the weights themselves. That tag does not close the MUSDB18-HQ commercial question. Demucs stays the quality reference and stays blocked by the author’s research-only statement. Spleeter and RoFormer stay blocked by unresolved weight or training rights. Cloud is out. Traditional DSP stays available for other audio work and is the wrong label for this feature.

Confidence: high on the approval gate, the callback boundary, the FLX4 gap, and the code-versus-weights split. Medium on offline precompute plus a single installed mix as the first playback shape. Low on `umxhq` being good enough to ship, and none on M4 Pro speed or memory until step 4 exists.

## Owner decisions required

1. Keep the no-AI product rule, or lift it in writing for this feature only.
2. Accept or reject a model whose weight file is MIT-tagged while its training set is educational / non-commercial. This is a rights decision.
3. Bundle a pinned weight file, or require a local file the app does not download. Silent downloads are out either way.
4. Accept baseline separation quality for a first slice, or wait until a commercially clear model with a current maintainer exists. No such model was identified here.
5. Set the library stem-cache cap and whether jobs may run on battery.
6. Confirm software latch buttons only, with FLX4 pads left unchanged.
7. Approve a later measurement session. This proposal does not run it.

Until those decisions are recorded, stem implementation stays stopped.
