import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";

const source = readFileSync(new URL("../src/lib/memoryLoops.ts", import.meta.url), "utf8");
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText;
const { memoryLoopSpans, memoryLoopFractions, memoryLoopColor, memoryMatchesLoop } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled).toString("base64")}`
);

const saved = { slot: 0, in_frame: 48_000, out_frame: 96_000, active: false };
const active = { slot: 1, in_frame: 0, out_frame: 48_000, active: true };
assert.equal(memoryLoopColor(false), "#e6b325");
assert.equal(memoryLoopColor(true), "#e23b3b");
assert.equal(memoryMatchesLoop([saved], 48_000.2, 95_999.5), true);
assert.equal(memoryMatchesLoop([saved], 10, 20), false);

assert.deepEqual(memoryLoopSpans([saved, active], 48_000, 1, 0, 4, 400), [
  { slot: 0, x: 100, width: 100, active: false },
  { slot: 1, x: 0, width: 100, active: true },
]);
assert.deepEqual(memoryLoopSpans([saved], 48_000, 1, 3, 4, 400), []);
assert.deepEqual(memoryLoopSpans([saved], 0, 1, 0, 4, 400), []);
assert.deepEqual(memoryLoopSpans([{ ...saved, out_frame: 48_000 }], 48_000, 1, 0, 4, 400), []);

assert.deepEqual(memoryLoopFractions([saved, active], 192_000), [
  { slot: 0, left: 0.25, width: 0.25, active: false },
  { slot: 1, left: 0, width: 0.25, active: true },
]);
assert.deepEqual(memoryLoopFractions([saved], 0), []);

const deck = readFileSync(new URL("../src/components/Deck.tsx", import.meta.url), "utf8");
const css = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
assert.match(deck, /title="Memory loop"/);
assert.match(deck, /title="Active loop"/);
assert.match(css, /\.sq-check \{[^}]*border-radius:\s*0;/);

console.log("Memory loops: waveform spans, overview fractions, and square checks passed");
