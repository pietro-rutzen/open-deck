const listeners = new Set<() => void>();

/** Paths the user added to a deck's Next list, in the order they should play. */
let queuedByDeck: Record<number, string[]> = {};

/** Drag order of one deck's Next list while a particular track is loaded. */
let manualOrder: Record<string, string[]> = {};

const publish = () => {
  listeners.forEach((listener) => listener());
};

export const nextOrderKey = (deck: number, loadedPath: string) => `${deck}:${loadedPath}`;

export const readQueuedPaths = (deck: number): readonly string[] => queuedByDeck[deck] ?? [];

export const readNextQueueSnapshot = () => ({ queuedByDeck, manualOrder });

export const subscribeNextQueue = (listener: () => void) => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};

export const enqueueOnDeck = (deck: number, path: string) => {
  const current = queuedByDeck[deck] ?? [];
  if (!path || current.includes(path)) return;
  const prefix = `${deck}:`;
  const nextQueued = [...current, path];
  const queuedSet = new Set(nextQueued);
  queuedByDeck = { ...queuedByDeck, [deck]: nextQueued };
  manualOrder = Object.fromEntries(
    Object.entries(manualOrder).map(([key, paths]) => {
      if (!key.startsWith(prefix) || paths.includes(path)) return [key, paths];
      const lastQueuedIndex = paths.reduce(
        (found, item, index) => (queuedSet.has(item) ? index : found),
        -1,
      );
      const next = [...paths];
      next.splice(lastQueuedIndex + 1, 0, path);
      return [key, next];
    }),
  );
  publish();
};

export const forgetQueuedPath = (deck: number, path: string) => {
  const current = queuedByDeck[deck] ?? [];
  const prefix = `${deck}:`;
  const queuedChanged = current.includes(path);
  const manualChanged = Object.entries(manualOrder).some(
    ([key, paths]) => key.startsWith(prefix) && paths.includes(path),
  );
  if (!queuedChanged && !manualChanged) return;
  if (queuedChanged) {
    queuedByDeck = { ...queuedByDeck, [deck]: current.filter((item) => item !== path) };
  }
  if (manualChanged) {
    manualOrder = Object.fromEntries(
      Object.entries(manualOrder).map(([key, paths]) => [
        key,
        key.startsWith(prefix) ? paths.filter((item) => item !== path) : paths,
      ]),
    );
  }
  publish();
};

export const replaceQueuedPaths = (deck: number, paths: readonly string[]) => {
  const prefix = `${deck}:`;
  const nextQueued = [...paths];
  const queuedSet = new Set(nextQueued);
  queuedByDeck = { ...queuedByDeck, [deck]: nextQueued };
  manualOrder = Object.fromEntries(
    Object.entries(manualOrder).map(([key, current]) => {
      if (!key.startsWith(prefix)) return [key, current];
      const suggestions = current.filter((path) => !queuedSet.has(path));
      return [key, [...nextQueued, ...suggestions]];
    }),
  );
  publish();
};

/** Remember a drag order, and keep queued tracks in that same relative order. */
export const setManualNextOrder = (deck: number, loadedPath: string, paths: readonly string[]) => {
  const queued = queuedByDeck[deck] ?? [];
  const queuedSet = new Set(queued);
  const reordered = paths.filter((path) => queuedSet.has(path));
  const rest = queued.filter((path) => !reordered.includes(path));
  queuedByDeck = { ...queuedByDeck, [deck]: [...reordered, ...rest] };
  manualOrder = { ...manualOrder, [nextOrderKey(deck, loadedPath)]: [...paths] };
  publish();
};

export const forgetQueuedPaths = (paths: readonly string[]) => {
  if (paths.length === 0) return;
  const dropped = new Set(paths);
  const queueChanged = Object.values(queuedByDeck).some((queued) =>
    queued.some((path) => dropped.has(path)),
  );
  const manualChanged = Object.entries(manualOrder).some(([key, queued]) => {
    const loaded = key.slice(key.indexOf(":") + 1);
    return dropped.has(loaded) || queued.some((path) => dropped.has(path));
  });
  if (!queueChanged && !manualChanged) return;
  queuedByDeck = Object.fromEntries(
    Object.entries(queuedByDeck).map(([deck, queued]) => [
      Number(deck),
      queued.filter((path) => !dropped.has(path)),
    ]),
  );
  manualOrder = Object.fromEntries(
    Object.entries(manualOrder).flatMap(([key, queued]) => {
      const loaded = key.slice(key.indexOf(":") + 1);
      if (dropped.has(loaded)) return [];
      return [[key, queued.filter((path) => !dropped.has(path))]];
    }),
  );
  publish();
};

export const retargetNextQueuePaths = (moved: ReadonlyMap<string, string>) => {
  if (moved.size === 0) return;
  const retarget = (path: string) => moved.get(path) ?? path;
  let changed = false;
  const nextQueued: Record<number, string[]> = {};
  for (const [deck, paths] of Object.entries(queuedByDeck)) {
    const updated = paths.map(retarget);
    if (updated.some((path, index) => path !== paths[index])) changed = true;
    nextQueued[Number(deck)] = updated;
  }
  const nextManual: Record<string, string[]> = {};
  for (const [key, paths] of Object.entries(manualOrder)) {
    const splitAt = key.indexOf(":");
    const deck = key.slice(0, splitAt);
    const loaded = key.slice(splitAt + 1);
    const updatedLoaded = retarget(loaded);
    const updated = paths.map(retarget);
    if (updatedLoaded !== loaded || updated.some((path, index) => path !== paths[index])) changed = true;
    nextManual[`${deck}:${updatedLoaded}`] = updated;
  }
  if (!changed) return;
  queuedByDeck = nextQueued;
  manualOrder = nextManual;
  publish();
};
