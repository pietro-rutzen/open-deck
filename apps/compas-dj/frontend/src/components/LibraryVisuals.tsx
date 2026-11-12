import { invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { RGB_WAVE_KEY, presentAmplitude, smoothAmplitudes, structureGain, trackLevelReference, waveColumn, wavePaths } from "../lib/waveformDisplay";
import { inTauri } from "../lib/ipc";

export interface TrackVisuals {
  peaks: number[];
  band_peaks: [number, number, number][];
  artwork_data_url: string | null;
  title: string | null;
  artist: string | null;
  album: string | null;
}
type VisualState = { status: "loading" | "ready" | "unavailable"; data?: TrackVisuals };
type Listener = (state: VisualState) => void;
type Job = {
  path: string;
  listeners: Set<Listener>;
  started: boolean;
  priority: number;
  order: number;
  attempts: number;
  timer: ReturnType<typeof setTimeout> | null;
};
// Shared by row thumbnails and the inspector. One decode runs at a time, matching
// the backend guard. A busy rejection stays pending and retries; it is not remembered.
export const VISUAL_ATTEMPT_LIMIT = 4;
const VISUAL_RETRY_BASE_MS = 40;
const VISUAL_RETRY_CAP_MS = 800;
const cache = new Map<string, VisualState>();
const jobs = new Map<string, Job>();
let active = 0;
let nextOrder = 0;
let cacheBytes = 0;
const CACHE_BYTE_LIMIT = 12 * 1024 * 1024;

export function visualRetryDelayMs(completedAttempts: number): number {
  const exponent = Math.min(8, Math.max(0, completedAttempts - 1));
  return Math.min(VISUAL_RETRY_CAP_MS, VISUAL_RETRY_BASE_MS * 2 ** exponent);
}

function visualFailureText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "message" in error && typeof error.message === "string") {
    return error.message;
  }
  return "";
}

function isBusyVisualError(error: unknown): boolean {
  return visualFailureText(error).includes("Track visuals are busy");
}
function stateBytes(state: VisualState): number {
  const d = state.data;
  return d ? (d.artwork_data_url?.length ?? 0) * 2 + d.peaks.length * 8 + (d.band_peaks?.length ?? 0) * 24 + 8192 : 256;
}
function remember(path: string, state: VisualState) {
  const previous = cache.get(path);
  if (previous) cacheBytes -= stateBytes(previous);
  cache.delete(path);
  cache.set(path, state);
  cacheBytes += stateBytes(state);
  while (cache.size > 128 || cacheBytes > CACHE_BYTE_LIMIT) {
    const oldest = cache.keys().next().value;
    if (oldest === undefined) break;
    const evicted = cache.get(oldest);
    cacheBytes -= evicted ? stateBytes(evicted) : 0;
    cache.delete(oldest);
    if (cache.size === 0) cacheBytes = 0;
  }
}
function publish(job: Job, state: VisualState, rememberResult: boolean) {
  if (rememberResult) remember(job.path, state);
  for (const notify of job.listeners) notify(state);
  if (job.timer != null) {
    clearTimeout(job.timer);
    job.timer = null;
  }
  jobs.delete(job.path);
}

function releaseWorker(job: Job) {
  job.started = false;
  active = Math.max(0, active - 1);
}

function pump() {
  const ordered = [...jobs.values()].sort(
    (left, right) => right.priority - left.priority || left.attempts - right.attempts || left.order - right.order,
  );
  for (const job of ordered) {
    if (active >= 1) return;
    if (job.started || job.timer != null || !job.listeners.size) continue;
    job.started = true;
    job.attempts += 1;
    active += 1;
    invoke<TrackVisuals>("track_visuals", { path: job.path })
      .then(
        (data) => {
          releaseWorker(job);
          publish(job, { status: "ready", data }, true);
          pump();
        },
        (error: unknown) => {
          releaseWorker(job);
          const retry = isBusyVisualError(error) && job.attempts < VISUAL_ATTEMPT_LIMIT && job.listeners.size > 0;
          if (!retry) {
            publish(job, { status: "unavailable" }, !isBusyVisualError(error));
            pump();
            return;
          }
          for (const notify of job.listeners) notify({ status: "loading" });
          job.timer = setTimeout(() => {
            job.timer = null;
            if (!job.listeners.size) {
              jobs.delete(job.path);
              return;
            }
            pump();
          }, visualRetryDelayMs(job.attempts));
          pump();
        },
      );
  }
}
function subscribe(path: string, listener: Listener, priority = 0) {
  const saved = cache.get(path);
  if (saved) {
    remember(path, saved);
    listener(saved);
    return () => {};
  }
  if (!inTauri()) {
    listener({ status: "unavailable" });
    return () => {};
  }
  let job = jobs.get(path);
  if (!job) {
    job = { path, listeners: new Set(), started: false, priority, order: nextOrder, attempts: 0, timer: null };
    nextOrder += 1;
    jobs.set(path, job);
  }
  job.priority = Math.max(job.priority, priority);
  job.listeners.add(listener);
  pump();
  return () => {
    job.listeners.delete(listener);
    if (job.listeners.size) return;
    if (job.timer != null) {
      clearTimeout(job.timer);
      job.timer = null;
    }
    if (!job.started) jobs.delete(path);
  };
}
export function useTrackVisuals(path: string | null, enabled = true, includeArtwork = true, priority = includeArtwork ? 1 : 0): VisualState {
  const [result, setResult] = useState<{ path: string; state: VisualState } | null>(null);
  useEffect(() => {
    if (!path || !enabled) { setResult(null); return; }
    let current = true;
    const unsubscribe = subscribe(path, (state) => {
      if (current) setResult({ path, state: !includeArtwork && state.data
        ? { ...state, data: { ...state.data, artwork_data_url: null } } : state });
    }, priority);
    return () => { current = false; unsubscribe(); };
  }, [path, enabled, includeArtwork, priority]);
  return result?.path === path ? result.state : { status: "loading" };
}

export function MiniWaveform({ peaks, bands = [], bandScale = 1, label = "Track frequency and amplitude overview" }: { peaks: number[]; bands?: [number, number, number][]; bandScale?: number; label?: string }) {
  const paths = useMemo(() => {
    const count = Math.min(peaks.length, 256);
    const radius = 19;
    const reference = trackLevelReference(peaks, bands);
    const points = Array.from({ length: count }, (_, column) => {
      const start = Math.floor(column * peaks.length / count);
      const end = Math.max(start + 1, Math.floor((column + 1) * peaks.length / count));
      const measured = waveColumn(peaks, bands, start, end, radius);
      return {
        ...measured,
        amp: presentAmplitude(measured.amp, reference, radius),
        x: ((column + 0.5) * 256) / count,
      };
    });
    return wavePaths(smoothAmplitudes(points), 20, structureGain(bands), bandScale, 'baseline');
  }, [peaks, bands, bandScale]);
  if (!peaks.length) return <span className="mini-waveform-state">No waveform</span>;
  return <svg className="mini-waveform" viewBox="0 0 256 40" preserveAspectRatio="none" role="img" aria-label={label}>
    <title>{RGB_WAVE_KEY}</title>
    {paths.map(path => <path key={path.fill} {...path} />)}
  </svg>;
}

/** One lazy subscription supplies artwork, tags and waveform for each visible row. */
export function TrackCells({ path, title, artist, color, bpmCell, keyCell }: { path: string; title: string; artist: string; color?: string; bpmCell?: ReactNode; keyCell?: ReactNode }) {
  const ref = useRef<HTMLSpanElement>(null);
  const [visible, setVisible] = useState(false);
  useEffect(() => {
    const el = ref.current;
    if (!el || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver(([entry]) => setVisible(entry.isIntersecting), {
      root: el.closest(".tl-sheet"), rootMargin: "120px 0px",
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [path]);
  const visual = useTrackVisuals(path, visible, true, 0);
  const [failedArtwork, setFailedArtwork] = useState(false);
  const artwork = visual.data?.artwork_data_url;
  useEffect(() => setFailedArtwork(false), [path, artwork]);
  return <>
    <span className="tl-mini-wave" style={color ? { color } : undefined}>
      {visual.status === "ready" && visual.data?.peaks.length ? <MiniWaveform bandScale={0.5} peaks={visual.data.peaks} bands={visual.data.band_peaks} /> : <span className="mini-waveform-state">{visual.status === "loading" ? "…" : "—"}</span>}
    </span>
    <span ref={ref} className="tl-title tl-title-with-art">
      <span className="tl-artwork" aria-hidden="true">
        {artwork && !failedArtwork
          ? <img src={artwork} alt="" loading="lazy" decoding="async" draggable={false} onError={() => setFailedArtwork(true)} />
          : <span>♫</span>}
      </span>
      <span className="tl-title-copy">
        <span className="tl-title-text">{visual.data?.title || title}</span>
        {visual.data?.album && <span className="tl-album" title={visual.data.album}>{visual.data.album}</span>}
      </span>
    </span>
    <span className="tl-artist">{visual.data?.artist || artist || "—"}</span>
    {bpmCell}
    {keyCell}
    <span className="tl-spacer" aria-hidden="true" />
  </>;
}
