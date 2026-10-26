import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { TitleBar } from "./components/TitleBar";
import { StatusBar } from "./components/StatusBar";
import { WaveformZone } from "./components/WaveformZone";
import { Deck } from "./components/Deck";
import { Mixer } from "./components/Mixer";
import { Library } from "./components/Library";
import { MixerFrame, WorkspaceSplit } from "./components/WorkspaceSplit";
import { Instrument } from "./components/Instrument";
import { MidiMap } from "./components/MidiMap";
import { ControllerMap } from "./components/ControllerMap";
import { Sampler } from "./components/Sampler";
import { SettingsPanel } from "./components/SettingsPanel";
import { ProfilePanel } from "./components/ProfilePanel";
import { isNativeDeckControl, useDeck, type DeckController } from "./hooks/useDeck";
import { BEAT_FX_LABELS, useBeatFx } from "./hooks/useBeatFx";
import { useAutoMix } from "./hooks/useAutoMix";
import { useCue } from "./hooks/useCue";
import { useBooth } from "./hooks/useBooth";
import { useAux } from "./hooks/useAux";
import { useInternalClock } from "./hooks/useInternalClock";
import { useMidi } from "./hooks/useMidi";
import { useMidiMap } from "./hooks/useMidiMap";
import { useSampler } from "./hooks/useSampler";
import { controllerLearn, controllerSession, onControllerSession, controllerFeedback, engineStatus, inTauri, onControllerBatch, onControllerUpdate, pickRecordingPath, pitchClassOf, setCrossfader, setCrossfaderConfig, setDeckFxMacro, setMasterGain, startRecording, stopRecording, type ControllerUpdate, type EngineStatus, type KeyNotation } from "./lib/ipc";

const DECK_COLORS = ["var(--deck-a)", "var(--deck-b)", "var(--deck-c)", "var(--deck-d)"];
const DECK_LETTERS = ["A", "B", "C", "D"];
const BEAT_LOOP_PADS = [0.25, 0.5, 1, 2, 4, 8, 16, 32];
const BEAT_JUMP_PADS = [-1, 1, -2, 2, -4, 4, -8, 8];
/** Pioneer key-shift pad layout: absolute semitones; pads 1 and 8 reset. */
const KEY_SHIFT_PADS = [0, 1, 2, 3, -3, -2, -1, 0];
/** SHIFT + key-shift pads: relative semitone steps. */
const KEY_SHIFT_STEP_PADS = [1, 2, 3, 4, -1, -2, -3, -4];

export function App() {
  // Four local/full-DSP decks; only two deck panels are shown at a time (switching slots),
  // while the mixer exposes all four channel strips.
  // The internal master clock is a global tempo source decks can follow (INT) and beat-synced FX
  // track; created before the decks so each useDeck can read its tempo.
  const internalClock = useInternalClock();
  const deckA = useDeck(0, true, internalClock);
  const deckB = useDeck(1, true, internalClock);
  const deckC = useDeck(2, true, internalClock);
  const deckD = useDeck(3, true, internalClock);
  const decks = [deckA, deckB, deckC, deckD];

  const [sampleRate, setSampleRate] = useState<number | null>(null);
  const [audioStatus, setAudioStatus] = useState<Pick<EngineStatus, "audio_online" | "audio_restarting" | "audio_restarts" | "audio_error" | "cue_device_latency_secs" | "cue_prime_latency_secs" | "booth_device_latency_secs" | "booth_prime_latency_secs"> | null>(null);
  const [xfade, setXfade] = useState(0.5);
  const [showKeys, setShowKeys] = useState(false);
  const [showMap, setShowMap] = useState(false);
  const [showPads, setShowPads] = useState(false);
  const [showControllers, setShowControllers] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [showProfile, setShowProfile] = useState(false);
  const [contrast, setContrast] = useState(false);
  const [keyNotation, setKeyNotation] = useState<KeyNotation>(
    () => (localStorage.getItem("compas.keyNotation") as KeyNotation | null) ?? "camelot",
  );
  useEffect(() => {
    localStorage.setItem("compas.keyNotation", keyNotation);
  }, [keyNotation]);
  useEffect(() => {
    if (!inTauri()) return;
    controllerLearn(false).catch(() => {});
  }, []);
  const [deckCount, setDeckCount] = useState<2 | 4>(() =>
    localStorage.getItem("compas.deckCount") === "4" ? 4 : 2,
  );
  useEffect(() => {
    localStorage.setItem("compas.deckCount", String(deckCount));
  }, [deckCount]);
  const [recording, setRecording] = useState(false);
  const [recBusy, setRecBusy] = useState(false);
  const [profileName, setProfileName] = useState(() => localStorage.getItem("compas.profileName") ?? "Main");
  const midi = useMidi();
  const cue = useCue();
  const booth = useBooth();
  const aux = useAux();
  const sampler = useSampler();
  // Which deck each on-screen slot controls: left ∈ {A,C}, right ∈ {B,D}.
  const [leftSel, setLeftSel] = useState(0);
  const [rightSel, setRightSel] = useState(1);
  const leftDeck = decks[leftSel];
  const rightDeck = decks[rightSel];

  const applyCrossfade = useCallback((v: number) => {
    setXfade(v);
    if (inTauri()) setCrossfader(v).catch(() => {});
  }, []);

  // Crossfader response config (curve steepness, additive/cut mode, reverse).
  const [xfCurve, setXfCurve] = useState(1);
  const [xfAdditive, setXfAdditive] = useState(false);
  const [xfReverse, setXfReverse] = useState(false);
  const applyXfConfig = useCallback((curve: number, additive: boolean, reverse: boolean) => {
    setXfCurve(curve);
    setXfAdditive(additive);
    setXfReverse(reverse);
    if (inTauri()) setCrossfaderConfig(curve, additive ? 1 : 0, reverse).catch(() => {});
  }, []);
  const auto = useAutoMix([leftDeck, rightDeck], applyCrossfade);

  const applyFxMacro = useCallback((deck: number, v: number) => {
    if (inTauri()) setDeckFxMacro(deck, v).catch(() => {});
  }, []);

  useEffect(() => {
    if (!inTauri()) return;
    let alive = true;
    const refreshStatus = () => {
      engineStatus()
        .then((s) => {
          if (!alive) return;
          setSampleRate(s.sample_rate);
          setAudioStatus({
            audio_online: s.audio_online,
            audio_restarting: s.audio_restarting,
            audio_restarts: s.audio_restarts,
            audio_error: s.audio_error,
            cue_device_latency_secs: s.cue_device_latency_secs,
            cue_prime_latency_secs: s.cue_prime_latency_secs,
            booth_device_latency_secs: s.booth_device_latency_secs,
            booth_prime_latency_secs: s.booth_prime_latency_secs,
          });
        })
        .catch(() => {
          if (!alive) return;
          setSampleRate(0);
          setAudioStatus({
            audio_online: false,
            audio_restarting: false,
            audio_restarts: 0,
            audio_error: "engine status unavailable",
            cue_device_latency_secs: 0,
            cue_prime_latency_secs: 0,
            booth_device_latency_secs: 0,
            booth_prime_latency_secs: 0,
          });
        });
    };
    refreshStatus();
    const statusTimer = window.setInterval(refreshStatus, 2000);
    return () => {
      alive = false;
      window.clearInterval(statusTimer);
    };
  }, []);

  const masterBpm = leftDeck.state.meta ? leftDeck.state.meta.bpm * leftDeck.state.tempo : null;
  const [loadedPathA, loadedPathB, loadedPathC, loadedPathD] = decks.map((d) => d.state.meta?.path);
  const loadedPaths = useMemo(
    () => [loadedPathA, loadedPathB, loadedPathC, loadedPathD],
    [loadedPathA, loadedPathB, loadedPathC, loadedPathD],
  );
  const profileInitial = profileName.trim().charAt(0).toUpperCase() || "M";

  useEffect(() => {
    localStorage.setItem("compas.profileName", profileName);
  }, [profileName]);

  const toggleRecord = useCallback(async () => {
    if (!inTauri() || recBusy) return;
    setRecBusy(true);
    try {
      if (recording) {
        await stopRecording();
        setRecording(false);
      } else {
        const path = await pickRecordingPath();
        if (path) {
          await startRecording(path);
          setRecording(true);
        }
      }
    } catch {
      setRecording(false);
    } finally {
      setRecBusy(false);
    }
  }, [recBusy, recording]);

  const openPanel = (setter: (open: boolean) => void) => {
    setter(true);
    setShowSettings(false);
  };

  // Two decks are sync-pairable when both visible slots are loaded with a tempo.
  const pairReady =
    !!leftDeck.state.meta && !!rightDeck.state.meta && leftDeck.state.meta.bpm > 0 && rightDeck.state.meta.bpm > 0;

  // Continuous beat-sync toggle: `target` follows the other visible deck (engine PLL), or
  // disengages. On engage we also match the displayed tempo; the engine refines phase on top.
  const toggleSync = (target: DeckController, source: DeckController) => {
    if (target.state.synced) {
      target.actions.sync(null);
      return;
    }
    if (!target.state.meta || !source.state.meta || target.state.meta.bpm <= 0 || source.state.meta.bpm <= 0) return;
    target.actions.setSyncMode(0);
    target.actions.setTempo((source.state.meta.bpm * source.state.tempo) / target.state.meta.bpm);
    target.actions.sync(source.deck);
  };

  // Key Sync: key-shift `target` so its sounding tonic matches the `source` deck (incl. the
  // source's own current shift). Picks the nearest transposition in ±6 semitones.
  const keySyncReady = (target: DeckController, source: DeckController) =>
    pitchClassOf(target.state.meta?.key_name) != null && pitchClassOf(source.state.meta?.key_name) != null;
  const keySync = (target: DeckController, source: DeckController) => {
    const tpc = pitchClassOf(target.state.meta?.key_name);
    const spc = pitchClassOf(source.state.meta?.key_name);
    if (tpc == null || spc == null) return;
    let d = (((spc + source.state.pitchShift - tpc) % 12) + 12) % 12;
    if (d > 6) d -= 12;
    target.actions.setPitchShift(d);
  };

  // MIDI SYNC binds per deck index; only the two on-screen decks have a defined partner.
  const syncDeckByIndex = (i: number) => {
    if (i === leftSel) toggleSync(leftDeck, rightDeck);
    else if (i === rightSel) toggleSync(rightDeck, leftDeck);
  };
  const midiMap = useMidiMap(decks, {
    crossfader: applyCrossfade,
    syncDeck: syncDeckByIndex,
    deckCue: cue.toggleDeckCue,
    samplerPad: sampler.trigger,
  });

  const beatFx = useBeatFx();

  // Native controller thread already applied these to the engine; UI state only reflects them.
  const reflectRef = useRef<(u: ControllerUpdate) => void>(() => {});
  reflectRef.current = ({ control, value }: ControllerUpdate) => {
    if (!Number.isFinite(value)) return;
    if (control === "mixer.crossfader") return setXfade(value);
    const deckMatch = control.match(/^deck\.(\d+)\.(.+)$/);
    const deckController = deckMatch ? decks[Number(deckMatch[1])] : undefined;
    if (deckController && deckMatch && isNativeDeckControl(deckMatch[2])) {
      deckController.actions.reflectController(deckMatch[2], value);
    }
  };

  // Controller bus: apply resolved controller:update events through the existing setters. A ref
  // keeps the handler current without re-subscribing the listener on every render.
  const dispatchRef = useRef<(u: ControllerUpdate) => void>(() => {});
  dispatchRef.current = (u: ControllerUpdate) => {
    const { control, value } = u;
    if (!Number.isFinite(value)) return;
    if (beatFx.handle(control, value, decks)) return;
    if (control === "mixer.crossfader") return setXfade(value); // Native controller path already applied audio.
    if (control === "mixer.cue_mix") return cue.setMix(Math.max(0, Math.min(1, value)));
    if (control === "mixer.cue_volume") return cue.setVolume(Math.max(0, Math.min(1, value)));
    if (control === "mixer.master_gain") {
      setMasterGain(value).catch(() => {});
      return;
    }
    if (control === "sampler.gain") {
      sampler.setGain(value);
      return;
    }
    const sm = control.match(/^sampler\.(\d+)\.(trigger|stop)$/);
    if (sm) {
      if (value < 0.5) return;
      if (sm[2] === "trigger") sampler.trigger(parseInt(sm[1], 10));
      else sampler.stop(parseInt(sm[1], 10));
      return;
    }
    const m = control.match(/^deck\.(\d+)\.(.+)$/);
    if (!m) return;
    const d = decks[parseInt(m[1], 10)];
    if (!d) return;
    const { actions: a, state: s } = d;
    const pad = m[2].match(/^(hotcue|clear_hotcue|beatloop|beatjump|key_shift|key_shift_step)\.(\d)$/);
    if (pad) {
      if (value >= 0.5) {
        const padIndex = Number(pad[2]);
        if (pad[1] === "hotcue") a.setHotCue(padIndex, u.heard_frame);
        else if (pad[1] === "clear_hotcue") a.clearHotCue(padIndex);
        else if (pad[1] === "beatloop") a.beatLoop(BEAT_LOOP_PADS[padIndex], u.heard_frame);
        else if (pad[1] === "beatjump") a.beatJump(BEAT_JUMP_PADS[padIndex], u.frame);
        else if (pad[1] === "key_shift") a.setPitchShift(KEY_SHIFT_PADS[padIndex]);
        else a.shiftPitchBy(KEY_SHIFT_STEP_PADS[padIndex]);
      }
      return;
    }
    switch (m[2]) {
      case "loop.in": if (value >= 0.5) a.loopIn(u.heard_frame); break;
      case "loop.out": if (value >= 0.5) a.loopOut(u.heard_frame); break;
      case "loop.toggle": if (value >= 0.5) a.toggleLoop(); break;
      case "loop.reloop_stop": if (value >= 0.5) a.reloopAndStop(); break;
      case "loop.half": if (value >= 0.5) a.scaleLoop(0.5); break;
      case "loop.double": if (value >= 0.5) a.scaleLoop(2); break;
      case "jump.back": if (value >= 0.5) a.beatJump(-4, u.frame); break;
      case "jump.forward": if (value >= 0.5) a.beatJump(4, u.frame); break;
      case "beatjump.scale_down": if (value >= 0.5) a.scaleBeatJump(1 / 16); break;
      case "beatjump.scale_up": if (value >= 0.5) a.scaleBeatJump(16); break;
      case "loop.nudge_in": a.nudgeLoopEdge("in", value); break;
      case "loop.nudge_out": a.nudgeLoopEdge("out", value); break;
      case "tempo_range": a.setTempoRange(value); break;
      case "sync_leader": if (value >= 0.5 && !s.isLeader) a.toggleLeader(); break;
      case "start": if (value >= 0.5) a.seekFrac(0); break;
      case "quantize": if (value >= 0.5) a.toggleQuantize(); break;
      case "gain": a.setGain(value); break; // legacy absolute effective gain
      case "trim": a.setTrim(value); break;
      case "volume": a.setVolume(value); break;
      case "filter": a.setFilter(value); break;
      case "tempo": a.setTempo(1 + value / 100); break;
      case "eq.low": a.setEq({ ...s.eq, low: value }); break;
      case "eq.mid": a.setEq({ ...s.eq, mid: value }); break;
      case "eq.high": a.setEq({ ...s.eq, hi: value }); break;
      case "play": if (value >= 0.5) a.togglePlay(); break; // hardware buttons latch on press
      case "cue": a.cueButton(value >= 0.5); break;
      case "keylock": if (value >= 0.5) a.toggleKeylock(); break;
      case "sync": if (value >= 0.5) syncDeckByIndex(parseInt(m[1], 10)); break;
    }
  };
  useEffect(() => {
    if (!inTauri()) return;
    const un = onControllerUpdate((u) => dispatchRef.current(u));
    const unBatch = onControllerBatch(({ mirror, updates }) => {
      mirror.forEach((update) => reflectRef.current(update));
      updates.forEach((update) => dispatchRef.current(update));
    });
    return () => {
      un.then((u) => u());
      unBatch.then((u) => u());
    };
  }, []);

  // LED/motor feedback: push each mapped control's current value to the device so the hardware
  // tracks software state (lit pads, LED rings, motor faders) — for UI changes, not just the
  // controller's own moves. The engine no-ops without an active profile/output and dedups
  // redundant resends. Tempo state is a ratio; the control bus speaks ±percent. (Master gain has
  // no UI control, so it rides the controller-echo path only.)
  const feedbackValues: [string, number][] = [
    ...decks.flatMap((d): [string, number][] => {
      const s = d.state;
      const prefix = `deck.${d.deck}`;
      return [
        ...s.hotCues.map((hotCue, i): [string, number] => [`${prefix}.hotcue.${i}`, hotCue == null ? 0 : 1]),
        ...BEAT_LOOP_PADS.map((beats, i): [string, number] => [`${prefix}.beatloop.${i}`, s.loop.active && s.loop.beats === beats ? 1 : 0]),
        ...BEAT_JUMP_PADS.map((_, i): [string, number] => [`${prefix}.beatjump.${i}`, s.meta ? 1 : 0]),
        ...KEY_SHIFT_PADS.map((semitones, i): [string, number] => [`${prefix}.key_shift.${i}`, s.meta && s.pitchShift === semitones ? 1 : 0]),
        [`${prefix}.loop.in`, s.loop.active || s.loop.armed ? 1 : 0],
        [`${prefix}.loop.out`, s.loop.active ? 1 : 0],
        [`${prefix}.loop.toggle`, s.loop.active ? 1 : 0],
        [`${prefix}.gain`, s.gain],
        [`${prefix}.trim`, s.trim],
        [`${prefix}.volume`, s.volume],
        [`${prefix}.headphone_cue`, cue.cued.has(d.deck) ? 1 : 0],
        [`${prefix}.filter`, s.filter],
        [`${prefix}.eq.low`, s.eq.low],
        [`${prefix}.eq.mid`, s.eq.mid],
        [`${prefix}.eq.high`, s.eq.hi],
        [`${prefix}.tempo`, (s.tempo - 1) * 100],
        [`${prefix}.play`, s.playing ? 1 : 0],
        [`${prefix}.keylock`, s.keylock ? 1 : 0],
        [`${prefix}.sync`, s.synced ? 1 : 0],
        [`fx.toggle.${d.deck}`, beatFx.isOn(d) ? 1 : 0],
      ];
    }),
    ...sampler.pads.map((pad, i): [string, number] => [`sampler.${i}.trigger`, pad.name ? 1 : 0]),
    ["mixer.crossfader", xfade],
    ["mixer.cue_mix", cue.mix],
    ["mixer.cue_volume", cue.volume],
    ["sampler.gain", sampler.gain],
  ];
  // Only changed values cross IPC; the native side keeps the desired state for reconnects.
  const sentFeedbackRef = useRef(new Map<string, number>());
  const sendFeedback = useCallback((values: [string, number][], force: boolean) => {
    if (!inTauri()) return;
    values
      .filter(([control, value]) => force || sentFeedbackRef.current.get(control) !== value)
      .forEach(([control, value]) => {
        sentFeedbackRef.current.set(control, value);
        controllerFeedback(control, value).catch(() => {});
      });
  }, []);
  useEffect(() => {
    sendFeedback(feedbackValues, false);
  });
  const feedbackValuesRef = useRef(feedbackValues);
  feedbackValuesRef.current = feedbackValues;
  const pushFeedback = useCallback(() => sendFeedback(feedbackValuesRef.current, true), [sendFeedback]);
  // Native discovery may connect before React mounts. Reconcile both the initial
  // session and later reconnects so controller LEDs reflect current software state.
  const feedbackRef = useRef(pushFeedback);
  feedbackRef.current = pushFeedback;
  useEffect(() => {
    if (!inTauri()) return;
    let live = true;
    const un = onControllerSession((session) => {
      if (live && session.connected && session.profile) feedbackRef.current();
    });
    un.then(() => controllerSession()).then((session) => {
      if (live && session.connected && session.profile) feedbackRef.current();
    }).catch(() => {});
    return () => { live = false; un.then((stop) => stop()).catch(() => {}); };
  }, []);
  // …and on demand when a profile is (re)activated, to sync the device to current state at connect.
  useEffect(() => {
    const onResync = () => pushFeedback();
    window.addEventListener("controller:resync", onResync);
    return () => window.removeEventListener("controller:resync", onResync);
  }, [pushFeedback]);

  const slotLane = (d: DeckController) => ({
    state: d.state,
    letter: DECK_LETTERS[d.deck],
    color: DECK_COLORS[d.deck],
    onSeek: d.actions.seekFrac,
    onNudgeGrid: d.actions.nudgeGrid,
    onResetGrid: d.actions.resetGrid,
  });

  const quad = deckCount === 4;
  // In 4-deck mode each deck syncs to the other deck in its row (A↔B, C↔D; partner index = deck ^ 1).
  const fourDeckProps = (d: DeckController) => {
    const partner = decks[d.deck ^ 1];
    const ready =
      !!d.state.meta && !!partner.state.meta && d.state.meta.bpm > 0 && partner.state.meta.bpm > 0;
    return {
      ctrl: d,
      color: DECK_COLORS[d.deck],
      onSync: () => toggleSync(d, partner),
      syncEnabled: ready,
      syncActive: d.state.synced,
      onKeySync: () => keySync(d, partner),
      keySyncEnabled: keySyncReady(d, partner),
      keyNotation,
      compact: true,
    };
  };
  // Mixer props shared by both layouts; only the visible channel set differs (2 strips vs 4).
  const mixerCommon = {
    crossfader: xfade,
    onCrossfader: applyCrossfade,
    xfader: { curve: xfCurve, additive: xfAdditive, reverse: xfReverse, onChange: applyXfConfig },
    onFxMacro: applyFxMacro,
    auto: { enabled: auto.enabled, transitioning: auto.transitioning, onToggle: auto.toggle, onMixNow: auto.mixNow },
    cue,
    loadedPaths,
  };
  const channelOf = (d: DeckController) => ({ ctrl: d, letter: DECK_LETTERS[d.deck], color: DECK_COLORS[d.deck] });

  return (
    <div className="app" data-contrast={contrast ? "high" : "standard"}>
      <TitleBar masterBpm={masterBpm} syncEnabled={pairReady} syncActive={rightDeck.state.synced} onSync={() => toggleSync(rightDeck, leftDeck)} keysOpen={showKeys} onToggleKeys={() => setShowKeys((v) => !v)} mapOpen={showMap} onToggleMap={() => setShowMap((v) => !v)} padsOpen={showPads} onTogglePads={() => setShowPads((v) => !v)} controllersOpen={showControllers} onToggleControllers={() => setShowControllers((v) => !v)} recording={recording} recBusy={recBusy} onToggleRecord={toggleRecord} onOpenSettings={() => setShowSettings(true)} onOpenProfile={() => setShowProfile(true)} profileInitial={profileInitial} contrast={contrast} onToggleContrast={() => setContrast((v) => !v)} />
      <div className="body">
        <div className="content">
          <WorkspaceSplit upper={quad ? (
            <>
              <WaveformZone lanes={[slotLane(deckA), slotLane(deckB), slotLane(deckC), slotLane(deckD)]} />
              <div className="deck-row deck-row--quad">
                <div className="deck-col">
                  <Deck {...fourDeckProps(deckA)} mirror />
                  <Deck {...fourDeckProps(deckC)} mirror />
                </div>
                <MixerFrame>
                  <Mixer channels={decks.map(channelOf)} {...mixerCommon} quad />
                </MixerFrame>
                <div className="deck-col">
                  <Deck {...fourDeckProps(deckB)} />
                  <Deck {...fourDeckProps(deckD)} />
                </div>
              </div>
            </>
          ) : (
            <>
              <WaveformZone lanes={[slotLane(leftDeck), slotLane(rightDeck)]} />
              <div className="deck-row">
                <Deck
                  ctrl={leftDeck}
                  color={DECK_COLORS[leftDeck.deck]}
                  onSync={() => toggleSync(leftDeck, rightDeck)}
                  syncEnabled={pairReady}
                  syncActive={leftDeck.state.synced}
                  onKeySync={() => keySync(leftDeck, rightDeck)}
                  keySyncEnabled={keySyncReady(leftDeck, rightDeck)}
                  keyNotation={keyNotation}
                  mirror
                  slots={[
                    { label: "A", active: leftSel === 0, onSelect: () => setLeftSel(0) },
                    { label: "C", active: leftSel === 2, onSelect: () => setLeftSel(2) },
                  ]}
                />
                <MixerFrame>
                  <Mixer channels={[leftDeck, rightDeck].map(channelOf)} {...mixerCommon} />
                </MixerFrame>
                <Deck
                  ctrl={rightDeck}
                  color={DECK_COLORS[rightDeck.deck]}
                  onSync={() => toggleSync(rightDeck, leftDeck)}
                  syncEnabled={pairReady}
                  syncActive={rightDeck.state.synced}
                  onKeySync={() => keySync(rightDeck, leftDeck)}
                  keySyncEnabled={keySyncReady(rightDeck, leftDeck)}
                  keyNotation={keyNotation}
                  slots={[
                    { label: "B", active: rightSel === 1, onSelect: () => setRightSel(1) },
                    { label: "D", active: rightSel === 3, onSelect: () => setRightSel(3) },
                  ]}
                />
              </div>
            </>
          )} lower={<Library loadedPaths={loadedPaths} keyNotation={keyNotation} />} />
        </div>
      </div>
      <StatusBar
        sampleRate={sampleRate}
        audioStatus={audioStatus}
        beatFxLabel={`${BEAT_FX_LABELS[beatFx.selection.effect]} · ${DECK_LETTERS[beatFx.selection.deck]}`}
      />
      {showKeys && <Instrument midi={midi} onClose={() => setShowKeys(false)} />}
      {showMap && <MidiMap midi={midi} map={midiMap} onClose={() => setShowMap(false)} />}
      {showPads && <Sampler sampler={sampler} onClose={() => setShowPads(false)} />}
      {showControllers && <ControllerMap onClose={() => setShowControllers(false)} />}
      {showSettings && (
        <SettingsPanel
          aux={aux}
          booth={booth}
          cue={cue}
          clock={internalClock}
          contrast={contrast}
          onToggleContrast={() => setContrast((v) => !v)}
          keyNotation={keyNotation}
          onToggleKeyNotation={() => setKeyNotation((v) => (v === "camelot" ? "musical" : "camelot"))}
          deckCount={deckCount}
          onToggleDeckCount={() => setDeckCount((v) => (v === 2 ? 4 : 2))}
          onOpenKeys={() => openPanel(setShowKeys)}
          onOpenMap={() => openPanel(setShowMap)}
          onOpenPads={() => openPanel(setShowPads)}
          onOpenControllers={() => openPanel(setShowControllers)}
          onClose={() => setShowSettings(false)}
        />
      )}
      {showProfile && (
        <ProfilePanel
          name={profileName}
          onName={setProfileName}
          onClose={() => setShowProfile(false)}
        />
      )}
    </div>
  );
}
