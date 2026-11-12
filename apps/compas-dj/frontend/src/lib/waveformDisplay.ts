// Display-only. Height encodes amplitude and hue encodes the measured spectral mix.
type Bands = readonly [number, number, number];
const finite = (value: number) => Number.isFinite(value) ? Math.max(0, value) : 0;
export const RGB_WAVE_KEY = 'Band thickness shows bass (blue), mids (amber) and treble (ivory). The outer rim blends bass red, mid green and treble sky blue.';
const NEUTRAL = '#8996a4';
const LOUDNESS_REFERENCE = 1;
const SMOOTH_RADIUS = 2;

type Color = readonly [number, number, number];
const BASS: Color = [255, 16, 33], MID: Color = [66, 255, 10], HIGH: Color = [8, 151, 255];
const LAVENDER: Color = [219, 157, 255], GOLD: Color = [250, 189, 32], MINT: Color = [52, 239, 209];
const BROADBAND: Color = [245, 237, 243];

// Interpolate in a triangle of band strengths. Each edge has its own mixed hue.
// The bass/treble edge passes through deep violet before the balanced lavender.
export function frequencyColor(low: number, mid: number, high: number) {
  const l = finite(low), m = finite(mid) * 0.9, h = finite(high) * 1.85;
  const strongest = Math.max(l, m, h);
  if (strongest === 0) return NEUTRAL;
  const weakest = Math.min(l, m, h);
  const middle = Math.max(Math.min(l, m), Math.min(Math.max(l, m), h));
  const primary = strongest === l ? BASS : strongest === m ? MID : HIGH;
  const pair = weakest === m ? LAVENDER : weakest === l ? MINT : GOLD;
  const span = strongest - weakest;
  // Quantize blend coordinates to cap the number of SVG color groups.
  const step = (value: number, levels: number) => Math.round(value * levels) / levels;
  let t = span > 0 ? step((middle - weakest) / span, 12) : 0;
  const white = step((weakest / strongest) ** 1.6, 4);
  let from = primary, to = pair;
  if (pair === LAVENDER && primary === BASS) {
    const violet: Color = [147, 35, 176];
    if (t < 0.375) { to = violet; t /= 0.375; }
    else { from = violet; t = (t - 0.375) / 0.625; }
  }
  const channel = (i: number) => Math.round((from[i] + (to[i] - from[i]) * t) * (1 - white) + BROADBAND[i] * white);
  return `rgb(${channel(0)},${channel(1)},${channel(2)})`;
}

export interface WavePoint { x: number; amp: number; color: string; rms?: Bands }

// One scale for the whole track keeps a breakdown quiet when the scrolling view zooms in.
export function structureGain(bands: Bands[]) {
  let peak = 0;
  for (const [l, m, h] of bands) peak = Math.max(peak, finite(l) + finite(m) + finite(h));
  return peak > 1e-5 ? 1 / peak : 0;
}

// Pool power across the whole output column, so a kick cannot erase a quieter hi-hat
// from its color. Height is that column's loudness against full scale, the same ruler for every track.
export function waveColumn(peaks: number[], bands: Bands[], start: number, end: number, radius: number) {
  let loudest = start, peak = 0, peakSum = 0, low = 0, mid = 0, high = 0;
  const measured = bands.length === peaks.length;
  for (let i = start; i < end; i++) {
    const p = finite(peaks[i]);
    peakSum += p;
    if (p > peak) { peak = p; loudest = i; }
    if (measured) {
      low += finite(bands[i][0]) ** 2;
      mid += finite(bands[i][1]) ** 2;
      high += finite(bands[i][2]) ** 2;
    }
  }
  const count = Math.max(1, end - start);
  const rms: Bands | undefined = measured
    ? [Math.sqrt(low / count), Math.sqrt(mid / count), Math.sqrt(high / count)] : undefined;
  const bandLevel = rms ? rms[0] + rms[1] + rms[2] : 0;
  const level = bandLevel > 0 ? bandLevel : peakSum / count;
  return {
    loudest,
    rms,
    amp: Math.min(1, level / LOUDNESS_REFERENCE) * radius,
    color: rms ? frequencyColor(...rms) : NEUTRAL,
  };
}

function smoothSeries(values: readonly number[], radius: number) {
  if (radius <= 0 || values.length < 3) return values.slice();
  const smoothed = new Array<number>(values.length);
  for (let index = 0; index < values.length; index += 1) {
    const start = Math.max(0, index - radius);
    const end = Math.min(values.length - 1, index + radius);
    let weighted = 0;
    let weight = 0;
    for (let sample = start; sample <= end; sample += 1) {
      const sampleWeight = radius + 1 - Math.abs(sample - index);
      weighted += values[sample] * sampleWeight;
      weight += sampleWeight;
    }
    smoothed[index] = weight > 0 ? weighted / weight : 0;
  }
  return smoothed;
}

export function smoothAmplitudes<T extends { amp: number }>(points: readonly T[], radius = SMOOTH_RADIUS): T[] {
  const smoothed = smoothSeries(points.map(point => point.amp), radius);
  return points.map((point, index) => ({ ...point, amp: smoothed[index] }));
}

export type WaveAnchor = 'center' | 'baseline';

// Adjacent columns share their interpolated edge. Batch all polygons of one hue
// into one SVG path to avoid creating an element per audio bin.
// A baseline anchor keeps the upper half and stretches it to the full height.
export function rgbPaths(points: WavePoint[], center: number, anchor: WaveAnchor = 'center') {
  const paths = new Map<string, string[]>();
  const coord = (v: number) => v.toFixed(1);
  const floor = anchor === 'baseline' ? coord(center * 2) : '';
  for (let i = 0; i < points.length; i++) {
    const p = points[i], before = points[i - 1] ?? p, after = points[i + 1] ?? p;
    if (p.amp + before.amp + after.amp === 0) continue;
    const l = coord((p.x + before.x) / 2), r = coord((p.x + after.x) / 2), x = coord(p.x);
    const a = (p.amp + before.amp) / 2, b = (p.amp + after.amp) / 2;
    const path = anchor === 'baseline'
      ? `M${l} ${coord((center - a) * 2)}L${x} ${coord((center - p.amp) * 2)} ${r} ${coord((center - b) * 2)} ${r} ${floor} ${l} ${floor}Z`
      : `M${l} ${coord(center-a)}L${x} ${coord(center-p.amp)} ${r} ${coord(center-b)} ${r} ${coord(center+b)} ${x} ${coord(center+p.amp)} ${l} ${coord(center+a)}Z`;
    const group = paths.get(p.color);
    if (group) group.push(path); else paths.set(p.color, [path]);
  }
  return Array.from(paths, ([fill, segments]) => ({ fill, d: segments.join('') }));
}

export function wavePaths(points: WavePoint[], center: number, gain: number, bandScale = 1, anchor: WaveAnchor = 'center') {
  const paths = rgbPaths(points, center, anchor);
  if (!gain || !paths.length || points.length < 2) return paths;
  if (anchor === 'baseline') return baselineBands(paths, points, center, gain, bandScale);
  const layers = Array.from({ length: 3 }, () => ({ upper: [] as string[], lower: [] as string[] }));
  const visible = [false, false, false];
  for (const point of points) {
    const [low, mid, high] = point.rms ?? [0, 0, 0];
    const sum = low + mid + high;
    // Keep the spectral rim visible at thumbnail size. Sustained energy fills
    // slightly more of the peak envelope than short transients.
    const relativeEnergy = Math.min(1, sum * gain);
    const bodyRadius = point.amp * (0.68 + 0.08 * relativeEnergy) * bandScale;
    const scale = sum > 0 ? bodyRadius / sum : 0;
    const depths = [sum * scale, (mid + high) * scale, high * scale];
    for (let i = 0; i < 3; i++) {
      const x = point.x.toFixed(1);
      layers[i].upper.push(x + ' ' + (center - depths[i]).toFixed(1));
      layers[i].lower.push(x + ' ' + (center + depths[i]).toFixed(1));
      visible[i] ||= depths[i] > 0;
    }
  }
  const fills = ['#137fff', '#ffa916', '#fff4d9'];
  for (let i = 0; i < layers.length; i++) {
    if (visible[i]) paths.push({ fill: fills[i],
      d: `M${layers[i].upper.join(' ')}L${layers[i].lower.reverse().join(' ')}Z` });
  }
  return paths;
}

function baselineBands(
  paths: { fill: string; d: string }[],
  points: WavePoint[],
  center: number,
  gain: number,
  bandScale: number,
) {
  const floor = (center * 2).toFixed(1);
  const crests = Array.from({ length: 3 }, () => [] as string[]);
  const visible = [false, false, false];
  let firstX = '';
  let lastX = '';
  for (const point of points) {
    const [low, mid, high] = point.rms ?? [0, 0, 0];
    const sum = low + mid + high;
    const relativeEnergy = Math.min(1, sum * gain);
    const bodyRadius = point.amp * (0.68 + 0.08 * relativeEnergy) * bandScale;
    const scale = sum > 0 ? bodyRadius / sum : 0;
    const depths = [sum * scale, (mid + high) * scale, high * scale];
    const x = point.x.toFixed(1);
    if (firstX === '') firstX = x;
    lastX = x;
    for (let i = 0; i < 3; i++) {
      crests[i].push(x + ' ' + ((center - depths[i]) * 2).toFixed(1));
      visible[i] ||= depths[i] > 0;
    }
  }
  const fills = ['#137fff', '#ffa916', '#fff4d9'];
  for (let i = 0; i < crests.length; i++) {
    if (visible[i]) paths.push({
      fill: fills[i],
      d: `M${crests[i].join(' ')}L${lastX} ${floor} ${firstX} ${floor}Z`,
    });
  }
  return paths;
}
