import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const source = readFileSync(new URL('../src/lib/hotCueMarkers.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 } }).outputText;
const { hotCueMarkers, CUE_COLORS } = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`);
const cues = [0, null, 96000, NaN, -1];
assert.deepEqual(hotCueMarkers(cues, 48000, 1, 0, 8, 800, 24), [
  {slot: 0, x: 0, color: CUE_COLORS[0]}, {slot: 2, x: 200, color: CUE_COLORS[2]}
]);
assert.equal(hotCueMarkers([96000], 48000, 2, 0, 8, 800, 24)[0].x, 100);
assert.equal(hotCueMarkers([96000], 48000, 1, 1, 4, 1600, 12)[0].x, 400);
assert.deepEqual(hotCueMarkers([0, 96000], 48000, 1, 3, 8, 800, 24), []);
assert.deepEqual(hotCueMarkers([0], 0, 1, 0, 8, 800, 24), []);
console.log('Hot cue markers: palette, source-rate, tempo, resize/zoom and clipping passed');
