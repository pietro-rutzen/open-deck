// Display-only envelopes: preserve signal timing and silence; never alter playback audio.
export const WAVE_COLORS = ["#087fff", "#ffad00", "#ffffff"] as const;
const finite = (v: number) => Number.isFinite(v) ? Math.max(0, v) : 0;
export function waveformScale(peaks: number[], bands: [number, number, number][] = []) {
  let peak = 0, band = 0;
  for (const p of peaks) peak = Math.max(peak, finite(p));
  for (const b of bands) for (const v of b) band = Math.max(band, finite(v));
  return { peakGain: peak > .00001 ? Math.min(6, 1 / peak) : 1, bandGain: band > .00001 ? Math.min(12, 1 / band) : 1 };
}
export function waveformHeight(value: number, gain: number) {
  return Math.pow(Math.min(1, finite(value) * gain), .72);
}
export function frequencyLayers(peak: number, bands: [number, number, number], scale: ReturnType<typeof waveformScale>) {
  const envelope = waveformHeight(peak, scale.peakGain);
  return bands.map((energy, index) => ({ color: WAVE_COLORS[index], height: Math.min(envelope, waveformHeight(energy, scale.bandGain)) }))
    .sort((a, b) => b.height - a.height);
}

export function bandAmplitudes(peak: number, bands: readonly [number, number, number] | undefined, scale: ReturnType<typeof waveformScale>): number[] {
  const envelope = waveformHeight(peak, scale.peakGain);
  if (!bands) return [envelope];
  return [0, 1, 2].map((index) => Math.min(envelope, waveformHeight(bands[index], scale.bandGain)));
}

const roundWave = (value: number) => (Math.round(value * 10) / 10).toFixed(1);

const clampCurve = (value: number, first: number, second: number, center: number, upper: boolean) => {
  const towardPeak = upper ? Math.min(first, second) : Math.max(first, second);
  const room = Math.abs(first - second) * 0.2;
  const bound = upper ? towardPeak - room : towardPeak + room;
  return upper ? Math.max(bound, Math.min(center, value)) : Math.min(bound, Math.max(center, value));
};

const curveThrough = (points: readonly { x: number; y: number }[], center: number, upper: boolean) => {
  const commands: string[] = [];
  for (let index = 0; index < points.length - 1; index += 1) {
    const before = points[Math.max(0, index - 1)];
    const from = points[index];
    const to = points[index + 1];
    const after = points[Math.min(points.length - 1, index + 2)];
    const controlFromX = from.x + (to.x - before.x) / 6;
    const controlToX = to.x - (after.x - from.x) / 6;
    const controlFromY = clampCurve(from.y + (to.y - before.y) / 6, from.y, to.y, center, upper);
    const controlToY = clampCurve(to.y - (after.y - from.y) / 6, from.y, to.y, center, upper);
    commands.push(` C ${roundWave(controlFromX)} ${roundWave(controlFromY)} ${roundWave(controlToX)} ${roundWave(controlToY)} ${roundWave(to.x)} ${roundWave(to.y)}`);
  }
  return commands.join("");
};

/** One filled silhouette through the peaks, mirrored around `centerY`. */
export function filledWavePath(points: readonly { x: number; amp: number }[], centerY: number): string {
  const usable = points.filter((point) => Number.isFinite(point.x) && Number.isFinite(point.amp) && point.amp >= 0);
  if (usable.length === 0) return "";
  const first = usable[0];
  if (usable.length === 1) {
    return `M ${roundWave(first.x)} ${roundWave(centerY - first.amp)} L ${roundWave(first.x)} ${roundWave(centerY + first.amp)} Z`;
  }
  const upper = usable.map((point) => ({ x: point.x, y: centerY - point.amp }));
  const lower = [...usable].reverse().map((point) => ({ x: point.x, y: centerY + point.amp }));
  const right = lower[0];
  return `M ${roundWave(upper[0].x)} ${roundWave(upper[0].y)}${curveThrough(upper, centerY, true)} L ${roundWave(right.x)} ${roundWave(right.y)}${curveThrough(lower, centerY, false)} Z`;
}
