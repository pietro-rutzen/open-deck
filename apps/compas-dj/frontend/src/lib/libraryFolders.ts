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
  key_camelot?: string | null;
}

const FOLDER_ORDER_KEY = "opendeck.library.folderOrder.v1";
const folderOrderListeners = new Set<() => void>();

export const readFolderOrders = (): Record<string, string[]> => {
  try {
    const parsed = JSON.parse(localStorage.getItem(FOLDER_ORDER_KEY) ?? "null") as unknown;
    if (!parsed || typeof parsed !== "object") return {};
    return Object.fromEntries(
      Object.entries(parsed).flatMap(([folder, paths]) =>
        Array.isArray(paths) && paths.every((path) => typeof path === "string") ? [[folder, paths]] : [],
      ),
    );
  } catch {
    return {};
  }
};

export const writeFolderOrders = (orders: Record<string, string[]>) => {
  try {
    localStorage.setItem(FOLDER_ORDER_KEY, JSON.stringify(orders));
  } catch {
    return;
  }
  folderOrderListeners.forEach((listener) => listener());
};

export const subscribeFolderOrders = (listener: () => void) => {
  folderOrderListeners.add(listener);
  return () => {
    folderOrderListeners.delete(listener);
  };
};

/** Saved order for `folder`, including a key that only differs by a trailing slash. */
export const savedFolderOrder = (orders: Readonly<Record<string, readonly string[]>>, folder: string) => {
  const normalized = normalizeDir(folder);
  const direct = orders[folder] ?? orders[normalized];
  if (direct) return direct;
  const key = Object.keys(orders).find((candidate) => normalizeDir(candidate) === normalized);
  return key ? orders[key] : undefined;
};

const fileOrder = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

/** Saved folder order first, then numeric file name. Tracks listed in the saved order stay ahead of any new files. */
export const folderOrderComparator = (folderOrder: readonly string[] | undefined) => {
  const rank = new Map((folderOrder ?? []).map((path, index) => [path, index]));
  return (leftPath: string, rightPath: string) => {
    const leftRank = rank.get(leftPath);
    const rightRank = rank.get(rightPath);
    if (leftRank != null && rightRank != null && leftRank !== rightRank) return leftRank - rightRank;
    if (leftRank != null) return -1;
    if (rightRank != null) return 1;
    return fileOrder.compare(baseName(leftPath), baseName(rightPath)) || leftPath.localeCompare(rightPath);
  };
};

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

/** Queued tracks play before ranked suggestions. A drag order, when present, wins. */
export function combineNextTracks<Track extends { path: string }>(
  suggestions: readonly Track[],
  queued: readonly Track[],
  preferred: readonly string[] | undefined,
): Track[] {
  const queuedPaths = new Set(queued.map((track) => track.path));
  const pool = [...queued, ...suggestions.filter((track) => !queuedPaths.has(track.path))];
  return orderSuggestions(pool, preferred);
}

const knownTempo = (bpm: number | null | undefined) =>
  bpm != null && Number.isFinite(bpm) && bpm > 0 ? bpm : null;

const stepsAhead = (index: number, here: number, count: number) => {
  if (here < 0 || count <= 0) return index;
  return index > here ? index - here : count - here + index;
};

const tempoFraction = (bpm: number | null, reference: number | null) => {
  if (reference == null || bpm == null) return 0.5;
  return Math.min(1, Math.abs(bpm - reference) / 32);
};

const bpmDistance = (bpm: number | null, reference: number | null) => {
  const tempo = knownTempo(bpm);
  if (reference == null || tempo == null) return Number.POSITIVE_INFINITY;
  return Math.abs(tempo - reference);
};

const parseCamelot = (value: string | null | undefined) => {
  if (!value) return null;
  const match = /^(\d{1,2})([AB])$/.exec(value.trim().toUpperCase());
  if (!match) return null;
  const number = Number(match[1]);
  const letter = match[2];
  if (number < 1 || number > 12 || (letter !== "A" && letter !== "B")) return null;
  return { number, letter };
};

const wheelDistance = (left: number, right: number) => {
  const delta = Math.abs(left - right) % 12;
  return Math.min(delta, 12 - delta);
};

const UNKNOWN_KEY_DISTANCE = 100;

/** Lower is closer on the Camelot wheel. Same key, then relative major/minor, then fewer steps. Unknown keys sort last. */
const camelotDistance = (fromKey: string | null | undefined, toKey: string | null | undefined) => {
  const from = parseCamelot(fromKey);
  const to = parseCamelot(toKey);
  if (!from || !to) return UNKNOWN_KEY_DISTANCE;
  const steps = wheelDistance(from.number, to.number);
  const modeChange = from.letter === to.letter ? 0 : 1;
  if (steps === 0) return modeChange;
  return steps * 2 + modeChange;
};

/** Performance order (`tempoWeight` >= 1): closest BPM, then Camelot proximity, then the next track in folder order.
 *  Lower weights follow folder order and only nudge toward a closer tempo. */
export function suggestNextTracks<Track extends NextTrackCandidate>(
  tracks: readonly Track[],
  currentPath: string,
  currentBpm: number | null,
  busyPaths: readonly string[],
  limit: number,
  tempoWeight = 1,
  currentKey: string | null = null,
  folderOrder?: readonly string[],
): Track[] {
  if (limit <= 0) return [];
  const folder = directoryOf(currentPath);
  const compareOrder = folderOrderComparator(folderOrder);
  const ordered = tracks
    .filter((track) => directoryOf(track.path) === folder)
    .sort((left, right) => compareOrder(left.path, right.path));
  const here = ordered.findIndex((track) => track.path === currentPath);
  const reference = knownTempo(currentBpm) ?? knownTempo(here >= 0 ? ordered[here]?.bpm : null);
  const busy = new Set([...busyPaths, currentPath]);
  const weight = Math.min(1, Math.max(0, tempoWeight));
  return ordered
    .map((track, index) => ({ track, index }))
    .filter(({ track }) => !busy.has(track.path))
    .sort((left, right) => {
      const leftSteps = stepsAhead(left.index, here, ordered.length);
      const rightSteps = stepsAhead(right.index, here, ordered.length);
      if (weight >= 1) {
        const leftDistance = bpmDistance(left.track.bpm, reference);
        const rightDistance = bpmDistance(right.track.bpm, reference);
        const leftShown = Number.isFinite(leftDistance) ? Math.round(leftDistance * 100) : leftDistance;
        const rightShown = Number.isFinite(rightDistance) ? Math.round(rightDistance * 100) : rightDistance;
        if (leftShown < rightShown) return -1;
        if (leftShown > rightShown) return 1;
        const leftKey = camelotDistance(currentKey, left.track.key_camelot);
        const rightKey = camelotDistance(currentKey, right.track.key_camelot);
        if (leftKey < rightKey) return -1;
        if (leftKey > rightKey) return 1;
        return leftSteps - rightSteps;
      }
      const span = Math.max(1, ordered.length);
      const leftScore = (1 - weight) * (leftSteps / span) + weight * tempoFraction(knownTempo(left.track.bpm), reference);
      const rightScore = (1 - weight) * (rightSteps / span) + weight * tempoFraction(knownTempo(right.track.bpm), reference);
      if (leftScore < rightScore) return -1;
      if (leftScore > rightScore) return 1;
      return leftSteps - rightSteps;
    })
    .slice(0, limit)
    .map(({ track }) => track);
}
