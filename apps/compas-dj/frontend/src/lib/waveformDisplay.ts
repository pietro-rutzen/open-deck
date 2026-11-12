// Display-only. Height encodes amplitude and hue encodes the measured spectral mix.
type Bands = readonly [number, number, number];
const finite = (value: number) => Number.isFinite(value) ? Math.max(0, value) : 0;
export const RGB_WAVE_KEY = 'Band thickness shows bass (blue), mids (amber) and treble (ivory). The outer rim blends bass red, mid green and treble sky blue.';
const NEUTRAL = '#8996a4';
const SMOOTH_RADIUS = 2;
const LEVEL_STEP = 0.4;
const LOUD_SECTION_PERCENTILE = 0.94;
const LOUD_SECTION_FLOOR = 0.1;
const SILENT_LEVEL = 1e-4;

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

function columnMeasure(peaks: number[], bands: Bands[], start: number, end: number) {
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
  return { loudest, rms, level: bandLevel > 0 ? bandLevel : peakSum / count };
}

// Pool power across the whole output column, so a kick cannot erase a quieter hi-hat
// from its color. Amplitude here is still absolute; row height is applied afterwards.
export function waveColumn(peaks: number[], bands: Bands[], start: number, end: number, radius: number) {
  const measured = columnMeasure(peaks, bands, start, end);
  return {
    loudest: measured.loudest,
    rms: measured.rms,
    amp: Math.min(1, measured.level) * radius,
    color: measured.rms ? frequencyColor(...measured.rms) : NEUTRAL,
  };
}

function percentile(values: readonly number[], fraction: number) {
  const ranked = [...values].sort((left, right) => left - right);
  if (ranked.length === 0) return 0;
  const index = Math.min(ranked.length - 1, Math.max(0, Math.ceil(fraction * ranked.length) - 1));
  return ranked[index] ?? 0;
}

// High percentile of the whole-track envelope, so one peak does not set the row height.
export function trackLevelReference(peaks: number[], bands: Bands[]) {
  const count = peaks.length;
  if (count === 0) return 1;
  const columns = Math.min(count, 512);
  const levels = Array.from({ length: columns }, (_, column) => {
    const start = Math.floor(column * count / columns);
    const end = Math.max(start + 1, Math.floor((column + 1) * count / columns));
    return columnMeasure(peaks, bands, start, end).level;
  });
  const smoothed = smoothLevels(levels);
  const reference = percentile(smoothed, LOUD_SECTION_PERCENTILE);
  const loudest = smoothed.reduce((highest, level) => Math.max(highest, level), 0);
  return reference > 1e-5 ? reference : Math.max(loudest, 1e-5);
}

// Silence stays empty. Anything audible keeps a floor, then rises in proportion to the loud section.
export function presentAmplitude(amp: number, reference: number, radius: number) {
  if (!(radius > 0) || !(amp > 0)) return 0;
  const level = amp / radius;
  if (level <= SILENT_LEVEL) return 0;
  const span = reference > 1e-5 ? reference : level;
  const relative = Math.min(1, level / span);
  return (LOUD_SECTION_FLOOR + (1 - LOUD_SECTION_FLOOR) * relative) * radius;
}

function smoothLevels(values: readonly number[], radius = SMOOTH_RADIUS) {
  if (radius <= 0 || values.length < 3) return values.slice();
  return values.map((amp, index) => {
    const start = Math.max(0, index - radius);
    const end = Math.min(values.length - 1, index + radius);
    let weighted = 0;
    let weight = 0;
    for (let sample = start; sample <= end; sample += 1) {
      const louder = Math.max(amp, values[sample]);
      const crossesStep = sample !== index && louder > 1e-8 && Math.abs(amp - values[sample]) / louder > LEVEL_STEP;
      if (crossesStep) continue;
      const sampleWeight = radius + 1 - Math.abs(sample - index);
      weighted += values[sample] * sampleWeight;
      weight += sampleWeight;
    }
    return weight > 0 ? weighted / weight : amp;
  });
}

export function smoothAmplitudes<T extends { amp: number }>(points: readonly T[], radius = SMOOTH_RADIUS): T[] {
  const smoothed = smoothLevels(points.map(point => point.amp), radius);
  return points.map((point, index) => ({ ...point, amp: smoothed[index] }));
}

export type WaveAnchor = 'center' | 'baseline';

function columnEdges(points: readonly { x: number }[]) {
  return points.map((point, index) => {
    const previous = points[index - 1];
    const next = points[index + 1];
    return {
      left: previous ? (previous.x + point.x) / 2 : point.x,
      right: next ? (point.x + next.x) / 2 : point.x,
    };
  });
}

const coord = (value: number) => value.toFixed(1);

function stepBaseline(points: readonly { x: number }[], crest: readonly number[], floor: number) {
  const edges = columnEdges(points);
  let path = `M${coord(edges[0].left)} ${coord(crest[0])}`;
  for (let index = 0; index < points.length; index += 1) {
    path += `L${coord(edges[index].right)} ${coord(crest[index])}`;
    if (index + 1 < points.length) path += `L${coord(edges[index].right)} ${coord(crest[index + 1])}`;
  }
  const end = edges[edges.length - 1].right;
  path += `L${coord(end)} ${coord(floor)}L${coord(edges[0].left)} ${coord(floor)}Z`;
  return path;
}

function stepMirrored(points: readonly { x: number }[], depth: readonly number[], center: number) {
  const edges = columnEdges(points);
  const top = (index: number) => center - depth[index];
  const bottom = (index: number) => center + depth[index];
  let path = `M${coord(edges[0].left)} ${coord(top(0))}`;
  for (let index = 0; index < points.length; index += 1) {
    path += `L${coord(edges[index].right)} ${coord(top(index))}`;
    if (index + 1 < points.length) path += `L${coord(edges[index].right)} ${coord(top(index + 1))}`;
  }
  for (let index = points.length - 1; index >= 0; index -= 1) {
    path += `L${coord(edges[index].right)} ${coord(bottom(index))}`;
    path += `L${coord(edges[index].left)} ${coord(bottom(index))}`;
    if (index > 0) path += `L${coord(edges[index].left)} ${coord(bottom(index - 1))}`;
  }
  return `${path}Z`;
}

function columnDepths(point: WavePoint, gain: number, bandScale: number): [number, number, number] {
  const [low, mid, high] = point.rms ?? [0, 0, 0];
  const sum = low + mid + high;
  const relativeEnergy = Math.min(1, sum * gain);
  const bodyRadius = point.amp * (0.68 + 0.08 * relativeEnergy) * bandScale;
  const scale = sum > 0 ? bodyRadius / sum : 0;
  return [sum * scale, (mid + high) * scale, high * scale];
}

// Each column holds its own height and meets the next on a vertical edge.
// Batch every bar of one hue into one SVG path.
// A baseline anchor keeps the upper half and stretches it to the full height.
export function rgbPaths(points: WavePoint[], center: number, anchor: WaveAnchor = 'center') {
  const paths = new Map<string, string[]>();
  const edges = columnEdges(points);
  const floor = center * 2;
  for (let i = 0; i < points.length; i++) {
    const p = points[i];
    if (p.amp === 0) continue;
    const { left, right } = edges[i];
    const path = anchor === 'baseline'
      ? `M${coord(left)} ${coord((center - p.amp) * 2)}L${coord(right)} ${coord((center - p.amp) * 2)} ${coord(right)} ${coord(floor)} ${coord(left)} ${coord(floor)}Z`
      : `M${coord(left)} ${coord(center - p.amp)}L${coord(right)} ${coord(center - p.amp)} ${coord(right)} ${coord(center + p.amp)} ${coord(left)} ${coord(center + p.amp)}Z`;
    const group = paths.get(p.color);
    if (group) group.push(path); else paths.set(p.color, [path]);
  }
  return Array.from(paths, ([fill, segments]) => ({ fill, d: segments.join('') }));
}

export function wavePaths(points: WavePoint[], center: number, gain: number, bandScale = 1, anchor: WaveAnchor = 'center') {
  const paths = rgbPaths(points, center, anchor);
  if (!gain || !paths.length || points.length < 2) return paths;
  const depths = points.map(point => columnDepths(point, gain, bandScale));
  const fills = ['#137fff', '#ffa916', '#fff4d9'];
  for (let layer = 0; layer < fills.length; layer += 1) {
    if (!depths.some(row => row[layer] > 0)) continue;
    const d = anchor === 'baseline'
      ? stepBaseline(points, depths.map(row => (center - row[layer]) * 2), center * 2)
      : stepMirrored(points, depths.map(row => row[layer]), center);
    paths.push({ fill: fills[layer], d });
  }
  return paths;
}
