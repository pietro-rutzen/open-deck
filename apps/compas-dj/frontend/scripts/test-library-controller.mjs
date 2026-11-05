// Run from frontend: node scripts/test-library-controller.mjs
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const source = readFileSync(new URL('../src/lib/libraryController.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 } }).outputText;
const { selectedControllerRow, stepControllerRow, controllerLoadPath } = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`);
const track = path => ({ key: `track:${path}`, kind: 'track', track: { path } });
const a = track('/a'), b = track('/b'), c = track('/c');
let rows = [a, b, c];
let key = a.key;
// Consecutive encoder events before any presentation update preserve every step.
key = stepControllerRow(rows, key, 1).key;
key = stepControllerRow(rows, key, 1).key;
assert.equal(controllerLoadPath(rows, key), '/c');
// Repeated refresh, new objects, sorting, PFL/deck changes must not retarget selected identity.
for (let i = 0; i < 10000; i++) {
  rows = i % 2 ? [track('/b'), track('/c'), track('/a')] : [track('/a'), track('/b'), track('/c')];
  assert.equal(controllerLoadPath(rows, key), '/c');

}
// Missing rows during refresh never silently choose the row at an obsolete numeric index.
assert.throws(() => controllerLoadPath([], key), /Select a track/);
assert.equal(controllerLoadPath([c], key), '/c');
assert.throws(() => controllerLoadPath([a, b], key), /Select a track/);
const folder = { key: 'folder:/music', kind: 'folder' };
assert.throws(() => controllerLoadPath([folder, a], folder.key), /folder is selected/);
assert.throws(() => controllerLoadPath([a], null), /Select a track/);
assert.equal(stepControllerRow([folder, a], null, 1), folder);
assert.equal(stepControllerRow([folder, a], null, -1), a);
assert.equal(stepControllerRow([a, b, c], c.key, 20), c);
assert.equal(stepControllerRow([a, b, c], a.key, -20), a);
assert.equal(stepControllerRow([], key, 1), undefined);
assert.equal(stepControllerRow([a], key, NaN), undefined);
assert.equal(selectedControllerRow([a, b], b.key), b);
console.log('library controller: identity, rapid browse, 10,000 refresh cycles and explicit LOAD failures passed');
