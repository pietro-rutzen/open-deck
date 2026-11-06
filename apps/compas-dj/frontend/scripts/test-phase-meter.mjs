import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const source = readFileSync(new URL('../src/lib/phaseMeter.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 } }).outputText;
const { phaseInBar } = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`);
assert.equal(phaseInBar(2, 0, .5), 0);
assert.equal(phaseInBar(2.0625, 0, .5), .125);
assert.equal(phaseInBar(2, .125, .5), 3.75);
assert.equal(phaseInBar(0, .125, .5), 3.75);
assert.equal(phaseInBar(16.25, 0, .5), phaseInBar(.25, 0, .5)); // loop or seek
assert.equal(phaseInBar(4.125, 0, .25), phaseInBar(8.25, 0, .5));
for (const interval of [NaN, 0, -.5, Infinity]) assert.equal(phaseInBar(0, 0, interval), null);
assert.equal(phaseInBar(NaN, 0, .5), null);
assert.equal(phaseInBar(1, NaN, .5), null);
console.log('Phase meter: downbeats, fractional drift, grid edits, seeks, loop wraps and unavailable grids passed');
