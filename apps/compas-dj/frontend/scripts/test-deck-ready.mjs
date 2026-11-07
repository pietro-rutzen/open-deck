import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';

const source = readFileSync(new URL('../src/lib/deckReady.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText;
const ready = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`);

const loaded = {
  deck: 0,
  generation: 4,
  path: '/music/same.wav',
  bpm: 0,
  bpm_confidence: 0,
  first_beat_sec: 0,
  beat_interval_sec: 0,
  key_camelot: '',
  key_name: '',
  band_peaks: [],
  replay_gain: 1.8,
  analysis_pending: true,
  analysis_failed: false,
  peaks: [0.2, 0.4],
};

const analysis = {
  deck: 0,
  generation: 4,
  path: '/music/same.wav',
  bpm: 126,
  bpm_confidence: 0.8,
  first_beat_sec: 0.12,
  beat_interval_sec: 60 / 126,
  key_camelot: '8A',
  key_name: 'Am',
  band_peaks: [[0.1, 0.2, 0.3]],
  replay_gain: 1.7,
  replay_gain_applied: false,
};

assert.equal(ready.bpmLabel(loaded, 1), '…');
assert.equal(ready.gridAvailable(loaded), false);
assert.equal(ready.laneTempo(loaded, 1), '… · …');
assert.equal(ready.bpmLabel(null, 1), '—');

const merged = ready.mergeDeckAnalysis(loaded, analysis);
assert.equal(merged.analysis_pending, false);
assert.equal(merged.bpm, 126);
assert.equal(merged.replay_gain, 1.8);
assert.deepEqual(merged.peaks, [0.2, 0.4]);
assert.equal(ready.gridAvailable(merged), true);
assert.equal(ready.bpmLabel(merged, 1), '126.00');
assert.equal(ready.laneTempo(merged, 1), '126.00 · 8A');

const replacement = { ...loaded, generation: 5 };
assert.equal(ready.mergeDeckAnalysis(replacement, analysis), null);
assert.equal(ready.mergeDeckAnalysis({ ...loaded, path: '/music/other.wav' }, analysis), null);
assert.equal(ready.mergeDeckAnalysis({ ...loaded, deck: 1 }, analysis), null);

const otherDeck = ready.mergeDeckAnalysis(
  { ...loaded, deck: 1, generation: 2, path: '/music/b.wav' },
  { ...analysis, deck: 1, generation: 2, path: '/music/b.wav', bpm: 100, beat_interval_sec: 0.6 },
);
assert.equal(otherDeck.bpm, 100);
assert.equal(ready.gridAvailable(loaded), false);

const failed = ready.clearPendingAnalysis(loaded, { deck: 0, generation: 4, path: '/music/same.wav' });
assert.equal(failed.analysis_pending, false);
assert.equal(failed.analysis_failed, true);
assert.equal(ready.bpmLabel(failed, 1), '—');
assert.equal(ready.gridAvailable(failed), false);
assert.equal(ready.clearPendingAnalysis(replacement, analysis), null);
assert.equal(ready.clearPendingAnalysis(merged, analysis), null);

const unusable = ready.mergeDeckAnalysis(loaded, {
  ...analysis,
  bpm: Number.NaN,
  beat_interval_sec: 0,
  replay_gain_applied: true,
  replay_gain: 0.4,
});
assert.equal(unusable.bpm, 0);
assert.equal(unusable.beat_interval_sec, 0);
assert.equal(unusable.replay_gain, 0.4);
assert.equal(ready.gridAvailable(unusable), false);

console.log('Deck ready: pending grid, same-path generation, both-deck identity, failure, and installed loudness hold passed');
