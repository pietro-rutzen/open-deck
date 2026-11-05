import { hotCueMarkers } from "../lib/hotCueMarkers";
import { type CSSProperties, memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type PointerEvent } from "react";
import { useElementSize } from "../hooks/useElementSize";
import type { DeckState } from "../hooks/useDeck";
import { rgbPaths, RGB_WAVE_KEY, waveColumn, waveformScale, type WavePoint } from "../lib/waveformDisplay";
import { inTauri, onControllerUpdate } from "../lib/ipc";

const NOW_FRAC = 0.38; // NOW playhead sits 38% from the left; track scrolls under it
const ZOOMS = [4, 8, 16, 32]; // audible seconds visible (preset buttons)
const MIN_VIEW = 2; // most zoomed-in (audible seconds visible)
const MAX_VIEW = 64; // most zoomed-out
const WHEEL_STEP = 0.85; // per wheel notch: <1 zooms in (fewer seconds visible)
const CLICK_STEP = 0.7; // per Ctrl/⌘+click step
const EMPTY_PEAKS: number[] = [];
const EMPTY_BANDS: [number, number, number][] = [];
const WAVE_SPAN = 8;
const WAVE_PAD = 3.2;
const WAVE_SLACK = 0.5;
const WAVE_HEIGHT_KEY = "opendeck.waveform.height.v1";
const MIN_WAVE_HEIGHT = 72;
const DECK_FLOOR = 380;
const DOWNBEAT = "#ff1a1a";
const DOWNBEAT_TEXT = "#ff3b30";

interface PlayheadMotion {
  playing: boolean;
  frame: number;
  frameAt: number;
  rate: number;
  sourceRate: number;
  latencySecs: number;
}

function shownPlayheadSeconds(sample: PlayheadMotion): number {
  if (sample.sourceRate <= 0) return 0;
  const elapsed = sample.frameAt > 0 ? Math.min(0.08, (performance.now() - sample.frameAt) / 1000) : 0;
  const frame = sample.frame + elapsed * sample.rate;
  return Math.max(0, frame - sample.rate * sample.latencySecs) / sample.sourceRate;
}

function playbackStretch(tempo: number): number {
  return tempo > 0.05 ? tempo : 1;
}

interface Lane {
  state: DeckState;
  letter: string;
  color: string;
  onSeek: (frac: number) => void;
  onNudgeGrid?: (deltaSec: number) => void;
  onResetGrid?: () => void;
}

export function WaveformZone({ lanes }: { lanes: Lane[] }) {
  const [zoneRef, zoneSize] = useElementSize<HTMLDivElement>();
  const drag = useRef<{ startY: number; startHeight: number } | null>(null);
  const [view, setView] = useState(8);
  const [height, setHeight] = useState<number | null>(() => {
    try {
      const saved = localStorage.getItem(WAVE_HEIGHT_KEY);
      const value = saved === null ? NaN : Number(saved);
      return Number.isFinite(value) && value >= MIN_WAVE_HEIGHT ? value : null;
    } catch {
      return null;
    }
  });
  const zoomBy = useCallback((factor: number) => {
    setView((v) => Math.min(MAX_VIEW, Math.max(MIN_VIEW, v * factor)));
  }, []);
  useEffect(() => {
    if (!inTauri()) return;
    const unlisten = onControllerUpdate(({ control, value }) => {
      if (control === "waveform.zoom" && value !== 0) zoomBy(WHEEL_STEP ** value);
    });
    return () => {
      unlisten.then((stop) => stop());
    };
  }, [zoomBy]);
  useEffect(() => {
    try {
      if (height == null) localStorage.removeItem(WAVE_HEIGHT_KEY);
      else localStorage.setItem(WAVE_HEIGHT_KEY, String(Math.round(height)));
    } catch { /* The strip still resizes for this session. */ }
  }, [height]);
  const maxHeight = () => {
    const parent = zoneRef.current?.parentElement;
    const floor = parent ? parseFloat(getComputedStyle(parent).getPropertyValue("--deck-control-floor")) || DECK_FLOOR : DECK_FLOOR;
    return Math.max(MIN_WAVE_HEIGHT, (parent?.clientHeight ?? MIN_WAVE_HEIGHT + floor) - floor);
  };
  const limitHeight = (pixels: number) => Math.min(maxHeight(), Math.max(MIN_WAVE_HEIGHT, pixels));
  // Highlight the preset closest to the current (possibly wheel-adjusted) view.
  const nearest = ZOOMS.reduce((a, b) => (Math.abs(b - view) < Math.abs(a - view) ? b : a));
  return (
    <div className="wf-zone" ref={zoneRef} style={height == null ? undefined : { height }}>
      <div className="wf-zoom">
        {ZOOMS.map((z) => (
          <button key={z} className={`wf-zoom-btn ${z === nearest ? "wf-zoom-btn--on" : ""}`} onClick={() => setView(z)}>
            {z}s
          </button>
        ))}
      </div>
      {/* single NOW playhead spanning both lanes */}
      <div className="wf-now" />
      <div className="wf-now-tag mono">NOW</div>
      {lanes.map((lane, i) => (
        <WaveLane key={i} lane={lane} view={view} zoomBy={zoomBy} />
      ))}
      <div
        className="wf-resize"
        role="separator"
        tabIndex={0}
        aria-orientation="horizontal"
        aria-label="Resize waveform"
        aria-valuemin={MIN_WAVE_HEIGHT}
        aria-valuenow={Math.round(zoneSize.height)}
        title="Drag to resize the waveform · double-click to reset"
        onDoubleClick={() => setHeight(null)}
        onPointerDown={(event) => {
          const zone = zoneRef.current;
          if (!zone || event.button !== 0) return;
          event.preventDefault();
          event.stopPropagation();
          event.currentTarget.setPointerCapture(event.pointerId);
          drag.current = { startY: event.clientY, startHeight: zone.getBoundingClientRect().height };
        }}
        onPointerMove={(event) => {
          const gesture = drag.current;
          if (!gesture) return;
          setHeight(limitHeight(gesture.startHeight + event.clientY - gesture.startY));
        }}
        onPointerUp={(event) => {
          drag.current = null;
          if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
        }}
        onPointerCancel={() => { drag.current = null; }}
        onLostPointerCapture={() => { drag.current = null; }}
        onKeyDown={(event) => {
          if (!["ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) return;
          event.preventDefault();
          const current = zoneRef.current?.getBoundingClientRect().height ?? MIN_WAVE_HEIGHT;
          const max = maxHeight();
          if (event.key === "Home") setHeight(MIN_WAVE_HEIGHT);
          else if (event.key === "End") setHeight(max);
          else setHeight(limitHeight(current + (event.key === "ArrowDown" ? 16 : -16)));
        }}
      >
        <span aria-hidden="true" />
      </div>
    </div>
  );
}

const WaveGraphic = memo(function WaveGraphic({
  peaks,
  bands,
  origin,
  binSec,
  view,
  width,
  height,
  beatInterval,
  beatOffset,
  stretch,
}: {
  peaks: number[];
  bands: [number, number, number][];
  origin: number;
  binSec: number;
  view: number;
  width: number;
  height: number;
  beatInterval: number;
  beatOffset: number;
  stretch: number;
}) {
  const displayScale = useMemo(() => waveformScale(peaks), [peaks]);
  const centerY = height / 2;
  const pixelsPerSecond = width / Math.max(view, 0.001);
  const span = view * WAVE_SPAN;
  const paths = useMemo(() => {
    if (binSec <= 0 || peaks.length === 0 || width <= 0 || stretch <= 0) return [];
    const sourceOrigin = origin * stretch;
    const sourceEnd = (origin + span) * stretch;
    const first = Math.max(0, Math.floor(sourceOrigin / binSec));
    const last = Math.min(peaks.length - 1, Math.ceil(sourceEnd / binSec));
    if (last < first) return [];
    const columns = Math.max(1, Math.ceil(width * WAVE_SPAN * 1.15));
    const step = Math.max(1, Math.ceil((last - first + 1) / columns));
    const points: WavePoint[] = [];
    for (let index = first; index <= last; index += step) {
      const end = Math.min(last, index + step - 1);
      const column = waveColumn(peaks, bands, index, end + 1, displayScale, centerY * 0.94);
      points.push({ ...column, x: (column.loudest * binSec / stretch - origin) * pixelsPerSecond });
    }
    return rgbPaths(points, centerY);
  }, [bands, binSec, centerY, displayScale, origin, peaks, pixelsPerSecond, span, stretch, width]);
  const beats = useMemo(() => {
    if (beatInterval <= 0.05 || width <= 0 || stretch <= 0) return [];
    const end = origin + span;
    const marks: { x: number; down: boolean; bar: number; beat: number }[] = [];
    let index = Math.floor((origin * stretch - beatOffset) / beatInterval) - 1;
    for (let count = 0; count < 4096; count += 1, index += 1) {
      const time = (beatOffset + index * beatInterval) / stretch;
      if (time > end) break;
      if (time < origin || time < 0) continue;
      const beatInBar = ((index % 4) + 4) % 4;
      marks.push({
        x: (time - origin) * pixelsPerSecond,
        down: beatInBar === 0,
        bar: Math.floor(index / 4) + 1,
        beat: beatInBar + 1,
      });
    }
    return marks;
  }, [beatInterval, beatOffset, origin, pixelsPerSecond, span, stretch, width]);
  const showBeatNumber = beatInterval / view * width > 45;
  return (
    <g>
      <title>{RGB_WAVE_KEY}</title>
      {paths.map(path => <path key={path.fill} {...path} />)}
      <g className="wf-beat-grid" aria-label="Beat grid, grouped in four beats">
        {beats.map((beat) => (
          <g key={`${beat.bar}-${beat.beat}-${beat.x.toFixed(1)}`}>
            <line x1={beat.x} y1={0} x2={beat.x} y2={height} stroke={beat.down ? DOWNBEAT : "#e6ebf1"} strokeWidth={beat.down ? 1.5 : 0.8} strokeDasharray={beat.down ? undefined : "2 4"} opacity={beat.down ? 1 : 0.38} vectorEffect="non-scaling-stroke" />
            <path d={`M ${beat.x - 3} 0 L ${beat.x + 3} 0 L ${beat.x} 5 Z M ${beat.x - 3} ${height} L ${beat.x + 3} ${height} L ${beat.x} ${height - 5} Z`} fill={beat.down ? DOWNBEAT : "#d9e0e8"} opacity={beat.down ? 1 : 0.7} />
            {(beat.down || showBeatNumber) && (
              <text x={beat.x + 6} y={12} fill={beat.down ? DOWNBEAT_TEXT : "#d9e0e8"} fontSize="10" fontWeight={beat.down ? 700 : 400} stroke="#080a0d" strokeWidth="3" paintOrder="stroke">
                {beat.down ? `${beat.bar}.1` : beat.beat}
              </text>
            )}
          </g>
        ))}
      </g>

    </g>
  );
});

const WaveLane = memo(function WaveLane({ lane, view, zoomBy }: { lane: Lane; view: number; zoomBy: (factor: number) => void }) {
  const { state, color, letter, onSeek, onNudgeGrid, onResetGrid } = lane;
  const [laneRef, size] = useElementSize<HTMLDivElement>(1368, 86);
  const VW = size.width;
  const VH = Math.max(1, size.height - 29);
  const meta = state.meta;
  const streaming = !state.dsp;
  const effBpm = meta ? meta.bpm * state.tempo : 0;
  const stretch = playbackStretch(state.tempo);

  const sr = meta?.source_rate ?? 1;
  const frames = meta?.frames ?? 0;
  const duration = frames > 0 ? frames / sr : 0;
  const peaks = meta?.peaks ?? EMPTY_PEAKS;
  const binSec = peaks.length > 0 && duration > 0 ? duration / peaks.length : 0;
  const bands = meta?.band_peaks ?? EMPTY_BANDS;
  const beatInterval = meta?.beat_interval_sec ?? 0;
  const beatOffset = (meta?.first_beat_sec ?? 0) + state.gridOffset;
  const [origin, setOrigin] = useState(0);
  const scrollRef = useRef<HTMLDivElement>(null);
  const motionRef = useRef<PlayheadMotion & { view: number; width: number; origin: number; stretch: number }>({
    playing: state.playing,
    frame: state.frame,
    frameAt: state.frameAt,
    rate: state.rate,
    sourceRate: sr,
    latencySecs: state.latencySecs,
    view,
    width: VW,
    origin,
    stretch,
  });
  motionRef.current = {
    playing: state.playing,
    frame: state.frame,
    frameAt: state.frameAt,
    rate: state.rate,
    sourceRate: sr,
    latencySecs: state.latencySecs,
    view,
    width: VW,
    origin,
    stretch,
  };
  const placeScroll = () => {
    const motion = motionRef.current;
    const playhead = shownPlayheadSeconds(motion) / motion.stretch;
    const edge = playhead - NOW_FRAC * motion.view;
    const dx = -((edge - motion.origin) * (motion.width / Math.max(motion.view, 0.001)));
    const node = scrollRef.current;
    if (!node) return;
    node.style.width = `${motion.width * WAVE_SPAN}px`;
    node.style.transform = `translate3d(${dx.toFixed(2)}px, 0, 0)`;
  };
  useLayoutEffect(() => {
    const motion = motionRef.current;
    const playhead = shownPlayheadSeconds(motion) / motion.stretch;
    const edge = playhead - NOW_FRAC * motion.view;
    const span = motion.view * WAVE_SPAN;
    const slack = motion.view * WAVE_SLACK;
    const covers = edge > origin + slack && edge + motion.view < origin + span - slack;
    if (!covers) setOrigin(edge - motion.view * WAVE_PAD);
    placeScroll();
  }, [origin, view, VW, state.frame, state.frameAt, state.playing, state.rate, state.latencySecs, state.tempo, sr]);
  useEffect(() => {
    let raf = 0;
    const tick = () => {
      placeScroll();
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, []);

  // Grab-scrub the lane like vinyl: pointer-down anchors at the current play position, and dragging
  // moves the track under the fixed NOW needle (drag right → rewind). Using a frozen anchor (startSec
  // captured at press) avoids the feedback loop an absolute clientX→time mapping would cause as the
  // seek shifts t0 under the cursor. A press with no real movement falls back to a needle-drop.
  const drag = useRef<{ startX: number; startSec: number; moved: boolean } | null>(null);
  const needleDrop = (clientX: number, r: DOMRect) => {
    const frac = (clientX - r.left) / r.width;
    const motion = motionRef.current;
    const playhead = shownPlayheadSeconds(motion);
    const sourceView = view * motion.stretch;
    const target = playhead - NOW_FRAC * sourceView + frac * sourceView;
    onSeek(Math.min(1, Math.max(0, target / duration)));
  };

  // Wheel zoom: React binds `onWheel` passively, so a native non-passive listener is needed to
  // preventDefault (otherwise the gesture would also scroll the page). Scroll up zooms in.
  useEffect(() => {
    const el = laneRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      zoomBy(e.deltaY < 0 ? WHEEL_STEP : 1 / WHEEL_STEP);
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [zoomBy]);

  const onDown = (e: PointerEvent<HTMLDivElement>) => {
    if (duration <= 0) return;
    // Ctrl/⌘+click steps zoom (hold Shift to zoom out) instead of scrubbing.
    if (e.ctrlKey || e.metaKey) {
      zoomBy(e.shiftKey ? 1 / CLICK_STEP : CLICK_STEP);
      return;
    }
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = { startX: e.clientX, startSec: shownPlayheadSeconds(motionRef.current), moved: false };
  };
  const onMove = (e: PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d || duration <= 0) return;
    const r = e.currentTarget.getBoundingClientRect();
    const dx = e.clientX - d.startX;
    if (Math.abs(dx) > 3) d.moved = true;
    if (!d.moved) return;
    const target = d.startSec - (dx / r.width) * view * motionRef.current.stretch;
    onSeek(Math.min(1, Math.max(0, target / duration)));
  };
  const onUp = (e: PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    drag.current = null;
    try {
      e.currentTarget.releasePointerCapture(e.pointerId);
    } catch {
      // capture may already be lost (e.g. pointercancel) — nothing to release
    }
    if (d && !d.moved && duration > 0) needleDrop(e.clientX, e.currentTarget.getBoundingClientRect());
  };

  return (
    <div className="wf-lane" style={{ "--deck-color": color } as CSSProperties} ref={laneRef} onPointerDown={onDown} onPointerMove={onMove} onPointerUp={onUp} onPointerCancel={onUp}>
      {peaks.length > 0 && (
        <div ref={scrollRef} className="wf-scroll">
          <svg className="wf-svg" viewBox={`0 0 ${Math.max(1, VW * WAVE_SPAN)} ${VH}`} preserveAspectRatio="none">
            <WaveGraphic peaks={peaks} bands={bands} origin={origin} binSec={binSec} view={view} width={VW} height={VH} beatInterval={beatInterval} beatOffset={beatOffset} stretch={stretch} />
            {state.loop.active && state.loop.outFrame > state.loop.inFrame && (
              <rect
                x={(state.loop.inFrame / sr / stretch - origin) * (VW / Math.max(view, 0.001))}
                y={0}
                width={((state.loop.outFrame - state.loop.inFrame) / sr / stretch) * (VW / Math.max(view, 0.001))}
                height={VH}
                fill={color}
                fillOpacity={0.12}
                stroke={color}
                strokeWidth={1}
                vectorEffect="non-scaling-stroke"
              />
            )}
            {!state.loop.active && state.loop.armed && (
              <line
                x1={(state.loop.inFrame / sr / stretch - origin) * (VW / Math.max(view, 0.001))}
                x2={(state.loop.inFrame / sr / stretch - origin) * (VW / Math.max(view, 0.001))}
                y1={0}
                y2={VH}
                stroke={color}
                strokeWidth={2}
                vectorEffect="non-scaling-stroke"
              />
            )}
      <g className="wf-hot-cues" pointerEvents="none" aria-label="Hot cues">
        {hotCueMarkers(state.hotCues, sr, stretch, origin, view, VW, view * WAVE_SPAN).map(({ slot, x, color: cueColor }) => (
          <g key={slot} aria-label={`Hot cue ${slot + 1}`}>
            <title>{`Hot cue ${slot + 1}`}</title>
            <line x1={x} x2={x} y1={0} y2={VH} stroke="#080a0d" strokeWidth={4} vectorEffect="non-scaling-stroke" />
            <line x1={x} x2={x} y1={0} y2={VH} stroke={cueColor} strokeWidth={2} vectorEffect="non-scaling-stroke" />
            <path d={`M ${x} ${VH} L ${x} ${Math.max(0, VH - 17)} L ${x + 19} ${Math.max(0, VH - 17)} L ${x + 19} ${Math.max(0, VH - 4)} Z`} fill={cueColor} />
            <text x={x + 5} y={VH - 6} fill="#080a0d" fontSize={10} fontWeight={700}>{slot + 1}</text>
          </g>
        ))}
      </g>
          </svg>
        </div>
      )}
      <div className="wf-scrim" style={{ width: `${NOW_FRAC * 100}%` }} />
      {streaming && <div className="wf-hatch" />}
      <div className="wf-label">
        <span className="wf-chip" style={{ color, background: `color-mix(in srgb, ${color} 12%, transparent)`, borderColor: color }}>{letter}</span>
        <span className="mono wf-bpm">
          {meta && effBpm > 0 ? `${effBpm.toFixed(2)} · ${meta.key_camelot}` : "— · —"}
        </span>
        <span className="wf-title">{meta ? `${meta.title} — ${meta.artist}` : "No track loaded"}</span>
        {streaming && <span className="wf-badge">STREAM · CONTROL-ONLY</span>}
        {meta && beatInterval > 0.05 && onNudgeGrid && (
          <span className="wf-grid-edit" onPointerDown={(e) => e.stopPropagation()} title="Nudge the beatgrid to line it up with the audio">
            <button onClick={() => onNudgeGrid(-0.005)}>◀</button>
            <button onClick={onResetGrid} className="wf-grid-reset">
              GRID{Math.abs(state.gridOffset) > 1e-4 ? ` ${state.gridOffset > 0 ? "+" : ""}${(state.gridOffset * 1000).toFixed(0)}ms` : ""}
            </button>
            <button onClick={() => onNudgeGrid(0.005)}>▶</button>
          </span>
        )}
      </div>
    </div>
  );
}, (previous, next) => previous.view === next.view && previous.zoomBy === next.zoomBy && previous.lane.state === next.lane.state && previous.lane.onSeek === next.lane.onSeek && previous.lane.onNudgeGrid === next.lane.onNudgeGrid && previous.lane.onResetGrid === next.lane.onResetGrid && previous.lane.color === next.lane.color && previous.lane.letter === next.lane.letter);
