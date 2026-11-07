/** Identity of one deck load. A second load of the same path gets a new generation. */
export interface LoadIdentity {
  deck: number;
  generation: number;
  path: string;
}

export interface AnalysisFields {
  bpm: number;
  bpm_confidence: number;
  first_beat_sec: number;
  beat_interval_sec: number;
  key_camelot: string;
  key_name: string;
  band_peaks: [number, number, number][];
  replay_gain: number;
  analysis_pending: boolean;
  analysis_failed?: boolean;
}

export interface AnalysisPatch extends LoadIdentity {
  bpm: number;
  bpm_confidence: number;
  first_beat_sec: number;
  beat_interval_sec: number;
  key_camelot: string;
  key_name: string;
  band_peaks: [number, number, number][];
  replay_gain: number;
  replay_gain_applied: boolean;
}

export const sameLoad = (current: LoadIdentity | null, next: LoadIdentity) =>
  current != null &&
  current.deck === next.deck &&
  current.generation === next.generation &&
  current.path === next.path;

/** A pending or failed measurement is not a grid. Sync and beat jumps stay off. */
export const gridAvailable = (
  meta: { analysis_pending?: boolean; bpm: number; beat_interval_sec: number } | null,
) =>
  !!meta &&
  !meta.analysis_pending &&
  Number.isFinite(meta.bpm) &&
  meta.bpm > 0 &&
  Number.isFinite(meta.beat_interval_sec) &&
  meta.beat_interval_sec > 0;

export const bpmLabel = (
  meta: { analysis_pending?: boolean; bpm: number } | null,
  tempo: number,
) => {
  if (!meta) return "—";
  if (meta.analysis_pending) return "…";
  const bpm = meta.bpm * tempo;
  return Number.isFinite(bpm) && bpm > 0 ? bpm.toFixed(2) : "—";
};

export const laneTempo = (
  meta: { analysis_pending?: boolean; bpm: number; key_camelot: string } | null,
  tempo: number,
) => {
  if (!meta) return "— · —";
  if (meta.analysis_pending) return "… · …";
  const bpm = meta.bpm * tempo;
  if (!Number.isFinite(bpm) || bpm <= 0) return "— · —";
  return `${bpm.toFixed(2)} · ${meta.key_camelot || "—"}`;
};

const finiteOr = (value: number, fallback: number) => (Number.isFinite(value) ? value : fallback);

export const mergeDeckAnalysis = <T extends LoadIdentity & AnalysisFields>(
  loaded: T,
  patch: AnalysisPatch,
): T | null => {
  if (!sameLoad(loaded, patch)) return null;
  const interval = finiteOr(patch.beat_interval_sec, 0);
  const bpm = finiteOr(patch.bpm, 0);
  const usable = interval > 0 && bpm > 0;
  return {
    ...loaded,
    analysis_pending: false,
    analysis_failed: false,
    bpm: usable ? bpm : 0,
    bpm_confidence: usable ? finiteOr(patch.bpm_confidence, 0) : 0,
    first_beat_sec: usable ? finiteOr(patch.first_beat_sec, 0) : 0,
    beat_interval_sec: usable ? interval : 0,
    key_camelot: patch.key_camelot,
    key_name: patch.key_name,
    band_peaks: patch.band_peaks,
    replay_gain:
      patch.replay_gain_applied && Number.isFinite(patch.replay_gain)
        ? patch.replay_gain
        : loaded.replay_gain,
  };
};

export const clearPendingAnalysis = <T extends LoadIdentity & AnalysisFields>(
  loaded: T,
  failure: LoadIdentity,
): T | null => {
  if (!sameLoad(loaded, failure) || !loaded.analysis_pending) return null;
  return {
    ...loaded,
    analysis_pending: false,
    analysis_failed: true,
    bpm: 0,
    bpm_confidence: 0,
    first_beat_sec: 0,
    beat_interval_sec: 0,
    key_camelot: "",
    key_name: "",
    band_peaks: [],
  };
};
