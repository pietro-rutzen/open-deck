import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  deckPause,
  deckPlay,
  deckScratch,
  deckSeek,
  deckUnload,
  setDeckKeylock,
  setDeckEffect,
  effectsSnapshot,
  onEffectsUpdate,
  type EffectsSnapshot,
  setDeckPitchShift,
  inTauri,
  loadTrack,
  onDeckError,
  onDeckLoaded,
  onDeckLoading,
  onLibraryPathsMoved,
  onDeckPosition,
  onAudioOutputReset,
  pickAudioFile,
  setBeatgrid,
  setDeckSync,
  setDeckSyncLive,
  setDeckSyncInternal,
  setDeckXfaderAssign,
  setDeckEq,
  setDeckEqBand,
  setDeckFilter,
  setDeckGain,
  setDeckTrim,
  setDeckTempo,
  setLoop as setLoopCmd,
  setLoopActive as setLoopActiveCmd,
  setLoopRoll as setLoopRollCmd,
  scaleLoop as scaleLoopCmd,
  moveLoop as moveLoopCmd,
  cueButton as cueButtonCmd,
  setCueMode as setCueModeCmd,
  setDeckSyncMode as setDeckSyncModeCmd,
  setSyncLeader as setSyncLeaderCmd,
  performanceSetHotCue,
  performanceSetHotCues,
  performanceSetJumpScale,
  performanceSetLoop,
  performanceSetQuantize,
  performanceSetSynced,
  dbClearCue,
  dbClearLoop,
  dbRecordPlay,
  dbSetCue,
  dbSetGain,
  dbSetGridOffset,
  dbSetLoop,
  dbTrackState,
  dbUpsertAnalysis,
  type ControllerUpdate,
  type DeckLoaded,
  type FilterMode,
} from "../lib/ipc";
import { listeningModeActive } from "../lib/listeningMode";
import { firstAudibleFrame } from "../lib/silence";
import { TRIM_MAX } from "../lib/trim";

export interface LoopState {
  active: boolean;
  /** Beat length of the active beat-loop, or null for a manual in/out loop. */
  beats: number | null;
  inFrame: number;
  outFrame: number;
  /** Manual loop-in set and waiting for OUT (no active loop yet) — for IN/OUT button feedback. */
  armed?: boolean;
}

/** Deck controls the native controller thread applies directly to the engine. */
export const NATIVE_DECK_CONTROLS = ["volume", "trim", "eq.low", "eq.mid", "eq.high", "filter", "tempo"] as const;
export type NativeDeckControl = (typeof NATIVE_DECK_CONTROLS)[number];
export const isNativeDeckControl = (control: string): control is NativeDeckControl =>
  (NATIVE_DECK_CONTROLS as readonly string[]).includes(control);

const NATIVE_PERFORMANCE_CONTROL =
  /^(?:hotcue|clear_hotcue|beatloop|beatjump)\.\d$|^(?:loop\.(?:in|out|toggle|half|double|reloop_stop|nudge_in|nudge_out)|jump\.(?:back|forward)|beatjump\.scale_(?:down|up)|quantize|sync)$/;

/** Cue, loop, jump, quantize, and sync actions the controller thread applies itself. */
export const isNativePerformanceControl = (control: string): boolean =>
  NATIVE_PERFORMANCE_CONTROL.test(control);

const TRIM_PERSIST_DELAY_MS = 250;

/** Tempo step per −/+ trim click (0.1%). */
const TEMPO_TRIM = 0.001;
/** Tempo trim range (±10%), keeping the pitch fader meaningful. */
const TEMPO_TRIM_MIN = 0.9;
const TEMPO_TRIM_MAX = 1.1;

export interface Eq {
  hi: number;
  mid: number;
  low: number;
}

export interface EchoState {
  active: boolean;
  /** Delay time as a fraction/multiple of a beat (¼, ½, 1, 2). */
  beats: number;
  /** Single 0..1 "amount" knob driving both feedback and wet mix. */
  depth: number;
}

export interface ReverbState {
  active: boolean;
  /** Room size 0..1 (tail length). */
  size: number;
  /** Wet/dry mix 0..1. */
  mix: number;
}

export interface FlangerState {
  active: boolean;
  /** LFO sweep period in beats (beat-synced). */
  beats: number;
  /** Sweep width 0..1 (also drives feedback/resonance). */
  depth: number;
}

export interface CrusherState {
  active: boolean;
  /** Crush amount 0..1 (0 = clean ~16-bit, 1 = ~2-bit). */
  crush: number;
  /** Sample-rate reduction 0..1 (0 = off, 1 = heavy decimation). */
  down: number;
}

/** Fallback echo time (seconds) for one beat when a track has no beatgrid. */
const NO_GRID_BEAT_SEC = 0.5;

export interface DeckState {
  meta: DeckLoaded | null;
  frame: number;
  playing: boolean;
  level: number;
  /** Effective advance in source frames/sec (for play-head extrapolation). */
  rate: number;
  /** Measured output (DAC) latency in seconds. */
  latencySecs: number;
  /** `performance.now()` when `frame` was last sampled. */
  frameAt: number;
  tempo: number;
  /** Hardware tempo-fader span in percent. SHIFT+SYNC cycles 8, 6, 10, 16, and 100. */
  tempoRange: number;
  /** Multiplier for beat-jump pads. SHIFT beat-jump pads scale it by 16, clamped to 1/16…16. */
  beatJumpScale: number;
  /** Key-lock (master tempo): tempo changes preserve pitch. */
  keylock: boolean;
  /** Key shift in semitones (± ), pitch-shifting the deck without changing tempo. */
  pitchShift: number;
  /** Manual beatgrid nudge (seconds) added to the analyzed first-beat anchor. */
  gridOffset: number;
  /** True while this deck is a continuous sync follower. */
  synced: boolean;
  /** Following the live mic/aux beat clock (tempo-match). Mutually exclusive with deck sync. */
  syncLive: boolean;
  /** Following the internal master clock. Mutually exclusive with deck sync and live sync. */
  syncInternal: boolean;
  /** Quantize: snap hot-cue jumps and beat-jumps to the beatgrid. */
  quantize: boolean;
  /** Main-cue behavior: 0 = CDJ, 1 = gated. */
  cueMode: number;
  /** Sync mode: 0 = full tempo+phase, 1 = tempo-only. */
  syncMode: number;
  /** Whether this deck is pinned as the explicit sync leader. */
  isLeader: boolean;
  /** Crossfader routing: 0 = A side, 1 = thru, 2 = B side. */
  xfaderAssign: number;
  eq: Eq;
  filter: number;
  /** Effective engine gain, always trim × volume. */
  gain: number;
  /** Input trim, separate from the performance channel fader. */
  trim: number;
  /** Channel fader: 0 = silent, 1 = unity. */
  volume: number;
  hotCues: (number | null)[];
  loop: LoopState;
  echo: EchoState;
  reverb: ReverbState;
  flanger: FlangerState;
  crusher: CrusherState;
  error: string | null;
  /** True between clicking load and the track being decoded/analyzed. */
  loading: boolean;
  /** Whether this deck supports full DSP (local) vs control-only (streaming). */
  dsp: boolean;
}

function filterParams(x: number): { mode: FilterMode; cutoff: number; resonance: number } {
  if (Math.abs(x) < 0.02) return { mode: "off", cutoff: 1000, resonance: 0.9 };
  if (x < 0) return { mode: "lowpass", cutoff: 20000 * Math.pow(200 / 20000, -x), resonance: 0.9 + -x };
  return { mode: "highpass", cutoff: 20 * Math.pow(4000 / 20, x), resonance: 0.9 + x };
}

export function useDeck(deck: number, dsp = true, clock?: { active: boolean; bpm: number }) {
  const [meta, setMeta] = useState<DeckLoaded | null>(null);
  const [frame, setFrame] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [level, setLevel] = useState(0);
  // Play-head extrapolation inputs: advance rate (source frames/sec), DAC latency, and the
  // wall-clock time `frame` was last sampled — the UI smooths between 30 Hz position events.
  const [rate, setRate] = useState(0);
  const [latencySecs, setLatencySecs] = useState(0);
  const [frameAt, setFrameAt] = useState(0);
  const [tempo, setTempo] = useState(1);
  const [tempoRange, setTempoRangeState] = useState(8);
  const [beatJumpScale, setBeatJumpScale] = useState(1);
  const beatJumpScaleRef = useRef(1);
  const [keylock, setKeylockState] = useState(false);
  const [pitchShift, setPitchShiftState] = useState(0);
  const [gridOffset, setGridOffset] = useState(0);
  const [synced, setSyncedState] = useState(false);
  const [syncLive, setSyncLiveState] = useState(false);
  const [syncInternal, setSyncInternalState] = useState(false);
  const [quantize, setQuantize] = useState(false);
  // Main-cue behavior (0 = CDJ, 1 = gated) and sync mode (0 = full, 1 = tempo-only) + leader pin.
  const [cueMode, setCueModeState] = useState(0);
  const [syncMode, setSyncModeState] = useState(0);
  const [isLeader, setIsLeaderState] = useState(false);
  // Default routing: deck 0 → A, deck 1 → B, decks C/D → through.
  const [xfaderAssign, setXfaderAssignState] = useState(deck === 0 ? 0 : deck === 1 ? 2 : 1);
  const [eq, setEqState] = useState<Eq>({ hi: 0, mid: 0, low: 0 });
  const eqRef = useRef<Eq>({ hi: 0, mid: 0, low: 0 });
  const [filter, setFilterState] = useState(0);
  const [gain, setGainState] = useState(1);
  const [trim, setTrimState] = useState(1);
  const [volume, setVolumeState] = useState(1);
  // Update synchronously in actions: multiple MIDI messages can arrive before React renders.
  const trimRef = useRef(1);
  const volumeRef = useRef(1);
  const gainEditRef = useRef(0);
  const gainLoadRef = useRef(0);
  const [hotCues, setHotCues] = useState<(number | null)[]>(Array(8).fill(null));
  const [loop, setLoopState] = useState<LoopState>({ active: false, beats: null, inFrame: 0, outFrame: 0 });
  const [echo, setEchoState] = useState<EchoState>({ active: false, beats: 0.5, depth: 0.5 });
  const [reverb, setReverbState] = useState<ReverbState>({ active: false, size: 0.6, mix: 0.3 });
  const [flanger, setFlangerState] = useState<FlangerState>({ active: false, beats: 4, depth: 0.6 });
  const [crusher, setCrusherState] = useState<CrusherState>({ active: false, crush: 0.5, down: 0.3 });
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const frameRef = useRef(0);
  const framePaintAt = useRef(0);
  const ratePaintRef = useRef(0);
  const tempoRef = useRef(tempo);
  tempoRef.current = tempo;
  const keylockRef = useRef(keylock);
  keylockRef.current = keylock;
  const gridOffsetRef = useRef(gridOffset);
  gridOffsetRef.current = gridOffset;
  const loopRef = useRef(loop);
  loopRef.current = loop;
  const echoRef = useRef(echo);
  echoRef.current = echo;
  const reverbRef = useRef(reverb);
  reverbRef.current = reverb;
  const flangerRef = useRef(flanger);
  flangerRef.current = flanger;
  const crusherRef = useRef(crusher);
  crusherRef.current = crusher;

  // Beat-synced FX derive their beat length from the deck's analyzed grid — UNLESS the deck follows
  // the internal master clock, in which case the engine rate-matches the deck to the clock, so the
  // deck's audio beats at the clock tempo and the FX must track that instead.
  const fxBeatSec =
    syncInternal && clock?.active && clock.bpm > 0
      ? 60 / clock.bpm
      : (meta?.beat_interval_sec ?? 0) > 0
        ? meta!.beat_interval_sec
        : NO_GRID_BEAT_SEC;

  useEffect(() => {
    if (!inTauri()) return;
    let disposed = false;
    let revision = -1;
    const reflect = (snapshot: EffectsSnapshot) => {
      if (disposed || snapshot.revision <= revision) return;
      const next = snapshot.decks[deck];
      if (!next) return;
      revision = snapshot.revision;
      echoRef.current = next.echo; setEchoState(next.echo);
      flangerRef.current = next.flanger; setFlangerState(next.flanger);
      reverbRef.current = next.reverb; setReverbState(next.reverb);
      crusherRef.current = next.crusher; setCrusherState(next.crusher);
      pitchShiftRef.current = next.pitch_shift; setPitchShiftState(next.pitch_shift);
    };
    const subscription = onEffectsUpdate(reflect);
    subscription.then(() => effectsSnapshot()).then(reflect).catch(() => {});
    return () => { disposed = true; subscription.then(unlisten => unlisten()); };
  }, [deck]);
  const applyEcho = useCallback((e: EchoState) => {
    setDeckEffect(deck, { effect: 'echo', ...e }).catch(error => setError(String(error)));
  }, [deck]);
  const applyFlanger = useCallback((f: FlangerState) => {
    setDeckEffect(deck, { effect: 'flanger', ...f }).catch(error => setError(String(error)));
  }, [deck]);
  useEffect(() => {
    if (!inTauri()) return;
    setDeckEffect(deck, { effect: 'beat_sec', seconds: fxBeatSec }).catch(error => setError(String(error)));
  }, [deck, fxBeatSec]);
  // Path of the loaded track + a once-per-load guard for recording a play.
  const pathRef = useRef<string | null>(null);
  const playedRef = useRef(false);
  const quantizeRef = useRef(quantize);
  quantizeRef.current = quantize;
  const pitchShiftRef = useRef(pitchShift);
  pitchShiftRef.current = pitchShift;
  const trimPersistTimerRef = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(trimPersistTimerRef.current), []);

  // Engine telemetry is the transport truth for UI, keyboard and native controller playback alike.
  useEffect(() => {
    const loadedPath = pathRef.current;
    if (!playing || !dsp || !loadedPath || playedRef.current) return;
    playedRef.current = true;
    if (listeningModeActive()) return;
    dbRecordPlay(loadedPath).catch(() => {});
  }, [playing, dsp]);

  useEffect(() => {
    if (!inTauri()) return;
    const unlistens: UnlistenFn[] = [];
    let active = true;
    const track = (p: Promise<UnlistenFn>) => p.then((u) => (active ? unlistens.push(u) : u()));

    track(
      onAudioOutputReset(() => {
        const flatEq = { hi: 0, mid: 0, low: 0 };
        eqRef.current = flatEq;
        setEqState(flatEq);
        setFilterState(0);
        tempoRef.current = 1;
        setTempo(1);
        if (!pathRef.current) return;
        pathRef.current = null;
        gainLoadRef.current += 1;
        setMeta(null);
        setPlaying(false);
        setLoading(false);
        setError("Speakers changed. Load this deck again.");
      }),
    );
    track(
      onDeckLoading((e) => {
        if (e.deck !== deck) return;
        setLoading(true);
        setError(null);
      }),
    );
    track(
      onDeckLoaded((e) => {
        if (e.deck !== deck) return;
        const gainLoad = ++gainLoadRef.current;
        const gainEdit = gainEditRef.current;
        setMeta(e);
        const origin = firstAudibleFrame(e.silence, e.frames);
        setFrame(origin);
        frameRef.current = origin;
        setPlaying(false);
        setTempo(1);
        setKeylockState(false); // engine resets key-lock on load
        setPitchShiftState(0); // engine resets key shift on load
        setGridOffset(0);
        setSyncedState(false); // engine resets sync on load
        setHotCues(Array(8).fill(null));
        setLoopState({ active: false, beats: null, inFrame: 0, outFrame: 0 });
        // Engine resets FX on load; mirror that (keep the user's param settings).
        setEchoState((e) => ({ ...e, active: false }));
        setReverbState((r) => ({ ...r, active: false }));
        setFlangerState((f) => ({ ...f, active: false }));
        setCrusherState((c) => ({ ...c, active: false }));
        setError(null);
        setLoading(false);
        pathRef.current = e.path;
        playedRef.current = false;

        if (dsp) {
          // Cache analysis onto the library row, then restore saved cues/loops/grid/gain.
          dbUpsertAnalysis(e).catch(() => {});
          dbTrackState(e.path)
            .then((st) => {
              if (pathRef.current !== e.path) return; // a newer track loaded meanwhile
              if (st.grid_offset_sec) {
                setGridOffset(st.grid_offset_sec);
                if ((e.beat_interval_sec ?? 0) > 0) {
                  const sr = e.source_rate;
                  setBeatgrid(
                    deck,
                    (e.first_beat_sec + st.grid_offset_sec) * sr,
                    e.beat_interval_sec * sr,
                  ).catch(() => {});
                }
              }
              // The saved legacy scalar is treated as input trim going forward; this is not
              // a data migration. Never restore track gain into the channel volume fader.
              if (gainLoadRef.current === gainLoad && gainEditRef.current === gainEdit) {
                const savedTrim = Number.isFinite(st.gain) ? Math.max(0, Math.min(TRIM_MAX, st.gain)) : 1;
                trimRef.current = savedTrim;
                setTrimState(savedTrim);
              }
              if (gainLoadRef.current === gainLoad) {
                const effective = trimRef.current * volumeRef.current;
                setGainState(effective);
                setDeckTrim(deck, trimRef.current).catch(() => {});
                setDeckGain(deck, volumeRef.current).catch(() => {});
              }
              if (st.cues.length) {
                const arr: (number | null)[] = Array(8).fill(null);
                for (const c of st.cues) if (c.slot >= 0 && c.slot < 8) arr[c.slot] = c.frame;
                setHotCues(arr);
                performanceSetHotCues(deck, arr).catch(() => {});
              }
              // Restore the saved loop region, but leave it disarmed (don't jump the play-head).
              const lp = st.loops.find((l) => l.slot === 0);
              if (lp) {
                const restored = {
                  active: false,
                  beats: lp.beats,
                  inFrame: lp.in_frame,
                  outFrame: lp.out_frame,
                };
                setLoopState(restored);
                performanceSetLoop(deck, restored.inFrame, restored.outFrame, false, false, restored.beats).catch(() => {});
              }
            })
            .catch(() => {
              if (pathRef.current !== e.path || gainLoadRef.current !== gainLoad) return;
              const effective = trimRef.current * volumeRef.current;
              setGainState(effective);
              setDeckTrim(deck, trimRef.current).catch(() => {});
                setDeckGain(deck, volumeRef.current).catch(() => {});
            });
        }
      }),
    );
    track(
      onDeckPosition((e) => {
        if (e.deck !== deck) return;
        const previousFrame = frameRef.current;
        frameRef.current = e.frame;
        const rate = e.rate ?? 0;
        const latency = e.latency_secs ?? 0;
        const syncMode = e.tempo_only ? 1 : 0;
        const now = performance.now();
        const previousRate = ratePaintRef.current;
        const jumped = Math.abs(e.frame - previousFrame) > Math.max(Math.abs(rate) * 0.2, 4410);
        const rateChanged = Math.abs(previousRate - rate) >= 1e-4;
        const moving = e.playing || Math.abs(rate) > 1e-3;
        setSyncModeState((current) => (current === syncMode ? current : syncMode));
        setPlaying((current) => (current === e.playing ? current : e.playing));
        setLevel((current) => (meterStep(current) === meterStep(e.level) ? current : e.level));
        setLatencySecs((current) => (Math.abs(current - latency) < 5e-4 ? current : latency));
        // Frame, timestamp, and rate stay one sample. A fresh scratch rate applied
        // across a stale elapsed interval throws the waveform backward, then the next packet snaps it forward.
        if (moving || rateChanged || jumped || now - framePaintAt.current >= 90) {
          ratePaintRef.current = rate;
          framePaintAt.current = now;
          setRate((current) => (Math.abs(current - rate) < 1e-4 ? current : rate));
          setFrame(e.frame);
          setFrameAt(now);
        }
      }),
    );
    track(
      onDeckError((e) => {
        if (e.deck !== deck) return;
        setError(e.message);
        setLoading(false);
      }),
    );
    track(
      onLibraryPathsMoved((moves) => {
        const current = pathRef.current;
        const next = current ? moves.find((move) => move.from === current)?.to : undefined;
        if (!current || !next) return;
        pathRef.current = next;
        setMeta((loaded) => (loaded && loaded.path === current ? { ...loaded, path: next } : loaded));
      }),
    );

    return () => {
      active = false;
      unlistens.forEach((u) => u());
    };
  }, [deck]);

  const actions = useMemo(() => {
    const swallow = () => {};
    // Per-track persistence is keyed by file path and only for local (DSP) decks.
    const path = meta?.path ?? null;
    const canPersist = dsp && !!path;
    const publishLoop = (next: LoopState) => {
      performanceSetLoop(deck, next.inFrame, next.outFrame, next.active, !!next.armed, next.beats).catch(swallow);
    };
    // Beat-synced echo/flanger pushes are defined at component scope (they track the internal
    // clock via `fxBeatSec` and re-push on tempo change); alias them here for the setters below.
    const pushEcho = applyEcho;
    const pushFlanger = applyFlanger;
    const pushReverb = (r: ReverbState) => {
      setDeckEffect(deck, { effect: "reverb", ...r }).catch(swallow);
    };
    // Bitcrusher: crush 0..1 → 16..2 bits; down 0..1 → 1..32× sample-and-hold. Full wet.
    const pushCrusher = (c: CrusherState) => {
      setDeckEffect(deck, { effect: "crusher", ...c }).catch(swallow);
    };
    // Current beatgrid in source frames (interval + first-beat offset incl. manual nudge), or
    // null when the track has no grid. Read live so a grid nudge takes effect immediately.
    const grid = (): { interval: number; offset: number } | null => {
      if (!meta || (meta.beat_interval_sec ?? 0) <= 0) return null;
      const sr = meta.source_rate;
      return {
        interval: meta.beat_interval_sec * sr,
        offset: (meta.first_beat_sec + gridOffsetRef.current) * sr,
      };
    };
    const snapBeat = (f: number) => {
      const g = grid();
      return g ? g.offset + Math.round((f - g.offset) / g.interval) * g.interval : f;
    };
    // Push the (possibly nudged) beatgrid to the engine in source frames, for the sync PLL.
    const pushBeatgrid = (gridOff: number) => {
      if (!meta || (meta.beat_interval_sec ?? 0) <= 0) return;
      const sr = meta.source_rate;
      setBeatgrid(deck, (meta.first_beat_sec + gridOff) * sr, meta.beat_interval_sec * sr).catch(swallow);
    };
    return {
      load: async () => {
        try {
          const path = await pickAudioFile();
          if (path) await loadTrack(deck, path);
        } catch (e) {
          setError(String(e));
        }
      },
      eject: () => {
        deckUnload(deck).catch(swallow);
        pathRef.current = null;
        gainLoadRef.current++;
        setMeta(null);
        setPlaying(false);
      },
      togglePlay: () => (playing ? deckPause(deck) : deckPlay(deck)).catch(swallow),
      play: () => deckPlay(deck).catch(swallow),
      pause: () => deckPause(deck).catch(swallow),
      cue: () => deckSeek(deck, 0).catch(swallow),
      seekFrac: (f: number) => {
        const frames = meta?.frames ?? 0;
        deckSeek(deck, f * frames).catch(swallow);
      },
      // Jog-wheel scratch: the platter streams a read-rate from drag velocity.
      scratch: (active: boolean, speed: number) => deckScratch(deck, active, speed).catch(swallow),
      setTempo: (ratio: number) => {
        setTempo(ratio);
        setDeckTempo(deck, ratio).catch(swallow);
      },
      toggleKeylock: () => {
        const next = !keylockRef.current;
        setKeylockState(next);
        setDeckKeylock(deck, next).catch(swallow);
      },
      // Key shift: transpose ± semitones without changing tempo (runs through the key-lock WSOLA).
      setPitchShift: (semitones: number) => {
        const s = Math.max(-12, Math.min(12, Math.round(semitones)));
        pitchShiftRef.current = s;
        setPitchShiftState(s);
        setDeckPitchShift(deck, s).catch(swallow);
      },
      // Relative key shift reads the ref so rapid pad presses accumulate before React renders.
      shiftPitchBy: (semitones: number) => {
        const s = Math.max(-12, Math.min(12, Math.round(pitchShiftRef.current + semitones)));
        pitchShiftRef.current = s;
        setPitchShiftState(s);
        setDeckPitchShift(deck, s).catch(swallow);
      },
      // Manual beatgrid anchor: shift the grid to line it up with the audio. Feeds waveform
      // rendering + beat-loop math (frontend) and the engine sync PLL (pushBeatgrid).
      nudgeGrid: (deltaSec: number) => {
        const next = gridOffsetRef.current + deltaSec;
        setGridOffset(next);
        pushBeatgrid(next);
        if (canPersist) dbSetGridOffset(path as string, next).catch(swallow);
      },
      resetGrid: () => {
        setGridOffset(0);
        pushBeatgrid(0);
        if (canPersist) dbSetGridOffset(path as string, 0).catch(swallow);
      },
      // Continuous beat-sync: follow `master` (deck index), or null to disengage.
      sync: (master: number | null) => {
        setSyncedState(master !== null);
        if (master !== null) {
          // The engine cancels live and internal-clock sync; mirror it in the UI.
          setSyncLiveState(false);
          setSyncInternalState(false);
        }
        setDeckSync(deck, master).catch(swallow);
        performanceSetSynced(deck, master !== null).catch(swallow);
      },
      // Tempo-match the live mic/aux beat clock (mutually exclusive with deck and internal sync).
      toggleSyncLive: () => {
        const next = !syncLive;
        setSyncLiveState(next);
        if (next) {
          setSyncedState(false);
          setSyncInternalState(false);
        }
        setDeckSyncLive(deck, next).catch(swallow);
      },
      // Tempo/phase-match the internal master clock (mutually exclusive with deck and live sync).
      toggleSyncInternal: () => {
        const next = !syncInternal;
        setSyncInternalState(next);
        if (next) {
          setSyncedState(false);
          setSyncLiveState(false);
        }
        setDeckSyncInternal(deck, next).catch(swallow);
      },
      // Crossfader routing (0 = A, 1 = thru, 2 = B).
      setXfaderAssign: (a: number) => {
        setXfaderAssignState(a);
        setDeckXfaderAssign(deck, a).catch(swallow);
      },
      // Persistent fine tempo trim (the jog wheel handles momentary pitch bend). Reads
      // the ref so rapid clicks accumulate instead of all seeing the same render's tempo.
      trimTempo: (dir: 1 | -1) => {
        const next = Math.min(TEMPO_TRIM_MAX, Math.max(TEMPO_TRIM_MIN, tempoRef.current + dir * TEMPO_TRIM));
        tempoRef.current = next;
        setTempo(next);
        setDeckTempo(deck, next).catch(swallow);
      },
      setEq: (next: Eq) => {
        const previous = eqRef.current;
        eqRef.current = next;
        setEqState(next);
        const changedBands = (
          [
            [0, previous.low, next.low],
            [1, previous.mid, next.mid],
            [2, previous.hi, next.hi],
          ] as const
        ).filter(([, before, after]) => before !== after);
        if (changedBands.length === 1) {
          const [band, , value] = changedBands[0];
          setDeckEqBand(deck, band, value).catch(swallow);
        } else if (changedBands.length > 1) {
          setDeckEq(deck, next.low, next.mid, next.hi).catch(swallow);
        }
      },
      setFilter: (x: number) => {
        setFilterState(x);
        const p = filterParams(x);
        setDeckFilter(deck, p.mode, p.cutoff, p.resonance).catch(swallow);
      },
      setTrim: (value: number) => {
        if (!Number.isFinite(value)) return;
        const next = Math.max(0, Math.min(TRIM_MAX, value));
        trimRef.current = next;
        gainEditRef.current++;
        setTrimState(next);
        const effective = next * volumeRef.current;
        setGainState(effective);
        setDeckTrim(deck, trimRef.current).catch(swallow);
        setDeckGain(deck, volumeRef.current).catch(swallow);
        if (canPersist) dbSetGain(path as string, next).catch(swallow);
      },
      setVolume: (value: number) => {
        if (!Number.isFinite(value)) return;
        const next = Math.max(0, Math.min(1, value));
        volumeRef.current = next;
        setVolumeState(next);
        const effective = trimRef.current * next;
        setGainState(effective);
        setDeckTrim(deck, trimRef.current).catch(swallow);
        setDeckGain(deck, volumeRef.current).catch(swallow);
      },
      // Legacy mappings request absolute effective gain, not a channel-volume adjustment.
      // Normalize the decomposition to trim=g, volume=1, including when the fader was zero.
      setGain: (value: number) => {
        if (!Number.isFinite(value)) return;
        const next = Math.max(0, Math.min(1.5, value));
        trimRef.current = next;
        volumeRef.current = 1;
        gainEditRef.current++;
        setTrimState(next);
        setVolumeState(1);
        setGainState(next);
        setDeckTrim(deck, next).catch(swallow);
        setDeckGain(deck, 1).catch(swallow);
        if (canPersist) dbSetGain(path as string, next).catch(swallow);
      },
      // Native controller path already applied these to the engine; only UI state follows.
      reflectController: (control: NativeDeckControl, value: number) => {
        if (!Number.isFinite(value)) return;
        switch (control) {
          case "volume": {
            const next = Math.max(0, Math.min(1, value));
            volumeRef.current = next;
            setVolumeState(next);
            setGainState(trimRef.current * next);
            return;
          }
          case "trim": {
            const next = Math.max(0, Math.min(TRIM_MAX, value));
            trimRef.current = next;
            gainEditRef.current++;
            setTrimState(next);
            setGainState(next * volumeRef.current);
            if (!canPersist) return;
            window.clearTimeout(trimPersistTimerRef.current);
            trimPersistTimerRef.current = window.setTimeout(
              () => dbSetGain(path as string, next).catch(swallow),
              TRIM_PERSIST_DELAY_MS,
            );
            return;
          }
          case "eq.low": {
            const next = { ...eqRef.current, low: value };
            eqRef.current = next;
            setEqState(next);
            return;
          }
          case "eq.mid": {
            const next = { ...eqRef.current, mid: value };
            eqRef.current = next;
            setEqState(next);
            return;
          }
          case "eq.high": {
            const next = { ...eqRef.current, hi: value };
            eqRef.current = next;
            setEqState(next);
            return;
          }
            return;
          case "filter":
            setFilterState(value);
            return;
          case "tempo": {
            const ratio = 1 + value / 100;
            tempoRef.current = ratio;
            setTempo(ratio);
            return;
          }
        }
      },
      reflectPerformance: (control: string, update: ControllerUpdate) => {
        const mirror = update.performance;
        const cue = control.match(/^(hotcue|clear_hotcue)\.(\d)$/);
        if (cue) {
          const slot = Number(cue[2]);
          if (cue[1] === "clear_hotcue") {
            setHotCues((current) => {
              const next = [...current];
              next[slot] = null;
              return next;
            });
            if (canPersist) dbClearCue(path as string, slot).catch(swallow);
            return;
          }
          const frame = mirror?.hot_cue_frame;
          if (frame == null || !Number.isFinite(frame)) return;
          setHotCues((current) => {
            const next = [...current];
            next[slot] = frame;
            return next;
          });
          if (canPersist) dbSetCue(path as string, slot, frame).catch(swallow);
          return;
        }
        if (mirror?.quantize != null) setQuantize(mirror.quantize);
        if (mirror?.jump_scale != null && Number.isFinite(mirror.jump_scale)) {
          beatJumpScaleRef.current = mirror.jump_scale;
          setBeatJumpScale(mirror.jump_scale);
        }
        if (mirror?.synced != null) {
          setSyncedState(mirror.synced);
          if (mirror.synced) {
            setSyncLiveState(false);
            setSyncInternalState(false);
            setSyncModeState(0);
          }
        }
        if (mirror?.loop_in == null || mirror.loop_out == null) return;
        const next: LoopState = {
          inFrame: mirror.loop_in,
          outFrame: mirror.loop_out,
          active: mirror.loop_active ?? false,
          armed: mirror.loop_armed ?? false,
          beats: mirror.clear_loop_beats ? null : (mirror.loop_beats ?? null),
        };
        loopRef.current = next;
        setLoopState(next);
        if (!canPersist || !next.active) return;
        if (control.startsWith("beatloop.") || control === "loop.out") {
          dbSetLoop(path as string, 0, next.inFrame, next.outFrame, next.beats).catch(swallow);
        }
      },
      setHotCue: (i: number, atFrame = frameRef.current) => {
        setHotCues((cur) => {
          const next = [...cur];
          // Set if empty, else jump to it.
          if (next[i] == null) {
            next[i] = atFrame;
            if (canPersist) dbSetCue(path as string, i, atFrame).catch(swallow);
            performanceSetHotCue(deck, i, atFrame).catch(swallow);
          } else {
            const target = quantizeRef.current ? snapBeat(next[i] as number) : (next[i] as number);
            deckSeek(deck, target).catch(swallow);
          }
          return next;
        });
      },
      clearHotCue: (i: number) => {
        setHotCues((cur) => {
          const next = [...cur];
          next[i] = null;
          return next;
        });
        if (canPersist) dbClearCue(path as string, i).catch(swallow);
        performanceSetHotCue(deck, i, null).catch(swallow);
      },
      beatLoop: (beats: number, atFrame = frameRef.current) => {
        if (!meta || (meta.beat_interval_sec ?? 0) <= 0) return;
        const sr = meta.source_rate;
        const interval = meta.beat_interval_sec * sr;
        const offset = (meta.first_beat_sec + gridOffsetRef.current) * sr;
        const k = Math.round((atFrame - offset) / interval);
        const inFrame = Math.max(0, offset + k * interval);
        const outFrame = inFrame + beats * interval;
        const cur = loopRef.current;
        if (cur.active && cur.beats === beats) {
          setLoopActiveCmd(deck, false).catch(swallow);
          const next = { ...cur, active: false };
          setLoopState(next);
          publishLoop(next);
        } else {
          setLoopCmd(deck, inFrame, outFrame, true).catch(swallow);
          const next = { active: true, beats, inFrame, outFrame };
          setLoopState(next);
          publishLoop(next);
          if (canPersist) dbSetLoop(path as string, 0, inFrame, outFrame, beats).catch(swallow);
        }
      },
      // Arm the loop: store the in-point and light IN until OUT lands (no engine loop yet).
      loopIn: (atFrame = frameRef.current) => {
        const next = { ...loopRef.current, inFrame: atFrame, beats: null, armed: true };
        loopRef.current = next;
        setLoopState(next);
        publishLoop(next);
      },
      loopOut: (atFrame = frameRef.current) => {
        const out = atFrame;
        const l = loopRef.current;
        if (out > l.inFrame) {
          setLoopCmd(deck, l.inFrame, out, true).catch(swallow);
          const next = { active: true, beats: null, inFrame: l.inFrame, outFrame: out, armed: false };
          loopRef.current = next;
          setLoopState(next);
          publishLoop(next);
          if (canPersist) dbSetLoop(path as string, 0, l.inFrame, out, null).catch(swallow);
        }
      },
      toggleLoop: () => {
        const l = loopRef.current;
        if (l.outFrame <= l.inFrame) return;
        const next = { ...l, active: !l.active, armed: false };
        setLoopCmd(deck, l.inFrame, l.outFrame, next.active).catch(swallow);
        loopRef.current = next;
        setLoopState(next);
        publishLoop(next);
      },
      // Pioneer SHIFT+RELOOP: return to the stored loop, engage it and stop on its in-point.
      reloopAndStop: () => {
        const l = loopRef.current;
        if (l.outFrame <= l.inFrame) return;
        deckPause(deck).catch(swallow);
        setLoopCmd(deck, l.inFrame, l.outFrame, true).catch(swallow);
        deckSeek(deck, l.inFrame).catch(swallow);
        const next = { ...l, active: true, armed: false };
        loopRef.current = next;
        setLoopState(next);
        publishLoop(next);
      },
      clearLoop: () => {
        setLoopActiveCmd(deck, false).catch(swallow);
        setLoopState((l) => {
          const next = { ...l, active: false, armed: false };
          publishLoop(next);
          return next;
        });
        if (canPersist) dbClearLoop(path as string, 0).catch(swallow);
      },
      // Halve/double the active loop, anchored at the loop-in. Mirror the new length locally.
      scaleLoop: (factor: number) => {
        scaleLoopCmd(deck, factor).catch(swallow);
        setLoopState((l) => {
          if (l.outFrame <= l.inFrame) return l;
          const len = Math.max(8, (l.outFrame - l.inFrame) * factor);
          const next = {
            ...l,
            outFrame: l.inFrame + len,
            beats: l.beats != null ? l.beats * factor : null,
          };
          publishLoop(next);
          return l.active ? next : l;
        });
      },
      // Shift the active loop by whole beats (or ~50 ms when there's no grid).
      moveLoop: (deltaBeats: number) => {
        const g = grid();
        const delta = g ? g.interval * deltaBeats : (meta?.source_rate ?? 44100) * 0.05 * Math.sign(deltaBeats);
        moveLoopCmd(deck, delta).catch(swallow);
        setLoopState((l) => {
          if (!l.active) return l;
          const next = { ...l, inFrame: Math.max(0, l.inFrame + delta), outFrame: l.outFrame + delta };
          publishLoop(next);
          return next;
        });
      },
      // Main CUE button (press/release) → engine cue state machine (CDJ preview / gated stutter).
      cueButton: (pressed: boolean) => cueButtonCmd(deck, pressed).catch(swallow),
      setCueMode: (mode: number) => {
        setCueModeState(mode);
        setCueModeCmd(deck, mode === 1 ? 1 : 0).catch(swallow);
      },
      // Sync mode: full tempo+phase vs tempo-only.
      setSyncMode: (mode: number) => {
        setSyncModeState(mode);
        setDeckSyncModeCmd(deck, mode === 1 ? 1 : 0).catch(swallow);
      },
      // Pin/unpin this deck as the explicit sync leader.
      toggleLeader: () => {
        const next = !isLeader;
        setIsLeaderState(next);
        setSyncLeaderCmd(deck, next).catch(swallow);
      },
      toggleQuantize: () => setQuantize((q) => {
        const next = !q;
        performanceSetQuantize(deck, next).catch(swallow);
        return next;
      }),
      // Jump the play-head by whole beats (negative = back). When quantize is on, the jump
      // starts from the nearest beat so repeated jumps stay grid-aligned.
      beatJump: (beats: number, fromFrame = frameRef.current) => {
        const g = grid();
        if (!g) return;
        const scaled = beats * beatJumpScaleRef.current;
        const base = quantizeRef.current ? snapBeat(fromFrame) : fromFrame;
        deckSeek(deck, Math.max(0, base + scaled * g.interval)).catch(swallow);
      },
      scaleBeatJump: (factor: number) => {
        if (!Number.isFinite(factor) || factor <= 0) return;
        const next = Math.min(16, Math.max(1 / 16, beatJumpScaleRef.current * factor));
        beatJumpScaleRef.current = next;
        setBeatJumpScale(next);
        performanceSetJumpScale(deck, next).catch(swallow);
      },
      setTempoRange: (percent: number) => {
        const allowed = [6, 8, 10, 16, 100];
        const match = allowed.find((range) => Math.abs(range - percent) < 0.01);
        if (match != null) setTempoRangeState(match);
      },
      nudgeLoopEdge: (edge: "in" | "out", counts: number) => {
        if (!Number.isFinite(counts) || counts === 0) return;
        const current = loopRef.current;
        if (current.outFrame <= current.inFrame) return;
        const sourceRate = meta?.source_rate ?? 0;
        if (sourceRate <= 0) return;
        const delta = counts * sourceRate * 0.01;
        const minimum = 8;
        const next =
          edge === "in"
            ? {
                ...current,
                beats: null,
                inFrame: Math.min(current.outFrame - minimum, Math.max(0, current.inFrame + delta)),
              }
            : {
                ...current,
                beats: null,
                outFrame: Math.max(current.inFrame + minimum, current.outFrame + delta),
              };
        if (next.inFrame === current.inFrame && next.outFrame === current.outFrame) return;
        loopRef.current = next;
        setLoopState(next);
        publishLoop(next);
        setLoopCmd(deck, next.inFrame, next.outFrame, next.active).catch(swallow);
      },
      // Momentary loop-roll with slip. Engaging loops the sub-beat region at the play-head;
      // releasing drops back in where the track would be (the engine tracks the shadow head).
      loopRoll: (beats: number, active: boolean) => {
        if (!active) {
          setLoopRollCmd(deck, 0, 0, false).catch(swallow);
          return;
        }
        const g = grid();
        if (!g) return;
        const step = g.interval * beats;
        const inFrame = Math.max(0, g.offset + Math.floor((frameRef.current - g.offset) / step) * step);
        setLoopRollCmd(deck, inFrame, inFrame + step, true).catch(swallow);
      },
      toggleEcho: () => {
        const next = { ...echoRef.current, active: !echoRef.current.active };
        echoRef.current = next;
        setEchoState(next);
        pushEcho(next);
      },
      setEchoBeats: (beats: number) => {
        const next = { ...echoRef.current, beats };
        echoRef.current = next;
        setEchoState(next);
        pushEcho(next);
      },
      setEchoDepth: (depth: number) => {
        const next = { ...echoRef.current, depth };
        echoRef.current = next;
        setEchoState(next);
        pushEcho(next);
      },
      toggleReverb: () => {
        const next = { ...reverbRef.current, active: !reverbRef.current.active };
        reverbRef.current = next;
        setReverbState(next);
        pushReverb(next);
      },
      setReverbSize: (size: number) => {
        const next = { ...reverbRef.current, size };
        reverbRef.current = next;
        setReverbState(next);
        pushReverb(next);
      },
      setReverbMix: (mix: number) => {
        const next = { ...reverbRef.current, mix };
        reverbRef.current = next;
        setReverbState(next);
        pushReverb(next);
      },
      toggleFlanger: () => {
        const next = { ...flangerRef.current, active: !flangerRef.current.active };
        flangerRef.current = next;
        setFlangerState(next);
        pushFlanger(next);
      },
      setFlangerBeats: (beats: number) => {
        const next = { ...flangerRef.current, beats };
        flangerRef.current = next;
        setFlangerState(next);
        pushFlanger(next);
      },
      setFlangerDepth: (depth: number) => {
        const next = { ...flangerRef.current, depth };
        flangerRef.current = next;
        setFlangerState(next);
        pushFlanger(next);
      },
      toggleCrusher: () => {
        const next = { ...crusherRef.current, active: !crusherRef.current.active };
        crusherRef.current = next;
        setCrusherState(next);
        pushCrusher(next);
      },
      setCrusherCrush: (crush: number) => {
        const next = { ...crusherRef.current, crush };
        crusherRef.current = next;
        setCrusherState(next);
        pushCrusher(next);
      },
      setCrusherDown: (down: number) => {
        const next = { ...crusherRef.current, down };
        crusherRef.current = next;
        setCrusherState(next);
        pushCrusher(next);
      },
    };
  }, [deck, playing, tempo, meta, isLeader, syncLive, syncInternal, applyEcho, applyFlanger]);

  const state: DeckState = useMemo(() => ({ meta, frame, playing, level, rate, latencySecs, frameAt, tempo, tempoRange, beatJumpScale, keylock, pitchShift, gridOffset, synced, syncLive, syncInternal, quantize, cueMode, syncMode, isLeader, xfaderAssign, eq, filter, gain, trim, volume, hotCues, loop, echo, reverb, flanger, crusher, error, loading, dsp }), [meta, frame, playing, level, rate, latencySecs, frameAt, tempo, tempoRange, beatJumpScale, keylock, pitchShift, gridOffset, synced, syncLive, syncInternal, quantize, cueMode, syncMode, isLeader, xfaderAssign, eq, filter, gain, trim, volume, hotCues, loop, echo, reverb, flanger, crusher, error, loading, dsp]);
  return useMemo(() => ({ state, actions, deck }), [state, actions, deck]);
}

const METER_STEPS = [1, 0.5011872, 0.25118864, 0.06309573, 0.015848932];
function meterStep(level: number) {
  const peak = Number.isFinite(level) ? Math.max(0, level) : 0;
  const step = METER_STEPS.findIndex((threshold) => peak >= threshold);
  return step < 0 ? METER_STEPS.length : step;
}

export type DeckController = ReturnType<typeof useDeck>;
