const directorySeparator = (path: string) =>
  path.includes('\\') && !path.includes('/') ? '\\' : '/';

const trimTrailingSeparator = (path: string) => {
  const trimmed = path.replace(/[\\/]+$/, '');
  return trimmed.length > 0 ? trimmed : directorySeparator(path);
};

export const selectedWatchFolder = (
  openPath: string | null,
  watchedFolders: readonly string[],
): string | null => {
  if (openPath == null) return null;
  const target = trimTrailingSeparator(openPath);
  const matches = watchedFolders.filter((folder) => {
    const root = trimTrailingSeparator(folder);
    const separator = directorySeparator(root);
    return target === root || target.startsWith(`${root}${separator}`);
  });
  return matches.length === 1 ? matches[0] : null;
};

export const mirrorDirectory = (openPath: string, watchRoot: string): string => {
  const root = trimTrailingSeparator(watchRoot);
  const open = trimTrailingSeparator(openPath);
  const separator = directorySeparator(root);
  const mirrorRoot = `${root}${separator}mp3`;
  if (open === mirrorRoot || open.startsWith(`${mirrorRoot}${separator}`)) return open;
  if (open === root) return mirrorRoot;
  const prefix = `${root}${separator}`;
  if (!open.startsWith(prefix)) return mirrorRoot;
  return `${mirrorRoot}${separator}${open.slice(prefix.length)}`;
};

export const isInsideMirror = (
  openPath: string | null,
  watchedFolders: readonly string[],
): boolean => {
  const root = selectedWatchFolder(openPath, watchedFolders);
  if (root == null || openPath == null) return false;
  const separator = directorySeparator(trimTrailingSeparator(root));
  const mirrorRoot = `${trimTrailingSeparator(root)}${separator}mp3`;
  const open = trimTrailingSeparator(openPath);
  return open === mirrorRoot || open.startsWith(`${mirrorRoot}${separator}`);
};

export const mp3CopyProgressLabel = (
  done: number,
  total: number,
  active: boolean,
): string => {
  if (!active || !Number.isFinite(done) || !Number.isFinite(total) || total <= 0) {
    return '';
  }
  const shownTotal = Math.floor(total);
  const shownDone = Math.min(Math.max(Math.floor(done), 0), shownTotal);
  return `${shownDone}/${shownTotal}`;
};
