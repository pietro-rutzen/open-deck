import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const source = readFileSync(new URL('../src/lib/waveformDisplay.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 } }).outputText;
const { frequencyColor, waveColumn, rgbPaths } = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`);
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
