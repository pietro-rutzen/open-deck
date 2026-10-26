/** Source frames still to play, not counting silent spans the engine will jump. */
export const audibleFramesRemaining = (
  frame: number,
  frames: number,
  silence: readonly (readonly [number, number])[] | undefined,
): number => {
  if (!(frames > 0)) return 0;
  const spans = silence ?? [];
  let cursor = Math.max(0, frame);
  let audible = 0;
  for (const [start, end] of spans) {
    if (!(end > cursor)) continue;
    if (start >= frames) break;
    if (start > cursor) audible += Math.min(start, frames) - cursor;
    cursor = Math.max(cursor, Math.min(end, frames));
  }
  if (cursor < frames) audible += frames - cursor;
  return Math.max(0, audible);
};

/** Where playback starts when the file opens on silence. */
export const firstAudibleFrame = (
  silence: readonly (readonly [number, number])[] | undefined,
  frames: number,
): number => {
  const leading = silence?.[0];
  if (!leading) return 0;
  const [start, end] = leading;
  return start <= 0 && end > 0 && end < frames ? end : 0;
};
