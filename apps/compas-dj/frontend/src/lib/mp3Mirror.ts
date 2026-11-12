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
