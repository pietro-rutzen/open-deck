import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { performance } from 'node:perf_hooks';
import { Window } from 'happy-dom';
import { createElement, Profiler, useState, act } from 'react';
import { createRoot } from 'react-dom/client';
import { createServer } from 'vite';
import ts from 'typescript';

const compile = (relativePath) => {
  const source = readFileSync(new URL(relativePath, import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  return import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`);
};

const { stepControllerRow } = await compile('../src/lib/libraryController.ts');
const {
  LIBRARY_HEADER_HEIGHT,
  LIBRARY_OVERSCAN,
  LIBRARY_ROW_HEIGHT,
  libraryMountedRanges,
  scrollTopForLibraryRow,
  visibleLibraryRowWindow,
} = await compile('../src/lib/libraryWindow.ts');

const viewportHeight = LIBRARY_HEADER_HEIGHT + LIBRARY_ROW_HEIGHT * 8;
const opened = visibleLibraryRowWindow(2000, 0, viewportHeight);
assert.equal(opened.start, 0);
assert.equal(opened.end, 8 + LIBRARY_OVERSCAN);
assert.equal(opened.topHeight, 0);
assert.equal(opened.bottomHeight, (2000 - opened.end) * LIBRARY_ROW_HEIGHT);
assert.ok(opened.end < 40);

const scrolled = visibleLibraryRowWindow(2000, 1000, viewportHeight);
assert.equal(scrolled.start, Math.floor(1000 / LIBRARY_ROW_HEIGHT) - LIBRARY_OVERSCAN);
assert.ok(scrolled.end > Math.ceil((1000 + viewportHeight - LIBRARY_HEADER_HEIGHT) / LIBRARY_ROW_HEIGHT));
assert.equal(scrolled.topHeight, scrolled.start * LIBRARY_ROW_HEIGHT);
assert.ok(scrolled.end - scrolled.start < 40);

const revealed = scrollTopForLibraryRow(100, 0, viewportHeight);
assert.equal(revealed, LIBRARY_HEADER_HEIGHT + 101 * LIBRARY_ROW_HEIGHT - viewportHeight);
const stayed = scrollTopForLibraryRow(2, 0, viewportHeight);
assert.equal(stayed, 0);
const home = scrollTopForLibraryRow(0, revealed, viewportHeight);
assert.equal(home, 0);
assert.deepEqual(
  libraryMountedRanges(100, 120, 4, 2000),
  [{ start: 4, end: 5 }, { start: 100, end: 120 }],
);
assert.deepEqual(libraryMountedRanges(100, 120, 110, 2000), [{ start: 100, end: 120 }]);
assert.deepEqual(libraryMountedRanges(0, 0, null, 0), []);

const dom = new Window({ url: 'http://localhost/' });
for (const name of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Document', 'CSS', 'localStorage', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'MutationObserver', 'IntersectionObserver', 'ResizeObserver', 'KeyboardEvent', 'MouseEvent', 'InputEvent']) {
  try {
    if (dom[name] !== undefined) globalThis[name] = dom[name];
  } catch {
    /* Node owns a read-only navigator. */
  }
}
globalThis.window = dom;
globalThis.document = dom.document;
globalThis.IS_REACT_ACT_ENVIRONMENT = true;

const track = (index, title = `Track ${String(index).padStart(4, '0')}`) => ({
  path: `/tmp/odqa-synthetic/track-${index}.wav`,
  title,
  artist: 'Fixture',
  album: '',
  duration_ms: 120000,
  bpm: 120,
  key_camelot: '8A',
  key_name: 'Am',
  grid_offset_sec: 0,
  gain: 1,
  play_count: 0,
  last_played_at: null,
  tags: [],
});

const server = await createServer({
  root: new URL('..', import.meta.url).pathname,
  configFile: false,
  logLevel: 'error',
  server: { middlewareMode: true, hmr: false },
  appType: 'custom',
  plugins: [{
    name: 'library-window-source',
    enforce: 'pre',
    resolveId(id) {
      if (id.endsWith('/hooks/useLibrary') || id.endsWith('/hooks/useLibrary.ts')) return '\0library-window-source';
      return null;
    },
    load(id) {
      if (id !== '\0library-window-source') return null;
      return `
        import { useState } from "react";
        export function useLibrary() {
          const [tracks, setTracks] = useState(globalThis.__LIBRARY_WINDOW_TRACKS);
          globalThis.__LIBRARY_WINDOW_SET_TRACKS = setTracks;
          return { tracks, busy: false, add: async () => {}, remove() {}, refresh() {} };
        }
      `;
    },
  }],
});
const { Library } = await server.ssrLoadModule('/src/components/Library.tsx');

const searchHandler = (host, node) => {
  const names = Object.getOwnPropertyNames(node);
  const propKey = names.find((key) => key.startsWith('__reactProps$'));
  if (propKey && node[propKey]?.onChange) return node[propKey].onChange;
  const fiberKey = names.find((key) => key.startsWith('__reactFiber$') || key.startsWith('__reactContainer$'));
  let fiber = fiberKey ? node[fiberKey] : null;
  if (!fiber) {
    const hostKey = Object.getOwnPropertyNames(host).find((key) => key.startsWith('__reactContainer$'));
    fiber = hostKey ? host[hostKey] : null;
  }
  const seen = new Set();
  const stack = fiber ? [fiber] : [];
  while (stack.length) {
    const current = stack.pop();
    if (!current || seen.has(current)) continue;
    seen.add(current);
    if (current.stateNode === node && current.memoizedProps?.onChange) return current.memoizedProps.onChange;
    if (current.child) stack.push(current.child);
    if (current.sibling) stack.push(current.sibling);
  }
  throw new Error('search handler missing');
};

const sheetOf = (host) => {
  const sheet = host.querySelector('.tl-sheet');
  if (!sheet) throw new Error('library sheet missing');
  let height = viewportHeight;
  Object.defineProperty(sheet, 'clientHeight', { configurable: true, get: () => height });
  sheet.setViewportHeight = (next) => {
    height = next;
  };
  return sheet;
};

const mountedPaths = (host) => [...host.querySelectorAll('[data-track-path]')].map((row) => row.getAttribute('data-track-path'));
const selectedRow = (host) => host.querySelector('[aria-selected="true"][data-track-path]');

const pressKey = (host, key) => {
  const target = selectedRow(host) ?? host.querySelector('[data-track-index="0"]');
  if (!target) throw new Error(`no row for ${key}`);
  target.focus();
  target.dispatchEvent(new window.KeyboardEvent('keydown', { key, bubbles: true }));
};

const stableLoadedPaths = [undefined, undefined, undefined, undefined];
const stableNextDecks = [{ index: 0, letter: 'A' }, { index: 1, letter: 'B' }];
const stableSamplerLoad = () => {};

const renderLibrary = async (tracks, commits = null) => {
  globalThis.__LIBRARY_WINDOW_TRACKS = tracks;
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  const controls = {};
  function Host() {
    const [frame, setFrame] = useState(0);
    controls.setFrame = setFrame;
    void frame;
    const library = createElement(Library, {
      loadedPaths: stableLoadedPaths,
      nextDecks: stableNextDecks,
      keyNotation: 'camelot',
      onSamplerLoad: stableSamplerLoad,
    });
    if (!commits) return library;
    return createElement(Profiler, {
      id: 'library',
      onRender(_id, _phase, actualDuration) {
        commits.push(actualDuration);
      },
    }, library);
  }
  await act(async () => {
    root.render(createElement(Host));
  });
  const sheet = sheetOf(host);
  await act(async () => {
    sheet.dispatchEvent(new window.Event('scroll'));
  });
  return { host, root, sheet, controls };
};

const behaviorTracks = [
  track(0, 'Zzz-last'),
  ...Array.from({ length: 198 }, (_, index) => track(index + 1)),
  track(199, 'Aaa-first'),
];
const behavior = await renderLibrary(behaviorTracks);
assert.ok(mountedPaths(behavior.host).length < 40, `mounted ${mountedPaths(behavior.host).length} rows for a short viewport`);
assert.equal(behavior.host.querySelector('[data-track-index="0"] .tl-tag')?.textContent, '01');

await act(async () => {
  pressKey(behavior.host, 'ArrowDown');
  pressKey(behavior.host, 'ArrowDown');
});
assert.equal(selectedRow(behavior.host)?.getAttribute('data-track-index'), '2');
assert.equal(selectedRow(behavior.host)?.getAttribute('data-track-path'), '/tmp/odqa-synthetic/track-2.wav');

let controllerKey = 'track:/tmp/odqa-synthetic/track-2.wav';
const controllerRows = behaviorTracks.map((item) => ({ key: `track:${item.path}`, kind: 'track', track: { path: item.path } }));
for (let step = 0; step < 30; step += 1) {
  controllerKey = stepControllerRow(controllerRows, controllerKey, 1).key;
  await act(async () => {
    pressKey(behavior.host, 'ArrowDown');
  });
}
assert.equal(selectedRow(behavior.host)?.getAttribute('data-browse-key'), controllerKey);
assert.equal(selectedRow(behavior.host)?.getAttribute('data-track-index'), '32');
assert.ok(Number(behavior.sheet.scrollTop) > 0);

await act(async () => {
  pressKey(behavior.host, 'End');
});
assert.equal(selectedRow(behavior.host)?.getAttribute('data-track-path'), '/tmp/odqa-synthetic/track-199.wav');
assert.equal(selectedRow(behavior.host)?.getAttribute('data-track-index'), '199');
const endScroll = behavior.sheet.scrollTop;
const endTop = LIBRARY_HEADER_HEIGHT + 199 * LIBRARY_ROW_HEIGHT;
assert.ok(endScroll > behavior.sheet.clientHeight);
assert.ok(endTop >= endScroll + LIBRARY_HEADER_HEIGHT - 0.5);
assert.ok(endTop + LIBRARY_ROW_HEIGHT <= endScroll + behavior.sheet.clientHeight + 0.5);

await act(async () => {
  pressKey(behavior.host, 'Home');
});
assert.equal(selectedRow(behavior.host)?.getAttribute('data-track-index'), '0');
assert.equal(behavior.sheet.scrollTop, 0);

await act(async () => {
  behavior.sheet.scrollTop = 1600;
  behavior.sheet.dispatchEvent(new window.Event('scroll'));
});
const scrolledPaths = mountedPaths(behavior.host);
assert.ok(scrolledPaths.includes('/tmp/odqa-synthetic/track-45.wav'));
assert.ok(!scrolledPaths.includes('/tmp/odqa-synthetic/track-10.wav'));
assert.ok(scrolledPaths.includes('/tmp/odqa-synthetic/track-0.wav'));
assert.equal(behavior.sheet.scrollTop, 1600);

const beforeMetadata = behavior.sheet.scrollTop;
const focusedKey = document.activeElement?.getAttribute('data-browse-key');
await act(async () => {
  globalThis.__LIBRARY_WINDOW_SET_TRACKS(behaviorTracks.map((item) => ({ ...item, bpm: 128.4 })));
});
assert.equal(behavior.sheet.scrollTop, beforeMetadata);
assert.equal(document.activeElement?.getAttribute('data-browse-key'), focusedKey);
assert.match(selectedRow(behavior.host)?.getAttribute('aria-label') ?? '', /128\.40 BPM/);

await act(async () => {
  behavior.host.querySelector('button[aria-label^="Title"]').dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
});
assert.equal(behavior.sheet.scrollTop, beforeMetadata);
assert.equal(selectedRow(behavior.host)?.getAttribute('data-track-path'), '/tmp/odqa-synthetic/track-0.wav');
assert.notEqual(behavior.host.querySelector('[data-track-index="0"]')?.getAttribute('data-track-path'), '/tmp/odqa-synthetic/track-0.wav');
assert.equal(document.querySelector('.track-inspector h3')?.textContent, 'Zzz-last');
assert.ok(behavior.host.querySelector('button') && [...behavior.host.querySelectorAll('button')].some((button) => button.textContent === 'Load A'));

const search = behavior.host.querySelector('input[aria-label="Search library"]');
const applySearch = (value) => {
  const setter = Object.getOwnPropertyDescriptor(dom.HTMLInputElement.prototype, 'value').set;
  search._valueTracker?.setValue(search.value);
  setter.call(search, value);
  searchHandler(behavior.host, search)({ target: search, currentTarget: search });
};
await act(async () => {
  applySearch('Aaa-first');
});
assert.deepEqual(mountedPaths(behavior.host), ['/tmp/odqa-synthetic/track-199.wav']);
await act(async () => {
  applySearch('');
});
assert.equal(selectedRow(behavior.host)?.getAttribute('data-track-path'), '/tmp/odqa-synthetic/track-0.wav');
assert.ok(mountedPaths(behavior.host).length < 40);
assert.ok(behavior.sheet.scrollTop < LIBRARY_HEADER_HEIGHT + 199 * LIBRARY_ROW_HEIGHT);

behavior.sheet.setViewportHeight(viewportHeight + LIBRARY_ROW_HEIGHT * 4);
await act(async () => {
  behavior.sheet.dispatchEvent(new window.Event('scroll'));
});
const expanded = mountedPaths(behavior.host).length;
behavior.sheet.setViewportHeight(viewportHeight);
await act(async () => {
  behavior.sheet.dispatchEvent(new window.Event('scroll'));
});
assert.ok(expanded > mountedPaths(behavior.host).length);

const menuRow = selectedRow(behavior.host);
await act(async () => {
  menuRow.dispatchEvent(new window.MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 12, clientY: 12 }));
});
assert.equal(document.querySelector('[role="menu"] [role="menuitem"]')?.textContent?.includes('Remove from library') || [...document.querySelectorAll('[role="menuitem"]')].some((item) => item.textContent?.includes('Remove from library')), true);
await act(async () => {
  document.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
});
assert.equal(document.querySelector('[role="menu"]'), null);
assert.equal(menuRow.getAttribute('draggable'), 'true');

await act(async () => {
  behavior.root.unmount();
});
behavior.host.remove();

const measure = async (count) => {
  const commits = [];
  const tracks = Array.from({ length: count }, (_, index) => track(index, index === 11 ? 'ODQA-11-indigo' : undefined));
  const started = performance.now();
  const view = await renderLibrary(tracks, commits);
  const mountMs = performance.now() - started;
  const mountedAtStart = mountedPaths(view.host).length;
  const afterMount = commits.length;
  for (let frame = 1; frame <= 50; frame += 1) {
    await act(async () => {
      view.controls.setFrame(frame);
    });
  }
  const afterTicks = commits.length;
  const searchBox = view.host.querySelector('input[aria-label="Search library"]');
  const setter = Object.getOwnPropertyDescriptor(dom.HTMLInputElement.prototype, 'value').set;
  const change = searchHandler(view.host, searchBox);
  const apply = (value) => {
    searchBox._valueTracker?.setValue(searchBox.value);
    setter.call(searchBox, value);
    change({ target: searchBox, currentTarget: searchBox });
  };
  const commitSince = () => commits.slice(mark).reduce((sum, value) => sum + value, 0);
  let mark = commits.length;
  const searchStarted = performance.now();
  await act(async () => apply('ODQA-11-indigo'));
  const searchMs = performance.now() - searchStarted;
  const searchCommitMs = commitSince();
  const filtered = mountedPaths(view.host).length;
  mark = commits.length;
  const clearStarted = performance.now();
  await act(async () => apply(''));
  const clearMs = performance.now() - clearStarted;
  const clearCommitMs = commitSince();
  mark = commits.length;
  const sortStarted = performance.now();
  await act(async () => {
    view.host.querySelector('button[aria-label^="Title"]').dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  });
  const sortMs = performance.now() - sortStarted;
  const sortCommitMs = commitSince();
  const rows = [...view.host.querySelectorAll('[data-track-path]')];
  const target = rows[Math.min(4, rows.length - 1)];
  mark = commits.length;
  const selectStarted = performance.now();
  await act(async () => {
    target.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  });
  const selectMs = performance.now() - selectStarted;
  const selectCommitMs = commitSince();
  const result = {
    count,
    mountMs: Number(mountMs.toFixed(1)),
    mountCommitMs: Number(commits.slice(0, afterMount).reduce((sum, value) => sum + value, 0).toFixed(1)),
    mountedRows: mountedAtStart,
    tickCommitMs: Number(commits.slice(afterMount, afterTicks).reduce((sum, value) => sum + value, 0).toFixed(1)),
    searchMs: Number(searchMs.toFixed(1)),
    searchCommitMs: Number(searchCommitMs.toFixed(1)),
    filteredRows: filtered,
    clearMs: Number(clearMs.toFixed(1)),
    clearCommitMs: Number(clearCommitMs.toFixed(1)),
    sortMs: Number(sortMs.toFixed(1)),
    sortCommitMs: Number(sortCommitMs.toFixed(1)),
    selectMs: Number(selectMs.toFixed(1)),
    selectCommitMs: Number(selectCommitMs.toFixed(1)),
    selectedPath: selectedRow(view.host)?.getAttribute('data-track-path') ?? null,
    clickedPath: target.getAttribute('data-track-path'),
  };
  await act(async () => {
    view.root.unmount();
  });
  view.host.remove();
  return result;
};

const results = [];
for (const count of [500, 2000]) results.push(await measure(count));
for (const result of results) {
  assert.ok(result.mountedRows < 40, `${result.count} mounted ${result.mountedRows}`);
  assert.equal(result.filteredRows, 1);
  assert.equal(result.clickedPath, result.selectedPath);
  assert.equal(result.tickCommitMs, 0);
}
console.log(JSON.stringify({
  label: 'synthetic React commits on the real Library component after visible-row windowing; not native app latency or layout time',
  run44Comparison: {
    note: 'run44 mounted every row. Those commit times were happy-dom React time, not native latency.',
    rows500: { mountCommitMs: 140.9, searchCommitMs: 28.7, clearCommitMs: 69.0, sortCommitMs: 14.5, selectCommitMs: 14.0 },
    rows2000: { mountCommitMs: 373.8, searchCommitMs: 102.5, clearCommitMs: 249.8, sortCommitMs: 52.3, selectCommitMs: 46.3 },
  },
  results,
}, null, 2));
await server.close();
