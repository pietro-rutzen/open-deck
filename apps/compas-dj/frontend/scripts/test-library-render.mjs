import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { performance } from 'node:perf_hooks';
import { createElement, memo, useState, act } from 'react';
import { createRoot } from 'react-dom/client';
import ts from 'typescript';

const compile = (relativePath) => {
  const source = readFileSync(new URL(relativePath, import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  return import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`);
};

const rowsModule = await compile('../src/lib/libraryRows.ts');
const surfaceModule = await compile('../src/lib/librarySurface.ts');
const controllerModule = await compile('../src/lib/libraryController.ts');
const { withTrackNumbers } = rowsModule;
const { libraryPropsEqual } = surfaceModule;
const { controllerLoadPath, stepControllerRow } = controllerModule;

const track = (path) => ({ kind: 'track', key: `track:${path}`, track: { path } });
const visibleRows = [
  { kind: 'up', key: 'up:library', path: null, name: 'All tracks' },
  { kind: 'folder', key: 'folder:/music/a', path: '/music/a', name: 'a' },
  { kind: 'separator', key: 'sep:slash', name: '/' },
  track('/music/a/two.wav'),
  track('/music/a/one.wav'),
];

const numbered = withTrackNumbers(visibleRows);
assert.deepEqual(
  numbered.map((row) => row.trackNumber),
  [null, null, null, 1, 2],
);
assert.deepEqual(
  numbered.map((row) => row.key),
  visibleRows.map((row) => row.key),
);
assert.deepEqual(withTrackNumbers(visibleRows.slice(3).reverse()).map((row) => [row.key, row.trackNumber]), [
  ['track:/music/a/one.wav', 1],
  ['track:/music/a/two.wav', 2],
]);
assert.deepEqual(withTrackNumbers([track('/music/a/one.wav')]).map((row) => row.trackNumber), [1]);
assert.deepEqual(withTrackNumbers([]), []);

const stepped = stepControllerRow(numbered, 'track:/music/a/two.wav', 1);
assert.equal(stepped.key, 'track:/music/a/one.wav');
assert.equal(controllerLoadPath(numbered, stepped.key), '/music/a/one.wav');
assert.equal(stepControllerRow(numbered, 'folder:/music/a', 1).key, 'sep:slash');
assert.equal(stepControllerRow(numbered, 'track:/music/a/one.wav', 1).key, 'track:/music/a/one.wav');
assert.equal(stepControllerRow(numbered, null, -1).key, 'track:/music/a/one.wav');
assert.throws(
  () => controllerLoadPath(numbered, 'folder:/music/a'),
  /folder is selected/,
);
assert.throws(
  () => controllerLoadPath(numbered, 'sep:slash'),
  /folder is selected/,
);

const precedingTrackNumbers = (rows) =>
  rows.map((row, index) =>
    row.kind === 'track'
      ? rows.slice(0, index).filter((item) => item.kind === 'track').length + 1
      : null,
  );

const countedRows = (rows) => {
  let reads = 0;
  return {
    reads: () => reads,
    rows: rows.map((row) => {
      const record = { ...row };
      Object.defineProperty(record, 'kind', {
        enumerable: true,
        get() {
          reads += 1;
          return row.kind;
        },
      });
      return record;
    }),
  };
};

const rowCount = 4000;
const syntheticTracks = Array.from({ length: rowCount }, (_, index) => track(`/music/${index}.wav`));
const linearProbe = countedRows(syntheticTracks);
const linearNumbers = withTrackNumbers(linearProbe.rows);
const quadraticProbe = countedRows(syntheticTracks);
const quadraticNumbers = precedingTrackNumbers(quadraticProbe.rows);
const plainLinearStarted = performance.now();
const plainLinear = withTrackNumbers(syntheticTracks);
const plainLinearMs = performance.now() - plainLinearStarted;
const plainQuadraticStarted = performance.now();
const plainQuadratic = precedingTrackNumbers(syntheticTracks);
const plainQuadraticMs = performance.now() - plainQuadraticStarted;
assert.equal(plainLinear.length, plainQuadratic.length);
assert.equal(linearProbe.reads(), rowCount * 2);
assert.equal(quadraticProbe.reads(), (rowCount * (rowCount + 1)) / 2);
assert.ok(linearProbe.reads() < quadraticProbe.reads());
assert.deepEqual(
  linearNumbers.map((row) => row.trackNumber),
  quadraticNumbers,
);

const sameSurface = {
  loadedPaths: ['/music/a/two.wav', undefined, undefined, undefined],
  nextDecks: [
    { index: 0, letter: 'A' },
    { index: 1, letter: 'B' },
  ],
  keyNotation: 'camelot',
  focusTarget: 'library',
  focusSeq: 3,
  onSamplerLoad: () => {},
};
assert.equal(
  libraryPropsEqual(sameSurface, {
    ...sameSurface,
    loadedPaths: [...sameSurface.loadedPaths],
    nextDecks: sameSurface.nextDecks.map((deck) => ({ ...deck })),
  }),
  true,
);
for (const changed of [
  { loadedPaths: ['/music/a/one.wav', undefined, undefined, undefined] },
  { loadedPaths: ['/music/a/two.wav', '/music/other.wav', undefined, undefined] },
  { nextDecks: [{ index: 0, letter: 'C' }, { index: 1, letter: 'B' }] },
  { nextDecks: [{ index: 2, letter: 'A' }, { index: 1, letter: 'B' }] },
  { nextDecks: [{ index: 0, letter: 'A' }] },
  { keyNotation: 'musical' },
  { focusTarget: 'crates' },
  { focusSeq: 4 },
  { onSamplerLoad: () => {} },
]) {
  assert.equal(libraryPropsEqual(sameSurface, { ...sameSurface, ...changed }), false);
}

const listeners = () => ({ addEventListener() {}, removeEventListener() {} });
const view = {
  HTMLIFrameElement: function HTMLIFrameElement() {},
  HTMLElement: function HTMLElement() {},
  Element: function Element() {},
  Node: function Node() {},
  document: null,
};
const element = (tag) => ({
  ...listeners(),
  nodeType: 1,
  nodeName: String(tag).toUpperCase(),
  tagName: String(tag).toUpperCase(),
  ownerDocument: null,
  childNodes: [],
  style: {},
  parentNode: null,
  namespaceURI: 'http://www.w3.org/1999/xhtml',
  textContent: '',
  appendChild(child) {
    child.parentNode = this;
    this.childNodes.push(child);
    return child;
  },
  removeChild(child) {
    this.childNodes = this.childNodes.filter((item) => item !== child);
    return child;
  },
  insertBefore(child) {
    return this.appendChild(child);
  },
  setAttribute() {},
  removeAttribute() {},
  getAttribute() {
    return null;
  },
  hasAttribute() {
    return false;
  },
});
const documentShim = {
  ...listeners(),
  nodeType: 9,
  defaultView: view,
  createElement: element,
  createElementNS: (_namespace, tag) => element(tag),
  createTextNode(text) {
    return { nodeType: 3, nodeName: '#text', textContent: String(text), ownerDocument: documentShim };
  },
  documentElement: null,
  body: null,
  activeElement: null,
};
view.document = documentShim;
const rootElement = element('div');
rootElement.ownerDocument = documentShim;
documentShim.documentElement = rootElement;
documentShim.body = rootElement;
globalThis.document = documentShim;
globalThis.window = view;
globalThis.HTMLElement = view.HTMLElement;
globalThis.IS_REACT_ACT_ENVIRONMENT = true;

const mountLibrary = async (compareProps) => {
  const host = element('div');
  host.ownerDocument = documentShim;
  const root = createRoot(host);
  const seen = {
    renders: 0,
    numbers: [],
    loadedPath: undefined,
    selectedKey: null,
    samplerLoad: null,
  };
  const stableLoad = (slot, path) => {
    seen.loadedSlot = slot;
    seen.loadedSample = path;
  };
  const controls = {};
  const Observed = memo(function Observed(props) {
    const [rows, setRows] = useState(visibleRows);
    const [selectedKey, setSelectedKey] = useState('track:/music/a/two.wav');
    controls.setRows = setRows;
    controls.select = setSelectedKey;
    seen.renders += 1;
    seen.numbers = withTrackNumbers(rows).map((row) => row.trackNumber);
    seen.loadedPath = props.loadedPaths[0];
    seen.selectedKey = selectedKey;
    seen.samplerLoad = props.onSamplerLoad;
    return null;
  }, compareProps);
  function Host() {
    const [frame, setFrame] = useState(0);
    const [path, setPath] = useState('/music/a/two.wav');
    const [letter, setLetter] = useState('A');
    const [samplerLoad, setSamplerLoad] = useState(() => stableLoad);
    controls.setFrame = setFrame;
    controls.setPath = setPath;
    controls.setLetter = setLetter;
    controls.setSamplerLoad = setSamplerLoad;
    void frame;
    return createElement(Observed, {
      loadedPaths: [path, undefined, undefined, undefined],
      nextDecks: [
        { index: 0, letter },
        { index: 1, letter: 'B' },
      ],
      keyNotation: 'camelot',
      focusTarget: 'library',
      focusSeq: 3,
      onSamplerLoad: samplerLoad,
    });
  }
  await act(async () => {
    root.render(createElement(Host));
  });
  return { root, seen, controls, stableLoad };
};

const tickCount = 200;
const shallow = await mountLibrary(undefined);
for (let frame = 1; frame <= tickCount; frame += 1) {
  await act(async () => {
    shallow.controls.setFrame(frame);
  });
}
assert.equal(shallow.seen.renders, 1 + tickCount);
await act(async () => {
  shallow.root.unmount();
});

const bounded = await mountLibrary(libraryPropsEqual);
for (let frame = 1; frame <= tickCount; frame += 1) {
  await act(async () => {
    bounded.controls.setFrame(frame);
  });
}
assert.equal(bounded.seen.renders, 1);
assert.deepEqual(bounded.seen.numbers, [null, null, null, 1, 2]);
assert.equal(bounded.seen.selectedKey, 'track:/music/a/two.wav');
assert.equal(bounded.seen.samplerLoad, bounded.stableLoad);

await act(async () => {
  bounded.controls.setPath('/music/a/one.wav');
});
assert.equal(bounded.seen.renders, 2);
assert.equal(bounded.seen.loadedPath, '/music/a/one.wav');

await act(async () => {
  bounded.controls.setLetter('C');
});
assert.equal(bounded.seen.renders, 3);

await act(async () => {
  bounded.controls.select('track:/music/a/one.wav');
});
assert.equal(bounded.seen.renders, 4);
assert.equal(bounded.seen.selectedKey, 'track:/music/a/one.wav');
assert.equal(controllerLoadPath(numbered, bounded.seen.selectedKey), '/music/a/one.wav');

const replacementRows = [track('/music/b/new.wav')];
await act(async () => {
  bounded.controls.setRows(replacementRows);
});
assert.equal(bounded.seen.renders, 5);
assert.deepEqual(bounded.seen.numbers, [1]);

let sampled = null;
await act(async () => {
  bounded.controls.setSamplerLoad(() => (slot, path) => {
    sampled = `${slot}:${path}`;
  });
});
assert.equal(bounded.seen.renders, 6);
bounded.seen.samplerLoad(2, '/music/b/new.wav');
assert.equal(sampled, '2:/music/b/new.wav');

await act(async () => {
  bounded.controls.setFrame(tickCount + 1);
});
assert.equal(bounded.seen.renders, 6);
assert.equal(bounded.seen.selectedKey, 'track:/music/a/one.wav');
assert.deepEqual(bounded.seen.numbers, [1]);
await act(async () => {
  bounded.root.unmount();
});

console.log(
  [
    'synthetic library numbering (not native app latency)',
    `rows=${rowCount}`,
    `linear kind reads=${linearProbe.reads()}`,
    `quadratic kind reads=${quadraticProbe.reads()}`,
    `plain linear ms=${plainLinearMs.toFixed(3)}`,
    `plain quadratic ms=${plainQuadraticMs.toFixed(3)}`,
    'synthetic library renders (not native app latency)',
    `playhead ticks=${tickCount}`,
    `shallow memo renders=${shallow.seen.renders}`,
    `library surface renders after ticks=${1}`,
    'library surface renders after path, deck, selection, metadata, callback=6',
    'library surface renders after one further tick=6',
  ].join('\n'),
);
