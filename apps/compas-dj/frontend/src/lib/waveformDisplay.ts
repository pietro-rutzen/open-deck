// Display-only. Height encodes amplitude and hue encodes the measured spectral mix.
type Bands = readonly [number, number, number];
const finite = (value: number) => Number.isFinite(value) ? Math.max(0, value) : 0;
export const RGB_WAVE_KEY = 'Waveform colors: bass red, mids green, treble sky blue. Mixed frequencies blend.';
const NEUTRAL = '#8996a4';

export function waveformScale(peaks: number[]) {
  let max = 0;
  for (const p of peaks) max = Math.max(max, finite(p));
  return max > 0.00001 ? Math.min(6, 1 / max) : 1;
}

// Quantized channels keep the SVG path count bounded at every zoom level.
export function frequencyColor(low: number, mid: number, high: number) {
  const r = finite(low), g = finite(mid), b = finite(high);
  const max = Math.max(r, g, b);
  if (max === 0) return NEUTRAL;
  // Strong band contrasts survive downscaling into a narrow library column.
  const level = (value: number) => Math.round(7 * (value / max) ** 1.35) / 7;
  const red = level(r), green = level(g), blue = level(b);
  const channel = (a: number, b: number, c: number) => Math.round(Math.max(a * red, b * green, c * blue));
  return `rgb(${channel(255, 66, 8)},${channel(16, 255, 151)},${channel(33, 10, 255)})`;
}

export interface WavePoint { x: number; amp: number; color: string }

// Pool power across the whole output column, so a kick cannot erase a quieter hi-hat
// from its color. The peak from that same interval supplies the silhouette.
export function waveColumn(peaks: number[], bands: Bands[], start: number, end: number,
  gain: number, radius: number, view: 'detail' | 'preview' = 'detail') {
  let loudest = start, peak = 0, low = 0, mid = 0, high = 0;
  const measured = bands.length === peaks.length;
  for (let i = start; i < end; i++) {
    const p = finite(peaks[i]);
    if (p > peak) { peak = p; loudest = i; }
    if (measured) {
      low += finite(bands[i][0]) ** 2;
      mid += finite(bands[i][1]) ** 2;
      high += finite(bands[i][2]) ** 2;
    }
  }
  const exponent = view === 'preview' ? 0.56 : 0.72;
  return { loudest, amp: Math.pow(Math.min(1, peak * gain), exponent) * radius,
    color: frequencyColor(Math.sqrt(low), Math.sqrt(mid), Math.sqrt(high)) };
}

// Adjacent columns share their interpolated edge. Batch all polygons of one hue
// into one SVG path to avoid creating an element per audio bin.
export function rgbPaths(points: WavePoint[], center: number) {
  const paths = new Map<string, string[]>();
  const coord = (v: number) => v.toFixed(1);
  for (let i = 0; i < points.length; i++) {
    const p = points[i], before = points[i - 1] ?? p, after = points[i + 1] ?? p;
    if (p.amp + before.amp + after.amp === 0) continue;
    const l = coord((p.x + before.x) / 2), r = coord((p.x + after.x) / 2), x = coord(p.x);
    const a = (p.amp + before.amp) / 2, b = (p.amp + after.amp) / 2;
    const path = `M${l} ${coord(center-a)}L${x} ${coord(center-p.amp)} ${r} ${coord(center-b)} ${r} ${coord(center+b)} ${x} ${coord(center+p.amp)} ${l} ${coord(center+a)}Z`;
    const group = paths.get(p.color);
    if (group) group.push(path); else paths.set(p.color, [path]);
  }
  return Array.from(paths, ([fill, segments]) => ({ fill, d: segments.join('') }));
}
