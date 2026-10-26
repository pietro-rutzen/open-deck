import type { DbTrack } from "./ipc";

const PITCH_NAMES = new Set(["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]);

const FLATS: Record<string, string> = {
  DB: "C#",
  EB: "D#",
  GB: "F#",
  AB: "G#",
  BB: "A#",
};

const camelotCode = (token: string) => {
  const match = /^([1-9]|1[0-2])([AB])$/i.exec(token.trim());
  return match ? `${match[1]}${match[2].toUpperCase()}` : null;
};

const musicalNames = (token: string) => {
  const compact = token.replace(/\s+/g, "").replaceAll("♯", "#").replaceAll("♭", "b");
  const lower = compact.toLowerCase();
  const suffix = lower.endsWith("minor")
    ? "minor"
    : lower.endsWith("major")
      ? "major"
      : lower.endsWith("min") && lower.length > 3
        ? "min"
        : lower.endsWith("maj") && lower.length > 3
          ? "maj"
          : lower.endsWith("m") && lower.length > 1
            ? "m"
            : "";
  const minor = suffix === "minor" || suffix === "min" || suffix === "m";
  const note = compact.slice(0, compact.length - suffix.length).toUpperCase();
  const sharp = FLATS[note] ?? note;
  if (!PITCH_NAMES.has(sharp)) return [];
  return minor ? [`${sharp}m`, `${sharp} minor`] : [sharp, `${sharp} major`];
};

const parseBpmEndpoint = (raw: string) => {
  const value = Number(raw.trim().replace(",", "."));
  return Number.isFinite(value) && value > 0 && value < 400 ? value : null;
};

const bpmWindow = (token: string): { min: number; max: number; inclusiveMax: boolean } | null => {
  const range = token.split("-");
  if (range.length === 2) {
    const first = parseBpmEndpoint(range[0] ?? "");
    const second = parseBpmEndpoint(range[1] ?? "");
    if (first == null || second == null) return null;
    return { min: Math.min(first, second), max: Math.max(first, second), inclusiveMax: true };
  }
  const normalized = token.trim().replace(",", ".");
  if (normalized.includes("-")) return null;
  const value = parseBpmEndpoint(normalized);
  if (value == null) return null;
  const fraction = normalized.split(".")[1];
  if (fraction == null) return { min: value, max: value + 1, inclusiveMax: false };
  if (fraction.length === 0) return null;
  const half = 10 ** -fraction.length / 2;
  return { min: value - half, max: value + half, inclusiveMax: false };
};

const bpmHits = (bpm: number | null, window: { min: number; max: number; inclusiveMax: boolean }) =>
  bpm != null && Number.isFinite(bpm) && bpm >= window.min && (window.inclusiveMax ? bpm <= window.max : bpm < window.max);

const keyHits = (track: DbTrack, token: string) => {
  const code = camelotCode(token);
  if (code && track.key_camelot?.toUpperCase() === code) return true;
  const name = track.key_name?.toUpperCase();
  return name != null && musicalNames(token).some((candidate) => candidate.toUpperCase() === name);
};

const textHits = (value: string, needle: string) => value.toLowerCase().includes(needle.toLowerCase());

type Term =
  | { kind: "text"; value: string; negated: boolean }
  | { kind: "bpm"; window: { min: number; max: number; inclusiveMax: boolean }; negated: boolean }
  | { kind: "key"; value: string; negated: boolean }
  | { kind: "field"; field: "artist" | "title" | "tag"; value: string; negated: boolean };

const parseTerm = (raw: string): Term | null => {
  const negated = raw.startsWith("-") && raw.length > 1;
  const token = negated ? raw.slice(1) : raw;
  const fieldAt = token.indexOf(":");
  if (fieldAt >= 0) {
    const field = token.slice(0, fieldAt).toLowerCase();
    const value = token.slice(fieldAt + 1);
    if (!value) return null;
    if (field === "bpm") {
      const window = bpmWindow(value);
      return window ? { kind: "bpm", window, negated } : null;
    }
    if (field === "key") return { kind: "key", value, negated };
    if (field === "artist" || field === "title" || field === "tag") return { kind: "field", field, value, negated };
    return null;
  }
  return { kind: "text", value: token, negated };
};

const textTermMatches = (track: DbTrack, value: string) => {
  const window = bpmWindow(value);
  return textHits(track.title, value)
    || textHits(track.artist, value)
    || (window != null && bpmHits(track.bpm, window))
    || keyHits(track, value);
};

const termMatches = (track: DbTrack, term: Term) => {
  const matched = term.kind === "text"
    ? textTermMatches(track, term.value)
    : term.kind === "bpm"
      ? bpmHits(track.bpm, term.window)
      : term.kind === "key"
        ? keyHits(track, term.value)
        : term.field === "tag"
          ? track.tags.some((tag) => tag.toLowerCase() === term.value.toLowerCase())
          : textHits(term.field === "artist" ? track.artist : track.title, term.value);
  return term.negated ? !matched : matched;
};

/** Tracks matching the library search box, including a tempo or key typed as shown. */
export const filterLibraryTracks = (tracks: readonly DbTrack[], query: string) => {
  const groups = query.split(/\s+/).reduce<Term[][]>((collected, raw) => {
    if (!raw) return collected;
    if (raw === "OR" || raw === "|") return [...collected, []];
    const term = parseTerm(raw);
    if (!term) return collected;
    const groups = [...collected];
    const current = groups[groups.length - 1] ?? [];
    groups[groups.length - 1] = [...current, term];
    return groups;
  }, [[]]);
  const active = groups.filter((group) => group.length > 0);
  if (active.length === 0) return [...tracks];
  return tracks.filter((track) => active.some((group) => group.every((term) => termMatches(track, term))));
};
