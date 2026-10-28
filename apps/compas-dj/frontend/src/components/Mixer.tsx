import { useEffect, useMemo, useRef, useState, type DragEvent } from "react";
import type { DeckController } from "../hooks/useDeck";
import type { CueApi } from "../hooks/useCue";
import type { BoothApi } from "../hooks/useBooth";
import type { AuxApi } from "../hooks/useAux";
import type { InternalClockApi } from "../hooks/useInternalClock";
import { dbListTracks, deckPlay, inTauri, loadTrack, onDeckLoaded, onLibraryChanged, transposeKey, type DbTrack } from "../lib/ipc";
import { listeningModeActive, subscribeListeningMode } from "../lib/listeningMode";
import { baseName, combineNextTracks, directoryOf, readFolderOrders, reorderPaths, savedFolderOrder, subscribeFolderOrders, suggestNextTracks } from "../lib/libraryFolders";
import { forgetQueuedPath, nextOrderKey, readNextQueueSnapshot, replaceQueuedPaths, setManualNextOrder, subscribeNextQueue } from "../lib/nextQueue";
import { TRIM_MAX } from "../lib/trim";
import { Knob } from "./Knob";
import { Fader } from "./Fader";
import { Meter } from "./Meter";
import { useTrackVisuals } from "./LibraryVisuals";
import { Icon } from "./icons";

interface Channel {
  ctrl: DeckController;
  letter: string;
  color: string;
}

interface AutoMixProps {
  enabled: boolean;
  transitioning: boolean;
  onToggle: () => void;
  onMixNow: () => void;
}

interface XfaderConfig {
  curve: number;
  additive: boolean;
  reverse: boolean;
  onChange: (curve: number, additive: boolean, reverse: boolean) => void;
}

export function Mixer({
  channels,
  crossfader,
  onCrossfader,
  xfader,
  onFxMacro,
  auto,
  cue,
  quad = false,
  loadedPaths,
}: {
  channels: Channel[];
  crossfader: number;
  onCrossfader: (v: number) => void;
  xfader?: XfaderConfig;
  onFxMacro?: (deck: number, v: number) => void;
  auto?: AutoMixProps;
  cue?: CueApi;
  /** 4-deck layout: lay the strips out 2×2 and render them compact. */
  quad?: boolean;
  loadedPaths?: (string | undefined)[];
}) {
  const [options, setOptions] = useState(false);
  return (
    <section className={`mixer mixer--hardware ${options ? "mixer--options" : ""}`}>
      <div className="mixer-head">
        <span className="overline">MIXER · {channels.length}CH</span>
        <button className="chip mixer-options-toggle" aria-expanded={options} onClick={() => setOptions(v => !v)} title="Mixer routing, effects and automatic transitions">Options</button>
      </div>
      <div className={`mixer-strips ${quad ? "mixer-strips--quad" : ""}`}>
        {channelPairs(channels).map((pair) => {
          const [left, right] = pair;
          if (!left) return null;
          return (
            <div className="mixer-pair" key={left.letter}>
              <ChannelStrip {...left} cue={cue} onFxMacro={onFxMacro} compact={quad} options={options} />
              {right && <ChannelStrip {...right} cue={cue} onFxMacro={onFxMacro} compact={quad} options={options} />}
            </div>
          );
        })}
      </div>
      <div className="xfader">
        <span className="overline" style={{ color: "var(--deck-a)" }}>A</span>
        <Fader
          value={crossfader}
          min={0}
          max={1}
          onChange={onCrossfader}
          orientation="horizontal"
          fill
          center
          color="var(--text-primary)"
        />
        <span className="overline" style={{ color: "var(--deck-b)" }}>B</span>
      </div>
      {options && <div className="mixer-options">
        {auto && (
          <div className="automix">
            <button
              className={`chip ${auto.enabled ? "chip--on" : ""}`}
              onClick={auto.onToggle}
              title="Auto-mix: beatmatch + bass-swap crossfade into the other deck near track end"
            >
              AUTO
            </button>
            <button
              className="chip"
              onClick={auto.onMixNow}
              disabled={auto.transitioning}
              title="Transition to the other deck now"
            >
              {auto.transitioning ? "MIXING…" : "MIX"}
            </button>
          </div>
        )}
        {xfader && <XfaderConfigRow {...xfader} />}
      </div>}
      {cue && <Phones cue={cue} />}
      <NextTracks channels={channels} loadedPaths={loadedPaths} limit={quad ? 2 : 4} />
    </section>
  );
}

const channelPairs = (channels: Channel[]) => {
  const pairs: Channel[][] = [];
  for (let index = 0; index < channels.length; index += 2) pairs.push(channels.slice(index, index + 2));
  return pairs;
};

function useLibraryTracks(): DbTrack[] {
  const [tracks, setTracks] = useState<DbTrack[]>([]);
  useEffect(() => {
    if (!inTauri()) return;
    let alive = true;
    let stop: (() => void) | undefined;
    const load = () => {
      dbListTracks().then((rows) => { if (alive) setTracks(rows); }).catch(() => {});
    };
    load();
    onLibraryChanged(load).then((unlisten) => {
      if (alive) stop = unlisten;
      else unlisten();
    }).catch(() => {});
    return () => {
      alive = false;
      stop?.();
    };
  }, []);
  return tracks;
}

const REPEAT_SUGGESTIONS_KEY = "opendeck.next.repeatSuggestions";

const readRepeatedSuggestions = () => {
  try {
    return localStorage.getItem(REPEAT_SUGGESTIONS_KEY) === "1";
  } catch {
    return false;
  }
};

const writeRepeatedSuggestions = (allowed: boolean) => {
  try {
    localStorage.setItem(REPEAT_SUGGESTIONS_KEY, allowed ? "1" : "0");
  } catch {
    /* The choice still applies until reload. */
  }
};

const audibleBpm = (fileBpm: number | null, tempo: number) =>
  fileBpm != null && tempo > 0 ? fileBpm * tempo : fileBpm;

/** Key the deck is sounding: explicit key shift, or the tempo fader when key lock is off and no shift is set. */
const audibleCamelot = (
  camelot: string | null | undefined,
  name: string | null | undefined,
  pitchShift: number,
  tempo: number,
  keylock: boolean,
) => {
  const semitones = !keylock && pitchShift === 0 && tempo > 0 ? Math.round(12 * Math.log2(tempo)) : pitchShift;
  return transposeKey(camelot, name, semitones).camelot;
};

function NextTracks({
  channels,
  loadedPaths,
  limit,
}: {
  channels: Channel[];
  loadedPaths?: (string | undefined)[];
  limit: number;
}) {
  const tracks = useLibraryTracks();
  const deckAFollow = (): Record<number, boolean> => (listeningModeActive() ? { 0: true } : {});
  const [listening, setListening] = useState(listeningModeActive);
  const [queueOn, setQueueOn] = useState<Record<number, boolean>>(deckAFollow);
  const [playOn, setPlayOn] = useState<Record<number, boolean>>(deckAFollow);
  useEffect(() => subscribeListeningMode(() => {
    const active = listeningModeActive();
    setListening(active);
    setQueueOn((current) => ({ ...current, 0: active }));
    setPlayOn((current) => ({ ...current, 0: active }));
  }), []);
  const [nextQueue, setNextQueue] = useState(readNextQueueSnapshot);
  useEffect(() => subscribeNextQueue(() => setNextQueue(readNextQueueSnapshot())), []);
  const [folderOrders, setFolderOrders] = useState(readFolderOrders);
  const [allowRepeats, setAllowRepeats] = useState(readRepeatedSuggestions);
  useEffect(() => subscribeFolderOrders(() => setFolderOrders(readFolderOrders())), []);
  const [dropKey, setDropKey] = useState<string | null>(null);
  const draggingPath = useRef<string | null>(null);
  const playAfterLoad = useRef(new Map<number, string>());
  const playOnRef = useRef(playOn);
  playOnRef.current = playOn;
  useEffect(() => {
    if (!inTauri()) return;
    let alive = true;
    let stop: (() => void) | undefined;
    onDeckLoaded((event) => {
      const expected = playAfterLoad.current.get(event.deck);
      if (!expected || expected !== event.path) return;
      playAfterLoad.current.delete(event.deck);
      if (playOnRef.current[event.deck]) deckPlay(event.deck).catch(() => {});
    }).then((unlisten) => {
      if (alive) stop = unlisten;
      else unlisten();
    }).catch(() => {});
    return () => {
      alive = false;
      stop?.();
    };
  }, []);
  const busyPaths = loadedPaths ?? channels.map((channel) => channel.ctrl.state.meta?.path);
  const loadedIdentityKey = channels.map((channel) => `${channel.ctrl.deck}:${channel.ctrl.state.meta?.path ?? ""}`).join("\0");
  useEffect(() => {
    for (const entry of loadedIdentityKey.split("\0")) {
      const splitAt = entry.indexOf(":");
      const deck = Number(entry.slice(0, splitAt));
      const path = entry.slice(splitAt + 1);
      if (path) forgetQueuedPath(deck, path);
    }
  }, [loadedIdentityKey]);
  const columns = useMemo(() => {
    const loaded = busyPaths.filter((path): path is string => Boolean(path));
    return channels.reduce<{ rows: { channel: Channel; suggestions: DbTrack[] }[]; taken: ReadonlySet<string> }>((state, channel) => {
      const meta = channel.ctrl.state.meta;
      const libraryTrack = tracks.find((track) => track.path === meta?.path);
      const fileBpm = libraryTrack?.bpm != null && libraryTrack.bpm > 0 ? libraryTrack.bpm : (meta && meta.bpm > 0 ? meta.bpm : null);
      const reference = audibleBpm(fileBpm, channel.ctrl.state.tempo);
      const referenceKey = meta
        ? audibleCamelot(
            libraryTrack?.key_camelot || meta.key_camelot,
            libraryTrack?.key_name || meta.key_name,
            channel.ctrl.state.pitchShift,
            channel.ctrl.state.tempo,
            channel.ctrl.state.keylock,
          )
        : null;
      const blocked = allowRepeats ? loaded : [...loaded, ...state.taken];
      const queuedPaths = nextQueue.queuedByDeck[channel.ctrl.deck] ?? [];
      const queuedTracks = queuedPaths.flatMap((path) => {
        if (path === meta?.path) return [];
        const track = tracks.find((item) => item.path === path);
        return track ? [track] : [];
      });
      const ranked = meta
        ? suggestNextTracks(
            tracks,
            meta.path,
            reference,
            blocked,
            limit,
            listening ? 0.2 : 1,
            listening ? null : referenceKey,
            savedFolderOrder(folderOrders, directoryOf(meta.path)),
          )
        : [];
      const suggestions = combineNextTracks(
        ranked,
        queuedTracks,
        meta ? nextQueue.manualOrder[nextOrderKey(channel.ctrl.deck, meta.path)] : undefined,
      );
      const taken = allowRepeats
        ? state.taken
        : new Set([...state.taken, ...suggestions.map((track) => track.path), ...queuedPaths]);
      return { rows: [...state.rows, { channel, suggestions }], taken };
    }, { rows: [], taken: new Set() }).rows;
  }, [channels, tracks, busyPaths, limit, nextQueue, listening, allowRepeats, folderOrders]);

  return (
    <div className={`mixer-next ${channels.length > 2 ? "mixer-next--quad" : ""}`}>
      <div className="next-head">
        <h3 className="overline">Next</h3>
        <label className="next-repeat" title="Allow the same track to be suggested on more than one deck. Off, each track is offered to one deck.">
          <input
            type="checkbox"
            checked={allowRepeats}
            onChange={(event) => {
              const allowed = event.target.checked;
              setAllowRepeats(allowed);
              writeRepeatedSuggestions(allowed);
            }}
          />
          Repeat suggestions
        </label>
      </div>
      <div className="next-grid">
      {columns.map(({ channel, suggestions }) => (
        <section className="mixer-next-col" key={channel.letter} aria-label={`Next tracks for deck ${channel.letter}`}>
          <div className="next-col-head">
            <div className={`next-modes next-modes--${channel.letter.toLowerCase()}`} role="group" aria-label={`Auto load for deck ${channel.letter}`}>
              <button
                type="button"
                className={`chip ${queueOn[channel.ctrl.deck] ? "chip--on" : ""}`}
                aria-pressed={!!queueOn[channel.ctrl.deck]}
                title="When this deck's track ends, load the next suggestion paused"
                onClick={() => setQueueOn((current) => ({ ...current, [channel.ctrl.deck]: !current[channel.ctrl.deck] }))}
              >
                Queue
              </button>
              <button
                type="button"
                className={`chip ${playOn[channel.ctrl.deck] ? "chip--on" : ""}`}
                aria-pressed={!!playOn[channel.ctrl.deck]}
                title="When this deck's track ends, load the next suggestion and start playback"
                onClick={() => setPlayOn((current) => ({ ...current, [channel.ctrl.deck]: !current[channel.ctrl.deck] }))}
              >
                Play
              </button>
            </div>
          </div>
          <DeckFollow
            deck={channel.ctrl.deck}
            playing={channel.ctrl.state.playing}
            frame={channel.ctrl.state.frame}
            frames={channel.ctrl.state.meta?.frames ?? 0}
            loading={channel.ctrl.state.loading}
            nextPath={suggestions[0]?.path ?? null}
            queue={!!queueOn[channel.ctrl.deck]}
            play={!!playOn[channel.ctrl.deck]}
            playAfterLoad={playAfterLoad}
          />
          {suggestions.length === 0 ? (
            <p className="next-empty">
              {channel.ctrl.state.meta == null
                ? `Load a track on deck ${channel.letter}.`
                : "No other tracks in this folder."}
            </p>
          ) : (
            <div className="next-list">
              {suggestions.map((track) => {
                const queued = (nextQueue.queuedByDeck[channel.ctrl.deck] ?? []).includes(track.path);
                return (
                <NextCard
                  key={track.path}
                  track={track}
                  deck={channel.ctrl.deck}
                  letter={channel.letter}
                  queued={queued}
                  dropping={dropKey === `${channel.ctrl.deck}:${track.path}`}
                  onRemove={() => forgetQueuedPath(channel.ctrl.deck, track.path)}
                  onDragBegin={(event) => {
                    draggingPath.current = track.path;
                    event.dataTransfer.setData("application/x-compas-track", track.path);
                    event.dataTransfer.setData("text/plain", track.path);
                    event.dataTransfer.effectAllowed = "copyMove";
                  }}
                  onDragFinish={() => {
                    draggingPath.current = null;
                    setDropKey(null);
                  }}
                  onDragHover={(event) => {
                    const from = draggingPath.current;
                    if (!from || from === track.path || !suggestions.some((item) => item.path === from)) return;
                    event.preventDefault();
                    event.dataTransfer.dropEffect = "move";
                    const key = `${channel.ctrl.deck}:${track.path}`;
                    if (dropKey !== key) setDropKey(key);
                  }}
                  onDropTrack={(event) => {
                    event.preventDefault();
                    const from = event.dataTransfer.getData("application/x-compas-track") || draggingPath.current;
                    setDropKey(null);
                    if (!from || from === track.path) return;
                    if (!suggestions.some((item) => item.path === from)) return;
                    const next = reorderPaths(suggestions.map((item) => item.path), from, track.path);
                    if (!next) return;
                    const loadedPath = channel.ctrl.state.meta?.path;
                    if (loadedPath) setManualNextOrder(channel.ctrl.deck, loadedPath, next);
                    else replaceQueuedPaths(channel.ctrl.deck, next);
                  }}
                />
                );
              })}
            </div>
          )}
        </section>
      ))}
      </div>
    </div>
  );
}

function DeckFollow({
  deck,
  playing,
  frame,
  frames,
  loading,
  nextPath,
  queue,
  play,
  playAfterLoad,
}: {
  deck: number;
  playing: boolean;
  frame: number;
  frames: number;
  loading: boolean;
  nextPath: string | null;
  queue: boolean;
  play: boolean;
  playAfterLoad: { current: Map<number, string> };
}) {
  const wasPlaying = useRef(playing);
  useEffect(() => {
    const ended = wasPlaying.current && !playing && frames > 0 && frame >= frames - 2;
    wasPlaying.current = playing;
    if (!ended || loading || !nextPath || (!queue && !play)) return;
    if (play) playAfterLoad.current.set(deck, nextPath);
    else playAfterLoad.current.delete(deck);
    loadTrack(deck, nextPath).catch(() => {
      playAfterLoad.current.delete(deck);
    });
  }, [deck, playing, frame, frames, loading, nextPath, queue, play, playAfterLoad]);
  return null;
}

function NextCard({
  track,
  deck,
  letter,
  queued,
  dropping,
  onRemove,
  onDragBegin,
  onDragFinish,
  onDragHover,
  onDropTrack,
}: {
  track: DbTrack;
  deck: number;
  letter: string;
  queued: boolean;
  dropping: boolean;
  onRemove: () => void;
  onDragBegin: (event: DragEvent<HTMLDivElement>) => void;
  onDragFinish: () => void;
  onDragHover: (event: DragEvent<HTMLDivElement>) => void;
  onDropTrack: (event: DragEvent<HTMLDivElement>) => void;
}) {
  const visual = useTrackVisuals(track.path, true, true, 0);
  const artwork = visual.data?.artwork_data_url;
  const title = visual.data?.title || track.title || baseName(track.path);
  const artist = visual.data?.artist || track.artist || "Unknown artist";
  const tempo = track.bpm != null && track.bpm > 0 ? track.bpm.toFixed(2) : null;
  const skipClick = useRef(false);
  const load = () => { loadTrack(deck, track.path).catch(() => {}); };
  return (
    <div
      role="button"
      tabIndex={0}
      draggable
      className={`next-card${queued ? " next-card--queued" : ""}${dropping ? " next-card--drop" : ""}`}
      onClick={() => { if (!skipClick.current) load(); }}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          load();
        }
      }}
      onDragStart={(event) => {
        skipClick.current = true;
        onDragBegin(event);
      }}
      onDragEnd={() => {
        onDragFinish();
        window.setTimeout(() => { skipClick.current = false; }, 0);
      }}
      onDragEnter={onDragHover}
      onDragOver={onDragHover}
      onDrop={onDropTrack}
      title={queued
        ? `Queued on deck ${letter}. With Queue or Play on, it loads when the track on that deck ends. Drag to reorder.`
        : `Load ${title} onto deck ${letter}. Drag onto a deck, or onto another suggestion to change what plays next.`}
    >
      <span className="next-art" aria-hidden="true">
        {artwork ? <img src={artwork} alt="" draggable={false} /> : <span>♫</span>}
      </span>
      <span className="next-copy">
        <span className="next-title">{title}</span>
        <span className="next-artist">{artist}</span>
      </span>
      <span className="next-side">
        {tempo && <span className="next-bpm">{tempo}</span>}
        {queued && (
          <button
            type="button"
            className="next-queue-remove"
            aria-label={`Remove ${title} from deck ${letter}'s queue`}
            title="Remove from this deck's Next queue"
            onMouseDown={(event) => event.stopPropagation()}
            onClick={(event) => {
              event.stopPropagation();
              onRemove();
            }}
          >
            ×
          </button>
        )}
      </span>
    </div>
  );
}

/** Internal master clock: a free-running metronome offered as a virtual sync leader (the deck INT
 *  chips lock to it), so decks can ride a global tempo with nothing playing. */
export function InternalClock({ clock }: { clock: InternalClockApi }) {
  return (
    <div className="phones int-clock">
      <Icon name="link" size={14} />
      <span className="overline" style={{ flex: "none" }}>INT CLK</span>
      <button
        className={`chip phones-on ${clock.active ? "chip--on" : ""}`}
        onClick={clock.toggle}
        title={clock.active ? "Internal master clock running — decks on INT lock to it" : "Start the internal master clock"}
      >
        {clock.active ? "ON" : "OFF"}
      </button>
      <input
        className="int-bpm mono"
        type="number"
        min={40}
        max={220}
        step={0.5}
        value={clock.bpm}
        onChange={(e) => {
          const v = parseFloat(e.target.value);
          if (Number.isFinite(v)) clock.setBpm(v);
        }}
        title="Internal clock tempo (BPM)"
      />
    </div>
  );
}

/** Aux / microphone input: on/off and input level (summed into the master). */
export function Aux({ aux }: { aux: AuxApi }) {
  return (
    <div className="phones aux">
      <Icon name="mic" size={14} />
      <button
        className={`chip phones-on ${aux.enabled ? "chip--on" : ""}`}
        onClick={aux.toggle}
        title={aux.enabled ? `Aux on: ${aux.connectedName ?? ""}` : "Start aux/mic input"}
      >
        {aux.enabled ? "ON" : "OFF"}
      </button>
      {aux.enabled && (
        <span
          className="aux-bpm mono"
          title={
            aux.live?.locked
              ? `Live beat lock · ${aux.live.bpm.toFixed(2)} BPM (conf ${(aux.live.confidence ?? 0).toFixed(2)})`
              : "Listening for a beat…"
          }
        >
          <span className={`aux-lock ${aux.live?.locked ? "aux-lock--on" : ""}`} />
          {aux.live?.locked ? aux.live.bpm.toFixed(2) : "— —"}
        </span>
      )}
      <Knob value={aux.gain} min={0} max={2} size={22} label="AUX" onChange={aux.setGain} />
    </div>
  );
}

/** Crossfader response: curve steepness (smooth → cut), additive/cut mode, and reverse. */
function XfaderConfigRow({ curve, additive, reverse, onChange }: XfaderConfig) {
  return (
    <div className="xf-config">
      <span className="overline">CURVE</span>
      <input
        className="xf-curve"
        type="range"
        min={0.5}
        max={6}
        step={0.1}
        value={curve}
        onChange={(e) => onChange(parseFloat(e.target.value), additive, reverse)}
        title="Crossfader curve: low = smooth blend, high = sharp cut"
      />
      <button
        className={`chip ${additive ? "chip--on" : ""}`}
        onClick={() => onChange(curve, !additive, reverse)}
        title="Additive (slow-fade / fast-cut) vs constant-power"
      >
        CUT
      </button>
      <button
        className={`chip ${reverse ? "chip--on" : ""}`}
        onClick={() => onChange(curve, additive, !reverse)}
        title="Reverse the crossfader (hamster switch)"
      >
        REV
      </button>
    </div>
  );
}

/** Headphone cue master controls: on/off, cue↔master blend, level. */
function Phones({ cue }: { cue: CueApi }) {
  const route = cue.device ? `${cue.connectedName ?? cue.device} · channels ${cue.firstChannel + 1}/${cue.firstChannel + 2}` : "Select a headphone output in Settings";
  return <>
    <div className="phones">
      <Icon name="headphones" size={14} />
      <button
        className={`chip phones-on ${cue.enabled ? "chip--on" : ""}`}
        onClick={cue.toggle}
        disabled={!cue.device}
        aria-pressed={cue.enabled}
        title={`${cue.enabled ? "Stop" : "Start"} headphone output: ${route}`}
      >
        {cue.enabled ? "ON" : "OFF"}
      </button>
      <Knob value={cue.mix} min={0} max={1} size={22} label="CUE◁▷MAS" onChange={cue.setMix} />
      <Knob value={cue.volume} min={0} max={1} size={22} label="PHONES" onChange={cue.setVolume} />
    </div>
    {cue.error ? <div role="alert" style={{ fontSize: 10 }}>{cue.error}</div>
      : !cue.device && <div role="status" style={{ fontSize: 10 }}>Select headphone output in Settings</div>}
  </>;
}

/** Booth monitor controls: on/off and post-master monitor level. */
export function Booth({ booth }: { booth: BoothApi }) {
  return (
    <div className="phones booth">
      <Icon name="sliders" size={14} />
      <button
        className={`chip phones-on ${booth.enabled ? "chip--on" : ""}`}
        onClick={booth.toggle}
        title={booth.enabled ? `Booth on: ${booth.connectedName ?? ""}` : "Start booth output"}
      >
        {booth.enabled ? "ON" : "OFF"}
      </button>
      <Knob value={booth.volume} min={0} max={1} size={22} label="BOOTH" onChange={booth.setVolume} />
    </div>
  );
}

const XF_LABELS = ["A", "│", "B"]; // assign: 0 = A side, 1 = thru, 2 = B side

function ChannelStrip({
  ctrl,
  letter,
  color,
  cue,
  onFxMacro,
  compact = false,
  options = false,
}: Channel & {
  cue?: CueApi;
  onFxMacro?: (deck: number, v: number) => void;
  compact?: boolean;
  options?: boolean;
}) {
  const { state, actions } = ctrl;
  const dsp = state.dsp;
  const cued = !!cue?.cued.has(ctrl.deck);
  const [fxMacro, setFxMacro] = useState(0);
  const ks = compact ? 22 : 26;

  return (
    <div className="strip">
      <div className="strip-level">
        <span className="strip-letter display" style={{ color }}>{letter}</span>
        <button
          className={`cue-btn ${cued ? "cue-btn--on" : ""}`}
          onClick={() => cue?.toggleDeckCue(ctrl.deck)}
          disabled={!cue}
          title="Pre-listen this deck in the headphones (PFL)"
        >
          <span>CUE</span>
        </button>
        <Fader value={state.volume} min={0} max={1} fill color={color} onChange={actions.setVolume} />
      </div>

      <div className="strip-channel">
        {options && <div className="xf-assign" title="Crossfader assign: A side / thru / B side">
          {XF_LABELS.map((lbl, i) => (
            <button
              key={i}
              className={`xf-seg ${state.xfaderAssign === i ? "xf-seg--on" : ""}`}
              style={state.xfaderAssign === i ? { color, borderColor: color } : undefined}
              onClick={() => actions.setXfaderAssign(i)}
            >
              {lbl}
            </button>
          ))}
        </div>}

        <div className="strip-eq">
          <div className={`knob-stack ${compact ? "knob-stack--compact" : ""} ${dsp ? "" : "knob-stack--locked"}`}>
            <Knob label="TRIM" value={state.trim} min={0} max={TRIM_MAX} law="trim" size={ks} color={color} disabled={!dsp}
              onChange={actions.setTrim} />
            <Knob label="HI" value={state.eq.hi} min={-26} max={6} size={ks} color={color} disabled={!dsp}
              onChange={(v) => actions.setEq({ ...state.eq, hi: v })} />
            <Knob label="MID" value={state.eq.mid} min={-26} max={6} size={ks} color={color} disabled={!dsp}
              onChange={(v) => actions.setEq({ ...state.eq, mid: v })} />
            <Knob label="LOW" value={state.eq.low} min={-26} max={6} size={ks} color={color} disabled={!dsp}
              onChange={(v) => actions.setEq({ ...state.eq, low: v })} />
            <Knob label="FILTER" value={state.filter} min={-1} max={1} size={ks} color={color} disabled={!dsp}
              onChange={(v) => actions.setFilter(v)} />
            {options && onFxMacro && (
              <Knob label="FX" value={fxMacro} min={0} max={1} size={ks} color={color} disabled={!dsp}
                onChange={(v) => { setFxMacro(v); onFxMacro(ctrl.deck, v); }} />
            )}
            {!dsp && <span className="eq-na">EQ N/A</span>}
          </div>
          <Meter level={state.level} streaming={!dsp} showClip={false} showLabels />
        </div>
      </div>
    </div>
  );
}
