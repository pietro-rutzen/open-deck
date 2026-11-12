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

export interface Mp3ConversionState {
  converted: number;
  convertible: number;
  alreadyMp3: number;
  failures: readonly string[];
  active: boolean;
}

const wholeCount = (value: number) =>
  Number.isFinite(value) ? Math.max(0, Math.floor(value)) : 0;

/// Painted fraction is mirrored files over every audio source in the open folder.
/// A finished folder hides it. Failures stay on the hover title.
export const mp3ConversionPresentation = (
  state: Mp3ConversionState,
): { fraction: string; activity: string; title: string; failed: boolean } => {
  const total = wholeCount(state.convertible);
  const done = Math.min(wholeCount(state.converted), total);
  const failures = state.failures.filter((failure) => failure.trim().length > 0);
  const complete = total > 0 && done >= total && failures.length === 0 && !state.active;
  const fraction = total > 0 && !complete ? `${done}/${total}` : '';
  const activity = fraction.length > 0 ? `converting ${fraction}` : '';
  const details = [
    fraction.length > 0 ? `${done} of ${total} in the mirror` : null,
    failures.length > 0 ? `Failed: ${failures.join('; ')}` : null,
  ].filter((line): line is string => line != null);
  const title = details.length > 0
    ? `Keep MP3 copies. ${details.join('. ')}.`
    : 'Keep MP3 copies';
  return { fraction, activity, title, failed: failures.length > 0 };
};
