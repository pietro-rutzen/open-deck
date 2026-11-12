import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const source = readFileSync(new URL('../src/lib/waveformDisplay.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 } }).outputText;
const { frequencyColor, waveColumn, rgbPaths, wavePaths, structureGain, overviewPaths } = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`);
for (let i = 0; i < 3; i++) {
  const band = [0, 0, 0]; band[i] = 1;
  const rgb = frequencyColor(...band).match(/\d+/g).map(Number);
  assert.equal(rgb.indexOf(Math.max(...rgb)), i);
}
assert.equal(frequencyColor(.4,.2,.1), frequencyColor(.8,.4,.2));
const combined = waveColumn([.3, .8], [[1,0,0], [0,0,1]], 0, 2, 1, 20);
assert.equal(combined.color, frequencyColor(1,0,1));
assert.equal(combined.loudest, 1);
assert.equal(waveColumn([.4], [], 0, 1, 1, 20).color, frequencyColor(0,0,0));
assert.deepEqual(rgbPaths([{x:0,amp:0,color:'red'}, {x:1,amp:0,color:'red'}],20), []);
console.log('RGB waveforms: primary bands, level independence, mixed-column energy, missing bands and silence passed');

// Preview lift keeps a quiet section readable while preserving silence and peak headroom.
for (const peak of [0, .25, 1]) {
  const args = [[peak], [[0, 0, 1]], 0, 1, 1, 19];
  const preview = waveColumn(...args, 'preview');
  const detail = waveColumn(...args, 'detail');
  assert.equal(preview.color, detail.color);
  if (peak === .25) assert.ok(preview.amp > detail.amp);
  else assert.equal(preview.amp, detail.amp);
}

// Treble in a full mix can sit well below bass RMS. It must visibly change the hue.
const channels = (low, mid, high) => frequencyColor(low, mid, high).match(/\d+/g).map(Number);
const deep = channels(1, .08, .24);
const light = channels(1, .08, .45);
assert.ok(deep[2] > deep[0] && deep[0] > deep[1], 'bass-led mixed energy reads as violet');
assert.ok(light[2] > light[0] && light[1] > deep[1] + 50, 'more treble opens violet into lavender');
assert.ok(channels(1, .2, 0)[0] > 200, 'bass without treble stays warm');

// With equal peaks and a bass dropout, the blue bass band should disappear while the RGB rim stays visible.
const bands = [[.6, .2, .1], [0, .2, .1]];
for (const view of ['preview', 'detail']) {
  const points = bands.map((_, x) => ({ x, ...waveColumn([1, 1], bands, x, x + 1, 1, 20, view) }));
  const base = rgbPaths(points, 20);
  const hybrid = wavePaths(points, 20, structureGain(bands));
  assert.deepEqual(hybrid.slice(0, base.length), base);
  const [bass, mids, highs] = hybrid.slice(base.length);
  const coordinates = layer => layer.d.match(/-?\d+\.\d+/g).map(Number);
  const thumbnail = wavePaths(points, 20, structureGain(bands), 0.5);
  assert.deepEqual(thumbnail.slice(0, base.length), base);
  for (const [index, layer] of [bass, mids, highs].entries()) {
    const original = coordinates(layer);
    const scaled = coordinates(thumbnail[base.length + index]);
    for (let y = 1; y < original.length; y += 2) {
      assert.ok(Math.abs(scaled[y] - (20 + (original[y] - 20) / 2)) <= 0.1);
    }
    const [x0, top0, x1, top1, x2, bottom1, x3, bottom0] = coordinates(layer);
    assert.deepEqual([x0, x1, x2, x3], [0, 1, 1, 0]);
    assert.equal(top0 + bottom0, 40);
    assert.equal(top1 + bottom1, 40);
    assert.ok(top0 >= 4.8 && top1 >= 4.8);
    if (layer === bass) {
      assert.equal(bottom0, 35.2); // The body fills 76% at full measured energy.
      assert.ok(bottom1 >= 33.6); // Even quieter band data fills at least 68%.
      assert.ok(bottom0 > bottom1);
    }
    else assert.ok(bottom1 > bottom0);
  }
  assert.equal(coordinates(bass)[5], coordinates(mids)[5]); // No visible blue at x=1.
  assert.deepEqual(wavePaths(points, 20, structureGain([])), base);
}

const previewPoints = bands.map((_, x) => ({ x, ...waveColumn([1, 1], bands, x, x + 1, 1, 20, 'preview') }));
const mirroredPreview = wavePaths(previewPoints, 20, structureGain(bands));
const baselinePreview = wavePaths(previewPoints, 20, structureGain(bands), 1, 'baseline');
const numbers = (path) => path.d.match(/-?\d+\.\d+/g).map(Number);
assert.equal(baselinePreview.length, mirroredPreview.length);
for (let index = 0; index < baselinePreview.length; index += 1) {
  const mirroredYs = numbers(mirroredPreview[index]).filter((_, offset) => offset % 2 === 1);
  const baselineYs = numbers(baselinePreview[index]).filter((_, offset) => offset % 2 === 1);
  const upper = mirroredYs.filter((y) => y <= 20);
  const crest = Math.min(...upper);
  assert.ok(baselineYs.every((y) => y <= 40.05 && y >= crest * 2 - 0.2));
  assert.ok(baselineYs.some((y) => Math.abs(y - 40) < 0.05));
  assert.ok(Math.abs(Math.min(...baselineYs) - crest * 2) <= 0.2);
}
assert.equal(numbers(baselinePreview.at(-1)).at(-1), 40);
assert.ok(numbers(baselinePreview[0]).join(',').length < numbers(mirroredPreview[0]).join(',').length);

const deck = readFileSync(new URL('../src/components/Deck.tsx', import.meta.url), 'utf8');
const library = readFileSync(new URL('../src/components/LibraryVisuals.tsx', import.meta.url), 'utf8');
const zone = readFileSync(new URL('../src/components/WaveformZone.tsx', import.meta.url), 'utf8');
const css = readFileSync(new URL('../src/styles.css', import.meta.url), 'utf8');
assert.match(deck, /overviewPaths\(peaks, bands, size\.width, size\.height\)/);
assert.match(library, /overviewPaths\(peaks, bands, 256, 40, bandScale\)/);
assert.equal(zone.includes('overviewPaths'), false);
assert.equal(zone.includes("'baseline'"), false);
assert.match(zone, /wavePaths\(points, centerY, bandGain\)/);
assert.match(css, /\.ov-cue \{[^}]*top:\s*0;[^}]*bottom:\s*0;/);
assert.match(css, /\.ov-loop \{[^}]*top:\s*0;[^}]*bottom:\s*0;/);
assert.match(css, /\.ov-memory \{[^}]*top:\s*0;[^}]*bottom:\s*0;/);
assert.match(css, /\.ov-memory-cue \{[^}]*top:\s*0;[^}]*bottom:\s*0;/);
assert.match(css, /\.ov-marks \{[^}]*top:\s*4px;/);
assert.match(deck, /y1="0"/);
assert.match(deck, /y2="100"/);
console.log('Preview waveforms use the upper half from the bottom edge; the scrolling waveform stays mirrored');

const heightOf = (path, x) => {
  const values = path.d.match(/-?\d+\.\d+/g).map(Number);
  const ys = [];
  for (let index = 0; index < values.length; index += 2) {
    if (Math.abs(values[index] - x) < 0.05) ys.push(values[index + 1]);
  }
  assert.equal(ys.length, 2);
  return Math.max(...ys) - Math.min(...ys);
};
const crestOf = (paths, x) => Math.min(...paths.flatMap(path => {
  const values = path.d.match(/-?\d+\.\d+/g).map(Number);
  const ys = [];
  for (let index = 0; index < values.length; index += 2) {
    if (Math.abs(values[index] - x) < 0.05) ys.push(values[index + 1]);
  }
  return ys;
}));

const loudBand = [1, 0.4, 0.15];
const quietBand = [0.2, 0.05, 0.02];
const bandSum = (band) => band[0] + band[1] + band[2];
const energyRatio = bandSum(quietBand) / bandSum(loudBand);
const overviewPeaks = Array.from({ length: 21 }, () => 1);
const overviewBands = overviewPeaks.map((_, index) => (index >= 7 && index <= 13 ? quietBand : loudBand));
const stacked = overviewPaths(overviewPeaks, overviewBands, 21, 40);
assert.deepEqual(stacked.map(path => path.fill), ['#137fff', '#ffa916', '#fff4d9']);
assert.equal(stacked.some(path => path.fill.startsWith('rgb')), false);
const overviewHeight = 40;
const overviewSpan = overviewHeight * 0.94;
const loudCrest = crestOf(stacked, 3.5);
const quietCrest = crestOf(stacked, 10.5);
assert.ok(Math.abs((overviewHeight - loudCrest) - overviewSpan) <= 0.15);
assert.ok(Math.abs((overviewHeight - quietCrest) / overviewSpan - energyRatio) <= 0.02);
assert.ok(quietCrest > loudCrest + 20);
for (const [band, previous] of stacked.slice(1).entries()) {
  const lower = stacked[band];
  const lowerValues = lower.d.match(/-?\d+\.\d+/g).map(Number);
  const upperValues = previous.d.match(/-?\d+\.\d+/g).map(Number);
  const shared = [];
  for (let index = 0; index < lowerValues.length; index += 2) {
    if (Math.abs(lowerValues[index] - 3.5) < 0.05) shared.push(lowerValues[index + 1]);
  }
  const above = [];
  for (let index = 0; index < upperValues.length; index += 2) {
    if (Math.abs(upperValues[index] - 3.5) < 0.05) above.push(upperValues[index + 1]);
  }
  assert.equal(Math.min(...shared), Math.max(...above));
}
assert.ok(heightOf(stacked[2], 3.5) < heightOf(stacked[1], 3.5));
assert.ok(heightOf(stacked[1], 3.5) < heightOf(stacked[0], 3.5));
assert.ok(stacked[0].d.includes(' 40.0'));

const alternating = Array.from({ length: 40 }, (_, index) => (index % 2 === 0 ? [1, 0, 0] : [0.1, 0, 0]));
const blended = overviewPaths(alternating.map(() => 1), alternating, 40, 40);
const evenHeight = heightOf(blended[0], 16.5);
const oddHeight = heightOf(blended[0], 17.5);
assert.ok(evenHeight / oddHeight < 2);
assert.ok(oddHeight > 0);

const spike = Array.from({ length: 21 }, () => 0);
spike[10] = 1;
const spiked = overviewPaths(spike, spike.map(value => [value, 0, 0]), 21, 40);
const spikeHeight = heightOf(spiked[0], 10.5);
const neighborHeight = heightOf(spiked[0], 11.5);
assert.ok(Math.abs(spikeHeight - overviewSpan) <= 0.15);
assert.ok(neighborHeight > spikeHeight * 0.5 && neighborHeight < spikeHeight);

const unbanded = [1, 1, 1, 1, 1, 0.25, 0.25, 0.25, 0.25, 0.25, 1, 1, 1, 1, 1];
const linear = overviewPaths(unbanded, [], 15, 40);
assert.deepEqual(linear.map(path => path.fill), ['#137fff']);
assert.ok(Math.abs(heightOf(linear[0], 7.5) / heightOf(linear[0], 2.5) - 0.25) <= 0.03);
assert.deepEqual(overviewPaths([0, 0, 0], [[0, 0, 0], [0, 0, 0], [0, 0, 0]], 3, 40), []);
assert.deepEqual(overviewPaths([], [], 10, 40), []);
console.log('Overview loudness is linear to the track peak, stacked, and lightly smoothed');
