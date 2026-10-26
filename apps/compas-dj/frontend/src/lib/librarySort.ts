import type { DbTrack } from "./ipc";

export const SORT_COLUMNS = ["order", "title", "artist", "bpm", "key", "duration"] as const;
export type SortColumn = typeof SORT_COLUMNS[number];
export type LibrarySort = { column: SortColumn; direction: "asc" | "desc" };
export const DEFAULT_SORT: LibrarySort = { column: "order", direction: "asc" };
export const SORT_STORAGE_KEY = "opendeck.library.sort.v1";
const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

export function parseLibrarySort(value: string | null): LibrarySort {
  try {
    const parsed = JSON.parse(value ?? "null");
    if (parsed && SORT_COLUMNS.includes(parsed.column) && ["asc", "desc"].includes(parsed.direction)) {
      return { column: parsed.column, direction: parsed.direction };
    }
  } catch { /* Old or damaged preferences fall back to source order. */ }
  return { ...DEFAULT_SORT };
}

function text(value: string | null | undefined): string | null {
  const clean = value?.trim();
  return clean && clean.toLowerCase() !== "unknown" ? clean : null;
}
function value(track: DbTrack, column: Exclude<SortColumn, "order">): string | number | null {
  switch (column) {
    case "title": return text(track.title) ?? track.path.split(/[\\/]/).pop() ?? "";
    case "artist": return text(track.artist);
    case "bpm": return track.bpm != null && Number.isFinite(track.bpm) && track.bpm > 0 ? track.bpm : null;
    case "key": return text(track.key_camelot);
    case "duration": return Number.isFinite(track.duration_ms) && track.duration_ms > 0 ? track.duration_ms : null;
  }
}

/** Never mutate source/playlist order; ties retain that order in either direction. */
export function sortTracks(tracks: DbTrack[], sort: LibrarySort): DbTrack[] {
  if (sort.column === "order") return sort.direction === "asc" ? tracks : [...tracks].reverse();
  const column = sort.column;
  const direction = sort.direction === "asc" ? 1 : -1;
  return tracks.map((track, index) => ({ track, index, value: value(track, column) }))
    .sort((a, b) => {
      if (a.value == null) return b.value == null ? a.index - b.index : 1;
      if (b.value == null) return -1;
      const compared = typeof a.value === "number" && typeof b.value === "number"
        ? a.value - b.value : collator.compare(String(a.value), String(b.value));
      return compared * direction || a.index - b.index;
    }).map(({ track }) => track);
}
