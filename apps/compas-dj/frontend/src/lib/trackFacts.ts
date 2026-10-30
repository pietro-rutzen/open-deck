import type { TrackFileFacts } from "./ipc";

export type FileFactStatus = "loading" | "ready" | "unavailable";

export const formatDuration = (ms: number): string => {
  if (!Number.isFinite(ms) || ms < 0) return "—";
  const seconds = Math.round(ms / 1000);
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
};

export const fileKind = (path: string): string => {
  const name = path.split(/[\\/]/).pop() ?? "";
  const dot = name.lastIndexOf(".");
  if (dot <= 0 || dot === name.length - 1) return "Audio";
  return name.slice(dot + 1).toUpperCase();
};

export const fileStem = (path: string): string => {
  const name = path.split(/[\\/]/).pop() ?? path;
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(0, dot) : name;
};

export const parentFolderName = (path: string): string => {
  const parts = path.split(/[\\/]/).filter((part) => part.length > 0);
  return parts.length < 2 ? "Local file" : parts[parts.length - 2];
};

const decimalSize = (value: number): string =>
  value.toFixed(value < 100 ? 1 : 0);

export const formatBytes = (bytes: number): string => {
  if (!Number.isFinite(bytes) || bytes < 0) return "Unknown";
  if (bytes < 1000) return `${Math.round(bytes)} bytes`;
  if (bytes < 1_000_000) return `${decimalSize(bytes / 1000)} KB`;
  if (bytes < 1_000_000_000) return `${decimalSize(bytes / 1_000_000)} MB`;
  return `${decimalSize(bytes / 1_000_000_000)} GB`;
};

export const formatSampleRate = (hz: number): string => {
  if (!Number.isFinite(hz) || hz <= 0) return "Unknown";
  if (hz % 1000 === 0) return `${hz / 1000} kHz`;
  const kilo = hz / 1000;
  if (Math.abs(kilo * 10 - Math.round(kilo * 10)) < 0.001) return `${kilo.toFixed(1)} kHz`;
  return `${Math.round(hz).toLocaleString()} Hz`;
};

export const formatChannels = (channels: number): string => {
  if (channels === 1) return "Mono";
  if (channels === 2) return "Stereo";
  return `${channels} channels`;
};

const averageBitrateKbps = (sizeBytes: number, durationMs: number): number | null => {
  if (!Number.isFinite(sizeBytes) || !Number.isFinite(durationMs) || sizeBytes <= 0 || durationMs <= 0) {
    return null;
  }
  const kbps = Math.round((sizeBytes * 8) / durationMs);
  return kbps > 0 ? kbps : null;
};

export const formatBitrate = (
  facts: TrackFileFacts | null,
  durationMs: number,
  status: FileFactStatus,
): string => {
  if (facts?.bitrate_kbps != null && facts.bitrate_kbps > 0) {
    const label = `${facts.bitrate_kbps.toLocaleString()} kbps`;
    return facts.bitrate_is_average ? `${label} average` : label;
  }
  if (facts) {
    const average = averageBitrateKbps(facts.size_bytes, durationMs);
    return average != null ? `${average.toLocaleString()} kbps average` : "Unknown";
  }
  return pendingFact(status, null);
};

const fileDate = new Intl.DateTimeFormat(undefined, {
  day: "numeric",
  month: "short",
  year: "numeric",
});

const playedAt = new Intl.DateTimeFormat(undefined, {
  day: "numeric",
  month: "short",
  year: "numeric",
  hour: "numeric",
  minute: "2-digit",
});

export const formatFileDate = (unixMs: number): string => fileDate.format(new Date(unixMs));

export const formatPlayedAt = (unixSeconds: number): string =>
  playedAt.format(new Date(unixSeconds * 1000));

export const formatGain = (gain: number): string | null => {
  if (!Number.isFinite(gain) || gain <= 0 || Math.abs(gain - 1) < 0.001) return null;
  const db = 20 * Math.log10(gain);
  return `${db > 0 ? "+" : ""}${db.toFixed(1)} dB`;
};

export const formatGridOffset = (seconds: number): string | null => {
  if (!Number.isFinite(seconds) || Math.abs(seconds) < 0.0005) return null;
  const ms = Math.round(seconds * 1000);
  return `${ms > 0 ? "+" : ""}${ms} ms`;
};

export const formatIndex = (number: string | null, total: string | null): string | null => {
  if (!number) return null;
  return total ? `${number} of ${total}` : number;
};

export const pendingFact = (status: FileFactStatus, value: string | null): string => {
  if (value) return value;
  if (status === "loading") return "…";
  if (status === "unavailable") return "Unavailable";
  return "Unknown";
};
