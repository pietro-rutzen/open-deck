// Source seconds already account for play speed and output latency.
export function phaseInBar(sourceSeconds: number, firstBeat: number, interval: number): number | null {
  if (!Number.isFinite(sourceSeconds) || !Number.isFinite(firstBeat) ||
      !Number.isFinite(interval) || interval <= 0.05) return null;
  const beats = (sourceSeconds - firstBeat) / interval;
  return ((beats % 4) + 4) % 4;
}
