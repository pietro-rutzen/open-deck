import { invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { rgbPaths, RGB_WAVE_KEY, waveColumn, waveformScale } from "../lib/waveformDisplay";
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
type Job = { path: string; listeners: Set<Listener>; started: boolean; priority: number };
// Shared by row thumbnails and the inspector: bounded decoded-result cache and one worker (matching the backend visual-analysis guard).
const cache = new Map<string, VisualState>();
const jobs = new Map<string, Job>();
let active = 0;
let cacheBytes = 0;
const CACHE_BYTE_LIMIT = 12 * 1024 * 1024;
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
function pump() {
  for (const job of [...jobs.values()].sort((a, b) => b.priority - a.priority)) {
    if (active >= 1) return;
    if (job.started || !job.listeners.size) continue;
    job.started = true;
    active++;
    invoke<TrackVisuals>("track_visuals", { path: job.path })
      .then((data): VisualState => ({ status: "ready", data }))
      .catch((): VisualState => ({ status: "unavailable" }))
      .then((state) => {
        remember(job.path, state);
        for (const notify of job.listeners) notify(state);
      })
      .finally(() => {
        jobs.delete(job.path);
        active--;
        pump();
      });
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
    job = { path, listeners: new Set(), started: false, priority };
    jobs.set(path, job);
  }
  job.priority = Math.max(job.priority, priority);
  job.listeners.add(listener);
  pump();
  return () => {
    job.listeners.delete(listener);
    if (!job.started && !job.listeners.size) jobs.delete(path);
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

export function MiniWaveform({ peaks, bands = [], label = "Track frequency and amplitude overview" }: { peaks: number[]; bands?: [number, number, number][]; label?: string }) {
  const paths = useMemo(() => {
    const gain = waveformScale(peaks);
    const count = Math.min(peaks.length, 256);
    const points = Array.from({ length: count }, (_, i) => ({
      ...waveColumn(peaks, bands, Math.floor(i * peaks.length / count), Math.floor((i + 1) * peaks.length / count), gain, 19, 'preview'),
      x: ((i + 0.5) * 256) / count,
    }));
    return rgbPaths(points, 20);
  }, [peaks, bands]);
  if (!peaks.length) return <span className="mini-waveform-state">No waveform</span>;
  return <svg className="mini-waveform" viewBox="0 0 256 40" preserveAspectRatio="none" role="img" aria-label={label}>
    <title>{RGB_WAVE_KEY}</title>
    {paths.map(path => <path key={path.fill} {...path} />)}
  </svg>;
}

/** One lazy subscription supplies artwork, tags and waveform for each visible row. */
export function TrackCells({ path, title, artist, color, afterArtist, beforeWave }: { path: string; title: string; artist: string; color?: string; afterArtist?: ReactNode; beforeWave?: ReactNode }) {
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
    {afterArtist}
    {beforeWave}
    <span className="tl-spacer" aria-hidden="true" />
    <span className="tl-mini-wave" style={color ? { color } : undefined}>
      {visual.status === "ready" && visual.data?.peaks.length ? <MiniWaveform peaks={visual.data.peaks} bands={visual.data.band_peaks} /> : <span className="mini-waveform-state">{visual.status === "loading" ? "…" : "—"}</span>}
    </span>
  </>;
}
