import assert from 'node:assert/strict';
import { Window } from 'happy-dom';
import { createElement, act } from 'react';
import { createRoot } from 'react-dom/client';
import { createServer } from 'vite';

const dom = new Window({ url: 'http://localhost/' });
for (const name of ['window', 'document', 'HTMLElement', 'Element', 'Node', 'Document', 'Event', 'requestAnimationFrame', 'cancelAnimationFrame']) {
  try {
    if (dom[name] !== undefined) globalThis[name] = dom[name];
  } catch {
    /* Node owns a read-only navigator. */
  }
}
globalThis.window = dom;
globalThis.document = dom.document;
globalThis.IS_REACT_ACT_ENVIRONMENT = true;

const server = await createServer({
  root: new URL('..', import.meta.url).pathname,
  configFile: false,
  logLevel: 'error',
  server: { middlewareMode: true, hmr: false },
  appType: 'custom',
});
const feedbackModule = await server.ssrLoadModule('/src/hooks/useControllerFeedback.ts');
const feedbackLib = await server.ssrLoadModule('/src/lib/controllerFeedback.ts');
const { useControllerFeedback } = feedbackModule;
const { collectControllerFeedback, controllerFeedbackScanCount, resetControllerFeedbackScans } = feedbackLib;

const deckCount = 4;
const feedbackControlCount = 212;
const telemetryTickCount = 50;
const ticksAfterChanges = 20;
const loadedMeta = { title: 'ODQA Loaded' };
const replacedMeta = { title: 'ODQA Replaced' };
const sampleName = 'ODQA Hat';
const laterSampleName = 'ODQA Kick';
const restingTelemetry = { frame: 0, frameAt: 0, level: 0.01, peak: 0.9, rate: 44100, latencySecs: 0.008 };

const idleDeck = () => ({
  hotCues: Array.from({ length: 8 }, () => null),
  loop: { active: false, beats: null, inFrame: 0, outFrame: 0 },
  meta: null,
  keyPad: null,
  gain: 1,
  trim: 1,
  volume: 1,
  filter: 0,
  eq: { hi: 0, mid: 0, low: 0 },
  tempo: 1,
  playing: false,
  keylock: false,
  synced: false,
  echo: { active: false, beats: 0.5, depth: 0.5 },
  reverb: { active: false, size: 0.6, mix: 0.3 },
  flanger: { active: false, beats: 4, depth: 0.6 },
  crusher: { active: false, crush: 0.5, down: 0.3 },
});

const replaceDeck = (decks, index, patch) => decks.map((deck, deckIndex) => (
  deckIndex === index ? { ...deck, ...patch } : deck
));

const idleFeedback = () => {
  const values = new Map();
  for (let deck = 0; deck < deckCount; deck += 1) {
    for (let pad = 0; pad < 8; pad += 1) {
      values.set(`deck.${deck}.hotcue.${pad}`, 0);
      values.set(`deck.${deck}.beatloop.${pad}`, 0);
      values.set(`deck.${deck}.beatjump.${pad}`, 0);
      values.set(`deck.${deck}.key_shift.${pad}`, 0);
    }
    for (const name of ['loop.in', 'loop.out', 'loop.toggle', 'filter', 'eq.low', 'eq.mid', 'eq.high', 'tempo', 'play', 'keylock', 'sync']) {
      values.set(`deck.${deck}.${name}`, 0);
    }
    values.set(`deck.${deck}.gain`, 1);
    values.set(`deck.${deck}.trim`, 1);
    values.set(`deck.${deck}.volume`, 1);
    values.set(`deck.${deck}.headphone_cue`, 0);
    values.set(`fx.toggle.${deck}`, 0);
  }
  for (let slot = 0; slot < 16; slot += 1) values.set(`sampler.${slot}.trigger`, 0);
  values.set('mixer.crossfader', 0.5);
  values.set('mixer.cue_mix', 0);
  values.set('mixer.cue_volume', 0.8);
  values.set('sampler.gain', 0.9);
  return values;
};

const pairsToMap = (pairs) => {
  const values = new Map();
  for (const [control, value] of pairs) {
    assert.equal(values.has(control), false, `duplicate feedback for ${control}`);
    values.set(control, value);
  }
  return values;
};

const assertMapEqual = (actual, expected, label) => {
  assert.equal(actual.size, expected.size, label);
  for (const [control, value] of expected) {
    assert.equal(actual.get(control), value, `${label} ${control}`);
  }
};

const beatjumpValues = (deck, value) => Array.from({ length: 8 }, (_, pad) => [`deck.${deck}.beatjump.${pad}`, value]);

const emits = [];
const model = {
  decks: Array.from({ length: deckCount }, () => idleDeck()),
  selection: { effect: 'echo', deck: 0 },
  cued: new Set(),
  cueMix: 0,
  cueVolume: 0.8,
  names: Array.from({ length: 64 }, () => null),
  nameAt: (slot) => model.names[slot] ?? null,
  bank: 0,
  samplerGain: 0.9,
  crossfader: 0.5,
  telemetry: restingTelemetry,
  generation: 0,
  resend: () => {},
};

function Harness({ model: fixture, generation }) {
  const decks = fixture.decks.map((slices, index) => ({
    deck: index,
    state: {
      ...slices,
      frame: fixture.telemetry.frame + generation * 4410 + index,
      frameAt: fixture.telemetry.frameAt + generation,
      level: generation % 2 === 0 ? fixture.telemetry.level : fixture.telemetry.peak,
      rate: fixture.telemetry.rate + generation,
      latencySecs: fixture.telemetry.latencySecs + generation * 0.0001,
    },
  }));
  const api = useControllerFeedback({
    decks,
    beatFx: {
      selection: fixture.selection,
      isOn: (deck) => deck.state[fixture.selection.effect].active,
    },
    cue: { cued: fixture.cued, mix: fixture.cueMix, volume: fixture.cueVolume },
    sampler: { nameAt: fixture.nameAt, bank: fixture.bank, gain: fixture.samplerGain },
    crossfader: fixture.crossfader,
    emit: (control, value) => {
      emits.push([control, value]);
    },
  });
  fixture.resend = api.resend;
  return null;
}

const mount = async (node) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  await act(async () => {
    root.render(node);
  });
  return {
    async render(next) {
      await act(async () => {
        root.render(next);
      });
    },
    async unmount() {
      await act(async () => {
        root.unmount();
      });
      host.remove();
    },
  };
};

const paint = (view) => view.render(createElement(Harness, { model, generation: model.generation }));

const renderUnchanged = async (view, label, mutate) => {
  const scans = controllerFeedbackScanCount();
  const emitted = emits.length;
  mutate();
  model.generation += 1;
  await paint(view);
  assert.equal(controllerFeedbackScanCount(), scans, label);
  assert.equal(emits.length, emitted, label);
};

const renderChange = async (view, label, mutate, changed) => {
  const scans = controllerFeedbackScanCount();
  const emitted = emits.length;
  mutate();
  model.generation += 1;
  await paint(view);
  assert.equal(controllerFeedbackScanCount(), scans + 1, label);
  assertMapEqual(pairsToMap(emits.slice(emitted)), new Map(changed), label);
  for (const [control, value] of changed) expected.set(control, value);
};

resetControllerFeedbackScans();
const expected = idleFeedback();
assert.equal(expected.size, feedbackControlCount);
const view = await mount(createElement(Harness, { model, generation: model.generation }));
const resendFromMount = model.resend;

assert.equal(controllerFeedbackScanCount(), 1);
assertMapEqual(pairsToMap(emits), expected, 'initial');
assert.equal(expected.get('deck.0.play'), 0);
assert.equal(expected.get('deck.0.hotcue.0'), 0);
assert.equal(expected.get('deck.3.volume'), 1);
assert.equal(expected.get('mixer.cue_volume'), 0.8);
assert.equal(expected.get('sampler.gain'), 0.9);
assert.equal(expected.get('mixer.crossfader'), 0.5);
assert.equal(emits.filter(([, value]) => value === 0).length > 100, true);

for (let tick = 0; tick < telemetryTickCount; tick += 1) {
  model.telemetry = {
    frame: 44100 * (tick + 1),
    frameAt: 20 * (tick + 1),
    level: tick % 2 === 0 ? 0.01 : 0.9,
    peak: tick % 2 === 0 ? 0.9 : 0.01,
    rate: 44000 + tick,
    latencySecs: 0.008 + tick * 0.0002,
  };
  model.generation += 1;
  await paint(view);
}
assert.equal(controllerFeedbackScanCount(), 1);
assert.equal(emits.length, feedbackControlCount);

await renderUnchanged(view, 'echo depth', () => {
  model.decks = replaceDeck(model.decks, 0, { echo: { ...model.decks[0].echo, depth: 0.15 } });
});
await renderUnchanged(view, 'unselected reverb', () => {
  model.decks = replaceDeck(model.decks, 0, { reverb: { ...model.decks[0].reverb, active: true } });
});
await renderUnchanged(view, 'beat fx deck', () => {
  model.selection = { effect: model.selection.effect, deck: 2 };
});

const emittedBeforeResend = emits.length;
resendFromMount();
assert.equal(controllerFeedbackScanCount(), 1);
assertMapEqual(pairsToMap(emits.slice(emittedBeforeResend)), expected, 'session resend');
assert.equal(model.resend, resendFromMount);

const emittedBeforeEvent = emits.length;
await act(async () => {
  window.dispatchEvent(new Event('controller:resync'));
});
assert.equal(controllerFeedbackScanCount(), 1);
assertMapEqual(pairsToMap(emits.slice(emittedBeforeEvent)), expected, 'profile resync');

await renderChange(view, 'play', () => {
  model.decks = replaceDeck(model.decks, 0, { playing: true });
}, [['deck.0.play', 1]]);
await renderChange(view, 'hotcue', () => {
  const hotCues = model.decks[0].hotCues.slice();
  hotCues[0] = 44100;
  model.decks = replaceDeck(model.decks, 0, { hotCues });
}, [['deck.0.hotcue.0', 1]]);
await renderChange(view, 'beat loop', () => {
  model.decks = replaceDeck(model.decks, 1, {
    loop: { active: true, beats: 1, inFrame: 100, outFrame: 200, armed: false },
  });
}, [
  ['deck.1.beatloop.2', 1],
  ['deck.1.loop.in', 1],
  ['deck.1.loop.out', 1],
  ['deck.1.loop.toggle', 1],
]);
await renderChange(view, 'armed loop', () => {
  model.decks = replaceDeck(model.decks, 2, {
    loop: { active: false, beats: null, inFrame: 50, outFrame: 0, armed: true },
  });
}, [['deck.2.loop.in', 1]]);
await renderChange(view, 'track load', () => {
  model.decks = replaceDeck(model.decks, 0, { meta: loadedMeta });
}, beatjumpValues(0, 1));
await renderUnchanged(view, 'loaded metadata object', () => {
  model.decks = replaceDeck(model.decks, 0, { meta: replacedMeta });
});
await renderChange(view, 'key shift', () => {
  model.decks = replaceDeck(model.decks, 0, { keyPad: 3 });
}, [['deck.0.key_shift.3', 1]]);
await renderChange(view, 'gain', () => {
  model.decks = replaceDeck(model.decks, 0, { gain: 0.75 });
}, [['deck.0.gain', 0.75]]);
await renderChange(view, 'trim', () => {
  model.decks = replaceDeck(model.decks, 0, { trim: 0.5 });
}, [['deck.0.trim', 0.5]]);
await renderChange(view, 'volume', () => {
  model.decks = replaceDeck(model.decks, 2, { volume: 0.25 });
}, [['deck.2.volume', 0.25]]);
await renderChange(view, 'filter', () => {
  model.decks = replaceDeck(model.decks, 3, { filter: 0.5 });
}, [['deck.3.filter', 0.5]]);
await renderChange(view, 'eq', () => {
  model.decks = replaceDeck(model.decks, 0, { eq: { ...model.decks[0].eq, hi: 0.5 } });
}, [['deck.0.eq.high', 0.5]]);
await renderChange(view, 'tempo', () => {
  model.decks = replaceDeck(model.decks, 0, { tempo: 1.5 });
}, [['deck.0.tempo', 50]]);
await renderChange(view, 'keylock', () => {
  model.decks = replaceDeck(model.decks, 0, { keylock: true });
}, [['deck.0.keylock', 1]]);
await renderChange(view, 'sync', () => {
  model.decks = replaceDeck(model.decks, 3, { synced: true });
}, [['deck.3.sync', 1]]);
await renderChange(view, 'headphone cue', () => {
  model.cued = new Set([1]);
}, [['deck.1.headphone_cue', 1]]);
await renderChange(view, 'cue mix', () => {
  model.cueMix = 0.25;
}, [['mixer.cue_mix', 0.25]]);
await renderChange(view, 'cue volume', () => {
  model.cueVolume = 1;
}, [['mixer.cue_volume', 1]]);
await renderChange(view, 'sampler load', () => {
  const names = model.names.slice();
  names[0] = sampleName;
  model.names = names;
  model.nameAt = (slot) => model.names[slot] ?? null;
}, [['sampler.0.trigger', 1]]);
await renderChange(view, 'sampler bank', () => {
  model.bank = 1;
}, [['sampler.0.trigger', 0]]);
await renderChange(view, 'sampler bank slot', () => {
  const names = model.names.slice();
  names[17] = laterSampleName;
  model.names = names;
  model.nameAt = (slot) => model.names[slot] ?? null;
}, [['sampler.1.trigger', 1]]);
await renderChange(view, 'sampler gain', () => {
  model.samplerGain = 0.5;
}, [['sampler.gain', 0.5]]);
await renderChange(view, 'crossfader', () => {
  model.crossfader = 0.25;
}, [['mixer.crossfader', 0.25]]);
await renderChange(view, 'selected echo', () => {
  model.decks = replaceDeck(model.decks, 0, { echo: { ...model.decks[0].echo, active: true } });
}, [['fx.toggle.0', 1]]);
await renderChange(view, 'effect type', () => {
  model.selection = { effect: 'reverb', deck: model.selection.deck };
}, []);
await renderChange(view, 'selected reverb on another deck', () => {
  model.decks = replaceDeck(model.decks, 3, { reverb: { ...model.decks[3].reverb, active: true } });
}, [['fx.toggle.3', 1]]);
await renderChange(view, 'effect type returns', () => {
  model.selection = { effect: 'echo', deck: 0 };
}, [['fx.toggle.3', 0]]);

for (let tick = 0; tick < ticksAfterChanges; tick += 1) {
  model.telemetry = {
    frame: 88200 + tick * 2000,
    frameAt: 5000 + tick,
    level: 0.2,
    peak: 0.8,
    rate: -22050,
    latencySecs: 0.02,
  };
  model.generation += 1;
  await paint(view);
}
const scansAfterChanges = controllerFeedbackScanCount();
const emitsAfterChanges = emits.length;

const emittedBeforeCurrentResend = emits.length;
resendFromMount();
assert.equal(controllerFeedbackScanCount(), scansAfterChanges);
assertMapEqual(pairsToMap(emits.slice(emittedBeforeCurrentResend)), expected, 'resend current');
assert.equal(expected.get('deck.0.play'), 1);
assert.equal(expected.get('deck.0.hotcue.0'), 1);
assert.equal(expected.get('deck.1.beatloop.2'), 1);
assert.equal(expected.get('deck.0.beatjump.0'), 1);
assert.equal(expected.get('deck.0.key_shift.3'), 1);
assert.equal(expected.get('deck.1.headphone_cue'), 1);
assert.equal(expected.get('sampler.1.trigger'), 1);
assert.equal(expected.get('fx.toggle.0'), 1);
assert.equal(expected.get('fx.toggle.3'), 0);
assert.equal(expected.get('deck.0.play') === 0, false);

await renderChange(view, 'unload', () => {
  model.decks = replaceDeck(model.decks, 0, { meta: null });
}, [
  ...beatjumpValues(0, 0),
  ['deck.0.key_shift.3', 0],
]);

await view.unmount();
const emitsAfterUnmount = emits.length;
await act(async () => {
  window.dispatchEvent(new Event('controller:resync'));
});
assert.equal(emits.length, emitsAfterUnmount);
assert.equal(controllerFeedbackScanCount(), scansAfterChanges + 1);

resetControllerFeedbackScans();
collectControllerFeedback({
  decks: [{ deck: 0, state: idleDeck() }],
  effectOn: () => false,
  cued: new Set(),
  cueMix: 0,
  cueVolume: 0.8,
  samplerNameAt: () => null,
  samplerBank: 0,
  samplerGain: 0.9,
  crossfader: 0.5,
});
assert.equal(controllerFeedbackScanCount(), 1);

console.log(`Controller feedback scans: mount 1; ${telemetryTickCount + ticksAfterChanges} telemetry ticks added 0; scans after control updates ${scansAfterChanges}; scans after unload ${scansAfterChanges + 1}. Session resend and controller:resync added 0 scans. Emits after control updates ${emitsAfterChanges} = 212 initial + 212 session + 212 resync + 35 deltas. One effect-type scan resent nothing because the LED value was unchanged. Physical FLX4 not exercised.`);
await server.close();
