import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { Window } from 'happy-dom';
import * as React from 'react';
import { createElement, act } from 'react';
import { createRoot } from 'react-dom/client';
import ts from 'typescript';

const source = readFileSync(new URL('../src/lib/cueCountdown.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 } }).outputText;
const { nextHotCue, formatCueDistance, SHOW_CUE_COUNTDOWN } = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`);
const loop = { active: false, inFrame: 48000, outFrame: 144000 };
const count = (cues, frame, looping = false) => nextHotCue(cues, frame, 48000, 0.5, { ...loop, active: looping });
assert.deepEqual(count([120000, 96000, null, 0], 0), { slot: 1, beats: 4 });
assert.deepEqual(count([120000, 96000], 96000), { slot: 0, beats: 1 });
assert.deepEqual(count([96001], 24000), { slot: 0, beats: 4 });
assert.deepEqual(count([48000, 144000, 150000], 120000, true), { slot: 0, beats: 1 });
assert.equal(count([0, 144000], 96000, true), null);
assert.equal(count([NaN, null, 96000], 96000), null);
assert.deepEqual(nextHotCue([Math.round(8 * 44100 * 60 / 127)], 0, 44100, 60 / 127, loop), { slot: 0, beats: 8 });
assert.equal(nextHotCue([48000], 0, 48000, 0, loop), null);
assert.equal(nextHotCue([48000], 0, 0, 0.5, loop), null);
assert.deepEqual([1, 4, 5, 8, 11].map(formatCueDistance), ['1 beat', '1 bar', '1 bar + 1 beat', '2 bars', '2 bars + 3 beats']);
assert.equal(SHOW_CUE_COUNTDOWN, false);
const deckSource = readFileSync(new URL('../src/components/Deck.tsx', import.meta.url), 'utf8');
assert.match(deckSource, /<CueCountdownReadout /);
assert.equal(deckSource.includes('CUE {'), false);
assert.equal(formatCueDistance(82), '20 bars + 2 beats');

const dom = new Window({ url: 'http://localhost/' });
globalThis.window = dom;
globalThis.document = dom.document;
globalThis.HTMLElement = dom.HTMLElement;
globalThis.Element = dom.Element;
globalThis.Node = dom.Node;
globalThis.Document = dom.Document;
globalThis.IS_REACT_ACT_ENVIRONMENT = true;

const componentSource = readFileSync(new URL('../src/components/CueCountdown.tsx', import.meta.url), 'utf8');
const compiledComponent = ts.transpileModule(componentSource, {
  compilerOptions: { jsx: ts.JsxEmit.React, target: ts.ScriptTarget.ES2022 },
}).outputText;
const componentBody = compiledComponent
  .replace(/import[\s\S]*?;\s*/g, '')
  .replace(/^export /gm, '');
const { CueCountdownReadout } = new Function(
  'React',
  'formatCueDistance',
  'SHOW_CUE_COUNTDOWN',
  `${componentBody}\nreturn { CueCountdownReadout };`,
)(React, formatCueDistance, SHOW_CUE_COUNTDOWN);
const host = document.createElement('div');
document.body.appendChild(host);
const root = createRoot(host);
const twentyBarsAndTwoBeats = { slot: 0, beats: 82 };
await act(async () => {
  root.render(createElement(CueCountdownReadout, {
    cue: twentyBarsAndTwoBeats,
    color: '#f80',
  }));
});
assert.equal(host.textContent, '');
assert.equal(host.querySelector('.tile'), null);
assert.equal(host.innerHTML.includes('CUE'), false);
assert.equal(host.innerHTML.includes('bars'), false);
await act(async () => {
  root.unmount();
});
console.log('Hot-cue countdown passed including seeks, fractional beats, loop wrap and missing grid.');
