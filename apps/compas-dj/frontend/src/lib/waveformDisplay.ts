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
const CONTRAST = 2.2;
const LOW_BLEND = 0.6;
const COLUMN_PERCENTILE = 0.8;
const PHRASE_BARS = 8;
const PHRASE_SPLIT = 0.18;

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

export interface WavePoint { x: number; amp: number; color: string; rms?: Bands; crest?: number; low?: number; time?: number }

export interface WaveGrid { offsetSec: number; intervalSec: number; durationSec: number }

export interface WaveStructure {
  broadbandRef: number;
  lowRef: number;
  barHeld: Float64Array | null;
  barSec: number;
  offsetSec: number;
}

// One scale for the whole track keeps a breakdown quiet when the scrolling view zooms in.
export function structureGain(bands: Bands[]) {
  let peak = 0;
  for (const [l, m, h] of bands) peak = Math.max(peak, finite(l) + finite(m) + finite(h));
  return peak > 1e-5 ? 1 / peak : 0;
}

function columnMeasure(peaks: number[], bands: Bands[], start: number, end: number) {
  let loudest = start, peak = 0, peakSum = 0, low = 0, mid = 0, high = 0;
  const broadbandBins: number[] = [];
  const lowBins: number[] = [];
  const measured = bands.length === peaks.length;
  for (let i = start; i < end; i++) {
    const p = finite(peaks[i]);
    peakSum += p;
    if (p > peak) { peak = p; loudest = i; }
    if (measured) {
      const binLow = finite(bands[i][0]);
      const binMid = finite(bands[i][1]);
      const binHigh = finite(bands[i][2]);
      low += binLow ** 2;
      mid += binMid ** 2;
      high += binHigh ** 2;
      broadbandBins.push(binLow + binMid + binHigh);
      lowBins.push(binLow);
    } else {
      broadbandBins.push(p);
    }
  }
  const count = Math.max(1, end - start);
  const rms: Bands | undefined = measured
    ? [Math.sqrt(low / count), Math.sqrt(mid / count), Math.sqrt(high / count)] : undefined;
  const bandLevel = rms ? rms[0] + rms[1] + rms[2] : 0;
  return {
    loudest,
    rms,
    level: bandLevel > 0 ? bandLevel : peakSum / count,
    crest: percentile(broadbandBins, COLUMN_PERCENTILE),
    lowCrest: lowBins.length > 0 ? percentile(lowBins, COLUMN_PERCENTILE) : 0,
  };
}

// Pool power across the whole output column, so a kick cannot erase a quieter hi-hat
// from its color. Amplitude here is still absolute; row height is applied afterwards.
export function waveColumn(peaks: number[], bands: Bands[], start: number, end: number, radius: number) {
  const measured = columnMeasure(peaks, bands, start, end);
  return {
    loudest: measured.loudest,
    rms: measured.rms,
    amp: Math.min(1, measured.level) * radius,
    crest: measured.crest,
    low: measured.lowCrest,
    color: measured.rms ? frequencyColor(...measured.rms) : NEUTRAL,
  };
}

function percentile(values: readonly number[], fraction: number) {
  const ranked = [...values].sort((left, right) => left - right);
  if (ranked.length === 0) return 0;
  const index = Math.min(ranked.length - 1, Math.max(0, Math.ceil(fraction * ranked.length) - 1));
  return ranked[index] ?? 0;
}

function referenceOf(levels: readonly number[]) {
  const smoothed = smoothLevels(levels);
  const reference = percentile(smoothed, LOUD_SECTION_PERCENTILE);
  const loudest = smoothed.reduce((highest, level) => Math.max(highest, level), 0);
  return reference > 1e-5 ? reference : Math.max(loudest, 1e-5);
}

function sampledColumns(peaks: number[], bands: Bands[]) {
  const count = peaks.length;
  const columns = Math.min(count, 512);
  return Array.from({ length: columns }, (_, column) => {
    const start = Math.floor(column * count / columns);
    const end = Math.max(start + 1, Math.floor((column + 1) * count / columns));
    return columnMeasure(peaks, bands, start, end);
  });
}

// High percentile of the whole-track envelope, so one peak does not set the row height.
export function trackLevelReference(peaks: number[], bands: Bands[]) {
  if (peaks.length === 0) return 1;
  return referenceOf(sampledColumns(peaks, bands).map(column => column.crest));
}

export function trackLowReference(peaks: number[], bands: Bands[]) {
  if (peaks.length === 0 || bands.length !== peaks.length) return 0;
  return referenceOf(sampledColumns(peaks, bands).map(column => column.lowCrest));
}

function contrastAmplitude(relative: number, radius: number) {
  if (!(relative > 0)) return 0;
  const shaped = Math.min(1, relative) ** CONTRAST;
  return (LOUD_SECTION_FLOOR + (1 - LOUD_SECTION_FLOOR) * shaped) * radius;
}

function blendLevels(crest: number, low: number, broadbandRef: number, lowRef: number) {
  if (crest <= SILENT_LEVEL) return 0;
  const span = broadbandRef > 1e-5 ? broadbandRef : crest;
  const broadband = Math.min(1, crest / span);
  const lowUseful = lowRef > 1e-4 && lowRef > broadbandRef * 0.05;
  if (!lowUseful) return broadband;
  return (1 - LOW_BLEND) * broadband + LOW_BLEND * Math.min(1, low / lowRef);
}

// Silence stays empty. Audible columns keep a floor, then a power curve opens 2–4 dB gaps.
export function presentAmplitude(amp: number, reference: number, radius: number) {
  if (!(radius > 0) || !(amp > 0)) return 0;
  const level = amp / radius;
  if (level <= SILENT_LEVEL) return 0;
  const span = reference > 1e-5 ? reference : level;
  return contrastAmplitude(Math.min(1, level / span), radius);
}

function median(values: readonly number[]) {
  const ranked = [...values].sort((left, right) => left - right);
  return ranked[Math.floor(ranked.length / 2)] ?? 0;
}

function holdBarLevels(barLevel: readonly number[]) {
  const audible = barLevel.flatMap((level, bar) => level > 0 ? [bar] : []);
  const segments: number[][] = [];
  let current: number[] = [];
  for (const bar of audible) {
    const level = barLevel[bar] ?? 0;
    const anchor = barLevel[current[0] ?? bar] ?? level;
    const louder = Math.max(level, anchor);
    const gap = louder > 1e-8 ? Math.abs(level - anchor) / louder : 0;
    if (current.length > 0 && (gap > PHRASE_SPLIT || current.length >= PHRASE_BARS)) {
      segments.push(current);
      current = [bar];
    } else {
      current.push(bar);
    }
  }
  if (current.length > 0) segments.push(current);
  const held = barLevel.slice();
  for (const segment of segments) {
    const level = median(segment.map(bar => barLevel[bar] ?? 0));
    for (const bar of segment) held[bar] = level;
  }
  return held;
}

export function waveStructure(peaks: number[], bands: Bands[], grid: WaveGrid | null): WaveStructure {
  const broadbandRef = trackLevelReference(peaks, bands);
  const lowRef = trackLowReference(peaks, bands);
  const empty: WaveStructure = { broadbandRef, lowRef, barHeld: null, barSec: 0, offsetSec: grid?.offsetSec ?? 0 };
  if (!grid || !(grid.intervalSec > 0.05) || !(grid.durationSec > 0) || peaks.length === 0) return empty;
  const barSec = grid.intervalSec * 4;
  const barCount = Math.max(1, Math.ceil(grid.durationSec / barSec) + 1);
  const groups = Array.from({ length: barCount }, () => [] as number[]);
  const count = peaks.length;
  const stride = Math.max(1, Math.floor(count / 512));
  for (let index = 0; index < count; index += stride) {
    const end = Math.min(count, index + stride);
    const measured = columnMeasure(peaks, bands, index, end);
    const time = ((index + end) / 2) / count * grid.durationSec;
    const bar = Math.floor((time - grid.offsetSec) / barSec);
    if (bar < 0 || bar >= barCount) continue;
    const relative = blendLevels(measured.crest, measured.lowCrest, broadbandRef, lowRef);
    if (relative > 0) groups[bar].push(relative);
  }
  const barLevel = groups.map(values => values.length > 0 ? median(values) : 0);
  return { broadbandRef, lowRef, barHeld: Float64Array.from(holdBarLevels(barLevel)), barSec, offsetSec: grid.offsetSec };
}

export function shapeWavePoints<T extends { amp: number; crest?: number; low?: number; time?: number }>(
  points: readonly T[],
  radius: number,
  structure: WaveStructure,
): T[] {
  const relative = points.map(point => {
    const crest = point.crest ?? (radius > 0 ? point.amp / radius : 0);
    if (crest <= SILENT_LEVEL) return 0;
    if (structure.barHeld && structure.barSec > 0 && point.time != null) {
      const bar = Math.floor((point.time - structure.offsetSec) / structure.barSec);
      const held = structure.barHeld[bar];
      if (held != null && held > 0) return held;
    }
    return blendLevels(crest, point.low ?? 0, structure.broadbandRef, structure.lowRef);
  });
  const shaped = relative.map(level => contrastAmplitude(level, radius));
  const amplitudes = structure.barHeld ? shaped : smoothLevels(shaped);
  return points.map((point, index) => ({ ...point, amp: amplitudes[index] ?? 0 }));
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
