export interface FolderEntry {
  path: string;
  name: string;
  trackCount: number;
  playedCount: number;
}

export interface PlayCounts {
  played: number;
  total: number;
}

/** White below 1/4 played, yellow through 1/2, orange through 3/4, red after that. */
export const playProgressTone = (played: number, total: number) => {
  if (total <= 0) return "quiet" as const;
  const ratio = played / total;
  if (ratio < 0.25) return "quiet" as const;
  if (ratio <= 0.5) return "yellow" as const;
  if (ratio <= 0.75) return "orange" as const;
  return "red" as const;
};

export const folderPlayCounts = <Track extends { path: string; play_count?: number }>(
  tracks: readonly Track[],
  folder: string | null,
): PlayCounts => {
  const inside = folder == null ? tracks : tracks.filter((track) => isInside(folder, track.path));
  return {
    total: inside.length,
    played: inside.filter((track) => (track.play_count ?? 0) > 0).length,
  };
};

const usesBackslash = (path: string) => path.includes("\\") && !path.includes("/");

const separatorOf = (path: string) => (usesBackslash(path) ? "\\" : "/");

export const normalizeDir = (path: string) => {
  const slash = separatorOf(path);
  const trimmed = path.replace(/[\\/]+$/, "");
  if (trimmed.length > 0) return trimmed;
  return slash;
};

export const baseName = (path: string) => {
  const slash = separatorOf(path);
  const trimmed = path.replace(/[\\/]+$/, "");
  const index = trimmed.lastIndexOf(slash);
  return index >= 0 ? trimmed.slice(index + 1) : trimmed;
};

export const directoryOf = (filePath: string) => {
  const slash = separatorOf(filePath);
  const trimmed = filePath.replace(/[\\/]+$/, "");
  const index = trimmed.lastIndexOf(slash);
  if (index < 0) return trimmed;
  if (index === 0) return slash;
  return trimmed.slice(0, index);
};

export const isInside = (dir: string, fileOrDir: string) => {
  const root = normalizeDir(dir);
  const target = normalizeDir(fileOrDir);
  return target === root || target.startsWith(`${root}${separatorOf(root)}`);
};

const parentDirectory = (folder: string) => {
  const normalized = normalizeDir(folder);
  const slash = separatorOf(normalized);
  const index = normalized.lastIndexOf(slash);
  if (index <= 0) return null;
  return normalized.slice(0, index);
};

export const containingRoot = (folder: string, roots: readonly string[]) =>
  roots.find((root) => isInside(root, folder)) ?? null;

/** Parent folder within an imported root. The imported root itself steps back out to the whole library. */
export const ascendFolder = (folder: string, roots: readonly string[]) => {
  const root = containingRoot(folder, roots);
  if (!root || normalizeDir(folder) === normalizeDir(root)) return null;
  return parentDirectory(folder);
};

export const folderCrumbs = (folder: string, roots: readonly string[]) => {
  const root = containingRoot(folder, roots) ?? folder;
  const slash = separatorOf(root);
  const rest = normalizeDir(folder).slice(normalizeDir(root).length);
  const parts = rest.split(slash).filter((part) => part.length > 0);
  const crumbs = [{ path: normalizeDir(root), name: baseName(root) }];
  return parts.reduce((pathCrumbs, part) => {
    const parent = pathCrumbs[pathCrumbs.length - 1]?.path ?? normalizeDir(root);
    return [...pathCrumbs, { path: `${parent}${slash}${part}`, name: part }];
  }, crumbs);
};

export const directTracks = <Track extends { path: string }>(tracks: readonly Track[], folder: string) => {
  const root = normalizeDir(folder);
  return tracks.filter((track) => directoryOf(track.path) === root);
};

export const childFolders = <Track extends { path: string; play_count?: number }>(
  tracks: readonly Track[],
  folder: string,
): FolderEntry[] => {
  const root = normalizeDir(folder);
  const slash = separatorOf(root);
  const counts = new Map<string, PlayCounts>();
  tracks.forEach((track) => {
    const file = normalizeDir(track.path);
    if (!isInside(root, file) || directoryOf(file) === root) return;
    const name = file.slice(root.length + 1).split(slash)[0];
    if (!name) return;
    const path = `${root}${slash}${name}`;
    const current = counts.get(path) ?? { played: 0, total: 0 };
    counts.set(path, {
      total: current.total + 1,
      played: current.played + ((track.play_count ?? 0) > 0 ? 1 : 0),
    });
  });
  return [...counts.entries()]
    .map(([path, countsForFolder]) => ({
      path,
      name: baseName(path),
      trackCount: countsForFolder.total,
      playedCount: countsForFolder.played,
    }))
    .sort((left, right) => left.name.localeCompare(right.name, undefined, { sensitivity: "base" }));
};

export interface NextTrackCandidate {
  path: string;
  bpm: number | null;
}

const fileOrder = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

export const compareFileNames = (left: string, right: string) => fileOrder.compare(left, right);

/** Move `from` into the place occupied by `to`. Returns null when either path is missing or they match. */
export const reorderPaths = (paths: readonly string[], from: string, to: string): string[] | null => {
  const fromIndex = paths.indexOf(from);
  if (fromIndex < 0 || !paths.includes(to) || from === to) return null;
  const next = [...paths];
  const [moved] = next.splice(fromIndex, 1);
  const insertAt = next.indexOf(to);
  if (!moved || insertAt < 0) return null;
  next.splice(insertAt, 0, moved);
  return next;
};

/** Keep a manual suggestion order, then append any newly suggested tracks. */
export function orderSuggestions<Track extends { path: string }>(suggestions: readonly Track[], preferred: readonly string[] | undefined): Track[] {
  if (!preferred?.length) return [...suggestions];
  const byPath = new Map(suggestions.map((track) => [track.path, track]));
  const kept = preferred.flatMap((path) => {
    const track = byPath.get(path);
    return track ? [track] : [];
  });
  const seen = new Set(kept.map((track) => track.path));
  return [...kept, ...suggestions.filter((track) => !seen.has(track.path))];
}

const knownTempo = (bpm: number | null | undefined) =>
  bpm != null && Number.isFinite(bpm) && bpm > 0 ? bpm : null;

const stepsAhead = (index: number, here: number, count: number) => {
  if (here < 0 || count <= 0) return index;
  return index > here ? index - here : count - here + index;
};

/** Closest analyzed BPM first. Equal distances follow the next filename in that folder. */
export function suggestNextTracks<Track extends NextTrackCandidate>(
  tracks: readonly Track[],
  currentPath: string,
  currentBpm: number | null,
  busyPaths: readonly string[],
  limit: number,
): Track[] {
  if (limit <= 0) return [];
  const folder = directoryOf(currentPath);
  const ordered = tracks
    .filter((track) => directoryOf(track.path) === folder)
    .sort((left, right) => fileOrder.compare(baseName(left.path), baseName(right.path)) || left.path.localeCompare(right.path));
  const here = ordered.findIndex((track) => track.path === currentPath);
  const reference = knownTempo(currentBpm) ?? knownTempo(here >= 0 ? ordered[here]?.bpm : null);
  const busy = new Set([...busyPaths, currentPath]);
  return ordered
    .map((track, index) => ({ track, index }))
    .filter(({ track }) => !busy.has(track.path))
    .sort((left, right) => {
      const leftTempo = knownTempo(left.track.bpm);
      const rightTempo = knownTempo(right.track.bpm);
      const leftDistance = reference == null || leftTempo == null ? Number.POSITIVE_INFINITY : Math.abs(leftTempo - reference);
      const rightDistance = reference == null || rightTempo == null ? Number.POSITIVE_INFINITY : Math.abs(rightTempo - reference);
      if (leftDistance < rightDistance) return -1;
      if (leftDistance > rightDistance) return 1;
      return stepsAhead(left.index, here, ordered.length) - stepsAhead(right.index, here, ordered.length);
    })
    .slice(0, limit)
    .map(({ track }) => track);
}
