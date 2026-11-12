import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const source = readFileSync(new URL('../src/lib/waveformDisplay.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 } }).outputText;
const { frequencyColor, waveColumn, rgbPaths, wavePaths, structureGain, smoothAmplitudes, trackLevelReference, presentAmplitude } = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`);
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
    const ysAt = (x) => {
      const values = coordinates(layer);
      const ys = [];
      for (let offset = 0; offset < values.length; offset += 2) {
        if (values[offset] === x) ys.push(values[offset + 1]);
      }
      return ys;
    };
    for (const x of [0, 1]) {
      const ys = ysAt(x);
      assert.equal(Math.min(...ys) + Math.max(...ys), 40);
    }
    assert.ok(Math.min(...ysAt(0)) < Math.min(...ysAt(1)));
    if (layer === bass) {
      assert.ok(ysAt(0).includes(33.7));
      assert.ok(Math.max(...ysAt(0)) > Math.max(...ysAt(1)));
    }
  }
  const ysAtLayer = (layer, x) => {
    const values = coordinates(layer);
    const ys = [];
    for (let offset = 0; offset < values.length; offset += 2) {
      if (values[offset] === x) ys.push(values[offset + 1]);
    }
    return ys.sort((left, right) => left - right);
  };
  assert.deepEqual(ysAtLayer(bass, 1), ysAtLayer(mids, 1));
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
assert.ok(numbers(mirroredPreview[0]).filter((_, offset) => offset % 2 === 1).every((y) => y < 40));

const deck = readFileSync(new URL('../src/components/Deck.tsx', import.meta.url), 'utf8');
const library = readFileSync(new URL('../src/components/LibraryVisuals.tsx', import.meta.url), 'utf8');
const zone = readFileSync(new URL('../src/components/WaveformZone.tsx', import.meta.url), 'utf8');
const css = readFileSync(new URL('../src/styles.css', import.meta.url), 'utf8');
assert.match(deck, /wavePaths\(smoothAmplitudes\(samples\), height \/ 2, structureGain\(bands\), 1, 'baseline'\)/);
assert.match(deck, /x: \(\(column \+ 0\.5\) \/ count\) \* width/);
assert.match(deck, /trackLevelReference\(peaks, bands\)/);
assert.match(deck, /presentAmplitude\(measured\.amp, reference, radius\)/);
assert.match(library, /wavePaths\(smoothAmplitudes\(points\), 20, structureGain\(bands\), bandScale, 'baseline'\)/);
assert.match(library, /trackLevelReference\(peaks, bands\)/);
assert.match(library, /presentAmplitude\(measured\.amp, reference, radius\)/);
assert.equal(zone.includes("'baseline'"), false);
assert.match(zone, /wavePaths\(smoothAmplitudes\(points\), centerY, bandGain\)/);
assert.match(zone, /trackLevelReference\(peaks, bands\)/);
assert.match(zone, /presentAmplitude\(column\.amp, levelReference, radius\)/);
assert.equal(source.includes('replay_gain'), false);
assert.equal(deck.includes('setDeckReplayGain'), false);
assert.equal(library.includes('setDeckReplayGain'), false);
assert.equal(zone.includes('setDeckReplayGain'), false);
const replayAnalysis = readFileSync(new URL('../../../../crates/compas-dsp/src/analysis.rs', import.meta.url), 'utf8');
const replayPrepare = readFileSync(new URL('../../src-tauri/src/deck_prepare.rs', import.meta.url), 'utf8');
const replayEngine = readFileSync(new URL('../../src-tauri/src/lib.rs', import.meta.url), 'utf8');
assert.match(replayAnalysis, /const TARGET_RMS: f64 = 0\.125;/);
assert.match(replayPrepare, /installed_replay_gain\(analysis::replaygain_linear\(&buffer\.samples\)\)/);
assert.match(replayEngine, /EngineMsg::SetDeckReplayGain \{\s*deck,\s*gain: loaded\.replay_gain,/);
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

const rippleBands = Array.from({ length: 40 }, (_, index) => (
  index % 2 === 0 ? [0.5, 0.2, 0.1] : [0.46, 0.18, 0.09]
));
const ripple = smoothAmplitudes(rippleBands.map((_, index) => ({
  x: index,
  ...waveColumn(rippleBands.map(() => 1), rippleBands, index, index + 1, 20),
})));
assert.ok(ripple[16].amp / ripple[17].amp < 1.04);

const stepBands = Array.from({ length: 40 }, (_, index) => (index % 2 === 0 ? [0.8, 0, 0] : [0.2, 0, 0]));
const preservedStep = smoothAmplitudes(stepBands.map((_, index) => ({
  x: index,
  ...waveColumn(stepBands.map(() => 1), stepBands, index, index + 1, 20),
})));
assert.ok(preservedStep[16].amp / preservedStep[17].amp > 3);

const abruptBands = Array.from({ length: 32 }, (_, index) => {
  if (index < 6 || index >= 26) return [0, 0, 0];
  if (index < 20) return [0.5, 0.2, 0.1];
  return [0.08, 0.04, 0.02];
});
const abruptPeaks = abruptBands.map((band) => band[0] + band[1] + band[2]);
const abrupt = smoothAmplitudes(abruptBands.map((_, index) => ({
  x: index,
  ...waveColumn(abruptPeaks, abruptBands, index, index + 1, 20),
})));
assert.ok(abrupt[5].amp < 0.05);
assert.ok(Math.abs(abrupt[6].amp - 16) < 0.05);
assert.ok(Math.abs(abrupt[19].amp - 16) < 0.05);
assert.ok(Math.abs(abrupt[20].amp - 2.8) < 0.05);
assert.ok(Math.abs(abrupt[25].amp - 2.8) < 0.05);
assert.ok(abrupt[26].amp < 0.05);
const abruptPaths = wavePaths(abrupt, 20, structureGain(abruptBands), 1, 'baseline');
const crestByX = new Map();
for (const path of abruptPaths) {
  const values = path.d.match(/-?\d+\.\d+/g).map(Number);
  for (let offset = 0; offset < values.length; offset += 2) {
    const key = values[offset].toFixed(1);
    crestByX.set(key, Math.min(crestByX.get(key) ?? 40, values[offset + 1]));
  }
}
const crestHeight = (x) => 40 - crestByX.get(x.toFixed(1));
const loudHeight = crestHeight(10.5);
const tailHeights = [21.5, 22.5, 23.5, 24.5].map(crestHeight);
assert.ok(tailHeights.every((height) => Math.abs(height - tailHeights[0]) < 0.15));
assert.ok(tailHeights[0] > 3 && tailHeights[0] < loudHeight * 0.35);
assert.ok(crestHeight(4.5) < 0.3);
assert.ok(crestHeight(6.5) > loudHeight * 0.9);
const highXs = [...crestByX.keys()].map(Number).filter((x) => crestHeight(x) > loudHeight * 0.7);
const quietXs = [...crestByX.keys()].map(Number).filter((x) => x >= Math.max(...highXs) && crestHeight(x) < loudHeight * 0.3 && crestHeight(x) > 1);
const silentXs = [...crestByX.keys()].map(Number).filter((x) => x >= Math.max(...quietXs) && crestHeight(x) < 0.3);
assert.ok(Math.min(...quietXs) - Math.max(...highXs) <= 2);
assert.ok(Math.min(...silentXs) - Math.max(...quietXs) <= 2);
assert.ok(Math.min(...highXs) - 4.5 <= 2);

const radius = 20;
const fitted = (rows) => {
  const peaks = rows.map((band) => band[0] + band[1] + band[2]);
  const reference = trackLevelReference(peaks, rows);
  return {
    reference,
    amps: rows.map((_, index) => presentAmplitude(waveColumn(peaks, rows, index, index + 1, radius).amp, reference, radius)),
  };
};
const quietMaster = fitted(Array.from({ length: 48 }, (_, index) => index < 40 ? [0.04, 0.02, 0.01] : [0, 0, 0]));
const loudMaster = fitted(Array.from({ length: 48 }, (_, index) => index < 40 ? [0.4, 0.2, 0.1] : [0, 0, 0]));
const meanOf = (values) => values.reduce((sum, value) => sum + value, 0) / values.length;
const quietLoudSection = meanOf(quietMaster.amps.slice(4, 36));
const loudLoudSection = meanOf(loudMaster.amps.slice(4, 36));
assert.ok(Math.abs(quietLoudSection - radius) < 0.05);
assert.ok(Math.abs(loudLoudSection - radius) < 0.05);
assert.equal(quietMaster.amps[44], 0);
assert.equal(loudMaster.amps[44], 0);

const spiked = Array.from({ length: 40 }, (_, index) => index === 20 ? [0.8, 0.1, 0.1] : [0.12, 0.05, 0.03]);
const spikeFit = fitted(spiked);
assert.ok(spikeFit.reference < 0.35);
assert.ok(spikeFit.reference > 0.15);
assert.ok(spikeFit.amps[4] > radius * 0.95);

const broken = Array.from({ length: 40 }, (_, index) => (
  index >= 28 && index < 36 ? [0.2, 0.1, 0.05] : [0.4, 0.2, 0.1]
));
const brokenFit = fitted(broken);
assert.ok(Math.abs(brokenFit.amps[30] - (0.1 + 0.9 * 0.5) * radius) < 0.6);
assert.ok(brokenFit.amps[30] < brokenFit.amps[8] * 0.7);
assert.ok(brokenFit.amps[30] > brokenFit.amps[8] * 0.45);

const fittedPoints = smoothAmplitudes(abruptBands.map((_, index) => {
  const measured = waveColumn(abruptPeaks, abruptBands, index, index + 1, radius);
  return {
    x: index,
    ...measured,
    amp: presentAmplitude(measured.amp, trackLevelReference(abruptPeaks, abruptBands), radius),
  };
}));
const fittedPaths = wavePaths(fittedPoints, 20, structureGain(abruptBands), 1, 'baseline');
const fittedCrest = new Map();
for (const path of fittedPaths) {
  const values = path.d.match(/-?\d+\.\d+/g).map(Number);
  for (let offset = 0; offset < values.length; offset += 2) {
    const key = values[offset].toFixed(1);
    fittedCrest.set(key, Math.min(fittedCrest.get(key) ?? 40, values[offset + 1]));
  }
}
const fittedHeight = (x) => 40 - fittedCrest.get(x.toFixed(1));
const fittedLoud = fittedHeight(10.5);
const fittedTail = [21.5, 22.5, 23.5, 24.5].map(fittedHeight);
assert.ok(fittedTail.every((height) => Math.abs(height - fittedTail[0]) < 0.15));
assert.ok(fittedTail[0] > radius * 0.1);
assert.ok(fittedHeight(4.5) < 0.3);
assert.ok(fittedHeight(6.5) > fittedLoud * 0.9);
const fittedHigh = [...fittedCrest.keys()].map(Number).filter((x) => fittedHeight(x) > fittedLoud * 0.7);
const fittedQuiet = [...fittedCrest.keys()].map(Number).filter((x) => x >= Math.max(...fittedHigh) && fittedHeight(x) < fittedLoud * 0.45 && fittedHeight(x) > 1);
const fittedSilent = [...fittedCrest.keys()].map(Number).filter((x) => x >= Math.max(...fittedQuiet) && fittedHeight(x) < 0.3);
assert.ok(Math.min(...fittedQuiet) - Math.max(...fittedHigh) <= 2);
assert.ok(Math.min(...fittedSilent) - Math.max(...fittedQuiet) <= 2);
assert.deepEqual(
  fittedPoints.map((point) => point.color),
  abruptBands.map((_, index) => waveColumn(abruptPeaks, abruptBands, index, index + 1, radius).color),
);
console.log('Each track uses its own loud section, and level steps keep a vertical edge');
