import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const source = readFileSync(new URL('../src/lib/waveformDisplay.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 } }).outputText;
const { frequencyColor, waveColumn, rgbPaths, wavePaths, structureGain, smoothAmplitudes } = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`);
for (let i = 0; i < 3; i++) {
  const band = [0, 0, 0]; band[i] = 1;
  const rgb = frequencyColor(...band).match(/\d+/g).map(Number);
  assert.equal(rgb.indexOf(Math.max(...rgb)), i);
}
assert.equal(frequencyColor(.4,.2,.1), frequencyColor(.8,.4,.2));
const combined = waveColumn([.3, .8], [[1,0,0], [0,0,1]], 0, 2, 20);
assert.equal(combined.color, frequencyColor(1,0,1));
assert.equal(combined.loudest, 1);
assert.equal(waveColumn([.4], [], 0, 1, 20).color, frequencyColor(0,0,0));
assert.deepEqual(rgbPaths([{x:0,amp:0,color:'red'}, {x:1,amp:0,color:'red'}],20), []);
console.log('RGB waveforms: primary bands, level independence, mixed-column energy, missing bands and silence passed');

// Loudness uses one full-scale ruler. A quieter column stays quieter, and silence stays empty.
const halfLevel = waveColumn([1], [[0.2, 0.2, 0.1]], 0, 1, 20);
const fullLevel = waveColumn([1], [[0.4, 0.4, 0.2]], 0, 1, 20);
const silent = waveColumn([0], [[0, 0, 0]], 0, 1, 20);
assert.equal(halfLevel.color, fullLevel.color);
assert.ok(Math.abs(halfLevel.amp - 10) < 1e-9);
assert.ok(Math.abs(fullLevel.amp - 20) < 1e-9);
assert.equal(silent.amp, 0);

// Treble in a full mix can sit well below bass RMS. It must visibly change the hue.
const channels = (low, mid, high) => frequencyColor(low, mid, high).match(/\d+/g).map(Number);
const deep = channels(1, .08, .24);
const light = channels(1, .08, .45);
assert.ok(deep[2] > deep[0] && deep[0] > deep[1], 'bass-led mixed energy reads as violet');
assert.ok(light[2] > light[0] && light[1] > deep[1] + 50, 'more treble opens violet into lavender');
assert.ok(channels(1, .2, 0)[0] > 200, 'bass without treble stays warm');

// With equal peaks and a bass dropout, the blue bass band should disappear while the RGB rim stays visible.
const bands = [[.6, .2, .1], [0, .2, .1]];
{
  const points = bands.map((_, x) => ({ x, ...waveColumn([1, 1], bands, x, x + 1, 20) }));
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
    assert.ok(top0 < top1);
    if (layer === bass) {
      assert.equal(bottom0, 33.7); // Full measured energy fills 76% of the fixed-scale height.
      assert.ok(bottom0 > bottom1);
    }
  }
  assert.equal(coordinates(bass)[5], coordinates(mids)[5]); // No visible blue at x=1.
  assert.deepEqual(wavePaths(points, 20, structureGain([])), base);
}

const previewPoints = bands.map((_, x) => ({ x, ...waveColumn([1, 1], bands, x, x + 1, 20) }));
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
assert.match(deck, /wavePaths\(smoothAmplitudes\(samples\), height \/ 2, structureGain\(bands\), 1, 'baseline'\)/);
assert.match(library, /wavePaths\(smoothAmplitudes\(points\), 20, structureGain\(bands\), bandScale, 'baseline'\)/);
assert.equal(zone.includes("'baseline'"), false);
assert.match(zone, /wavePaths\(smoothAmplitudes\(points\), centerY, bandGain\)/);
assert.match(css, /\.ov-cue \{[^}]*top:\s*0;[^}]*bottom:\s*0;/);
assert.match(css, /\.ov-loop \{[^}]*top:\s*0;[^}]*bottom:\s*0;/);
assert.match(css, /\.ov-memory \{[^}]*top:\s*0;[^}]*bottom:\s*0;/);
assert.match(css, /\.ov-memory-cue \{[^}]*top:\s*0;[^}]*bottom:\s*0;/);
assert.match(css, /\.ov-marks \{[^}]*top:\s*4px;/);
assert.match(deck, /y1="0"/);
assert.match(deck, /y2="100"/);
console.log('Preview waveforms use the upper half from the bottom edge; the scrolling waveform stays mirrored');

const colorPeaks = [0.4, 0.9, 0.3];
const colorBands = [[0.6, 0.2, 0.1], [0, 0.2, 0.1], [0.2, 0.5, 0.4]];
const colorPoints = colorBands.map((_, x) => ({ x, ...waveColumn(colorPeaks, colorBands, x, x + 1, 20) }));
const colorsBeforeStack = ['rgb(253,71,86)', 'rgb(52,239,209)', 'rgb(30,195,232)'];
assert.deepEqual(colorPoints.map(point => point.color), colorsBeforeStack);
const colored = wavePaths(smoothAmplitudes(colorPoints), 20, structureGain(colorBands), 1, 'baseline');
assert.deepEqual(colored.map(path => path.fill), [...colorsBeforeStack, '#137fff', '#ffa916', '#fff4d9']);
assert.deepEqual(smoothAmplitudes(colorPoints).map(point => point.color), colorPoints.map(point => point.color));
console.log('Preview colors match the palette from before the stacked-color change');

const plateauBands = Array.from({ length: 21 }, (_, index) => (
  index >= 7 && index <= 13 ? [0.1, 0.05, 0.02] : [0.4, 0.2, 0.1]
));
const plateau = plateauBands.map((_, index) => ({
  x: index,
  ...waveColumn(plateauBands.map(() => 1), plateauBands, index, index + 1, 20),
}));
const softened = smoothAmplitudes(plateau);
assert.ok(Math.abs(plateau[3].amp - 14) < 1e-9);
assert.ok(Math.abs(plateau[10].amp - 3.4) < 1e-9);
assert.ok(Math.abs(softened[10].amp - plateau[10].amp) < 0.05);
assert.ok(softened[10].amp < softened[3].amp * 0.4);

const alternatingBands = Array.from({ length: 40 }, (_, index) => (index % 2 === 0 ? [0.8, 0, 0] : [0.2, 0, 0]));
const alternating = smoothAmplitudes(alternatingBands.map((_, index) => ({
  x: index,
  ...waveColumn(alternatingBands.map(() => 1), alternatingBands, index, index + 1, 20),
})));
assert.ok(alternating[16].amp / alternating[17].amp < 2);
console.log('Loudness stays on a fixed scale, and light smoothing keeps a breakdown');
