import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const source = readFileSync(new URL('../src/lib/waveformDisplay.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 } }).outputText;
const { frequencyColor, waveColumn, rgbPaths, wavePaths, structureGain } = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`);
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
