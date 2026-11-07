import assert from 'node:assert/strict';
import { Window } from 'happy-dom';
import { createElement, useState, act } from 'react';
import { createRoot } from 'react-dom/client';
import { createServer } from 'vite';

const dom = new Window({ url: 'http://localhost/' });
for (const name of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Document', 'MutationObserver', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle']) {
  try {
    if (dom[name] !== undefined) globalThis[name] = dom[name];
  } catch {
    /* Node owns a read-only navigator. */
  }
}
globalThis.window = dom;
globalThis.document = dom.document;
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
window.__TAURI_INTERNALS__ = {};

const observers = [];
class RecordingObserver {
  constructor(callback) {
    this.callback = callback;
    observers.push(this);
  }
  observe() {}
  disconnect() {}
  unobserve() {}
  takeRecords() { return []; }
  intersect(isIntersecting) {
    this.callback([{ isIntersecting }]);
  }
}
globalThis.IntersectionObserver = RecordingObserver;

const pending = [];
globalThis.__VISUAL_INVOKE = (command, args) => new Promise((resolve, reject) => {
  pending.push({ command, path: args?.path, resolve, reject });
});

const server = await createServer({
  root: new URL('..', import.meta.url).pathname,
  configFile: false,
  logLevel: 'error',
  server: { middlewareMode: true, hmr: false },
  ssr: { noExternal: ['@tauri-apps/api'] },
  appType: 'custom',
  plugins: [{
    name: 'visual-invoke',
    enforce: 'pre',
    resolveId(id) {
      if (id === '@tauri-apps/api/core' || String(id).includes('@tauri-apps/api/core')) return '\0visual-invoke';
      return null;
    },
    load(id) {
      if (id !== '\0visual-invoke') return null;
      return 'export function invoke(command, args) { return globalThis.__VISUAL_INVOKE(command, args); }';
    },
  }],
});
const visuals = await server.ssrLoadModule('/src/components/LibraryVisuals.tsx');
const { TrackCells, useTrackVisuals, VISUAL_ATTEMPT_LIMIT, visualRetryDelayMs } = visuals;

const busyMessage = 'Track visuals are busy; retry shortly';
const decodeError = 'Track contains no decoded audio';
let serial = 0;
const trackPath = (name) => `/tmp/odqa-visual/${name}-${serial += 1}.wav`;

const artworkFor = (label) => ({
  peaks: [0.2, 0.4],
  band_peaks: [[0.2, 0.1, 0.3]],
  artwork_data_url: `data:image/png;base64,${label}`,
  title: label,
  artist: 'Fixture',
  album: null,
});

const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const nextCall = async (timeout = 1000) => {
  const started = Date.now();
  while (!pending.length) {
    if (Date.now() - started > timeout) throw new Error('timed out waiting for a visuals request');
    await delay(5);
  }
  return pending.shift();
};

const settle = async () => {
  await act(async () => {
    await delay(0);
  });
};

function Probe({ path, enabled = true, marker = 'probe' }) {
  const visual = useTrackVisuals(path, enabled, true, 0);
  return createElement('output', {
    'data-marker': marker,
    'data-status': visual.status,
    'data-art': visual.data?.artwork_data_url ?? '',
    'data-title': visual.data?.title ?? '',
  });
}

const mount = async (node) => {
  const host = document.createElement('div');
  host.className = 'tl-sheet';
  document.body.appendChild(host);
  const root = createRoot(host);
  await act(async () => {
    root.render(node);
  });
  return {
    host,
    root,
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

const readProbe = (host, marker = 'probe') => host.querySelector(`[data-marker="${marker}"]`);

assert.equal(visualRetryDelayMs(1), 40);
assert.equal(visualRetryDelayMs(2), 80);
assert.equal(visualRetryDelayMs(3), 160);
assert.equal(visualRetryDelayMs(20), 800);
assert.equal(VISUAL_ATTEMPT_LIMIT, 4);

{
  const path = trackPath('busy-then-ready');
  const view = await mount(createElement(Probe, { path }));
  assert.equal(readProbe(view.host).getAttribute('data-status'), 'loading');
  const first = await nextCall();
  assert.equal(first.command, 'track_visuals');
  assert.equal(first.path, path);
  await act(async () => {
    first.reject(busyMessage);
  });
  assert.equal(readProbe(view.host).getAttribute('data-status'), 'loading');
  const second = await nextCall(visualRetryDelayMs(1) + 500);
  assert.equal(second.path, path);
  await act(async () => {
    second.resolve(artworkFor('Recovered'));
  });
  await settle();
  assert.equal(readProbe(view.host).getAttribute('data-status'), 'ready');
  assert.equal(readProbe(view.host).getAttribute('data-title'), 'Recovered');
  const again = await mount(createElement(Probe, { path, marker: 'cached' }));
  await settle();
  assert.equal(pending.length, 0);
  assert.equal(readProbe(again.host, 'cached').getAttribute('data-art'), 'data:image/png;base64,Recovered');
  await view.unmount();
  await again.unmount();
}

{
  const path = trackPath('decode-error');
  const view = await mount(createElement(Probe, { path }));
  const call = await nextCall();
  await act(async () => {
    call.reject(decodeError);
  });
  await settle();
  assert.equal(readProbe(view.host).getAttribute('data-status'), 'unavailable');
  await view.unmount();
  const again = await mount(createElement(Probe, { path }));
  await delay(visualRetryDelayMs(1) + 30);
  assert.equal(pending.length, 0);
  assert.equal(readProbe(again.host).getAttribute('data-status'), 'unavailable');
  await again.unmount();
}

{
  const path = trackPath('budget');
  const busyRejections = [
    busyMessage,
    new Error(busyMessage),
    { message: busyMessage },
    busyMessage,
  ];
  const view = await mount(createElement(Probe, { path }));
  for (let attempt = 0; attempt < VISUAL_ATTEMPT_LIMIT; attempt += 1) {
    const call = await nextCall(1000);
    assert.equal(call.path, path);
    await act(async () => {
      call.reject(busyRejections[attempt]);
    });
  }
  await settle();
  assert.equal(readProbe(view.host).getAttribute('data-status'), 'unavailable');
  await delay(visualRetryDelayMs(VISUAL_ATTEMPT_LIMIT) + 40);
  assert.equal(pending.length, 0);
  await view.unmount();
  const retry = await mount(createElement(Probe, { path }));
  const call = await nextCall();
  assert.equal(call.path, path);
  await act(async () => {
    call.resolve(artworkFor('AfterBudget'));
  });
  await settle();
  assert.equal(readProbe(retry.host).getAttribute('data-status'), 'ready');
  await retry.unmount();
}

{
  const path = trackPath('cancel-timer');
  const view = await mount(createElement(Probe, { path }));
  const call = await nextCall();
  await act(async () => {
    call.reject(busyMessage);
  });
  await view.unmount();
  await delay(visualRetryDelayMs(1) + visualRetryDelayMs(2) + 40);
  assert.equal(pending.length, 0);
}

{
  const path = trackPath('cancel-inflight-ready');
  const view = await mount(createElement(Probe, { path }));
  const call = await nextCall();
  await view.unmount();
  await act(async () => {
    call.resolve(artworkFor('CachedAfterLeave'));
  });
  const again = await mount(createElement(Probe, { path }));
  await settle();
  assert.equal(pending.length, 0);
  assert.equal(readProbe(again.host).getAttribute('data-title'), 'CachedAfterLeave');
  await again.unmount();
}

{
  const path = trackPath('cancel-inflight-busy');
  const view = await mount(createElement(Probe, { path }));
  const call = await nextCall();
  await view.unmount();
  await act(async () => {
    call.reject(busyMessage);
  });
  await delay(visualRetryDelayMs(1) + 40);
  assert.equal(pending.length, 0);
  const again = await mount(createElement(Probe, { path }));
  const retry = await nextCall();
  assert.equal(retry.path, path);
  await act(async () => {
    retry.resolve(artworkFor('Remounted'));
  });
  await settle();
  assert.equal(readProbe(again.host).getAttribute('data-status'), 'ready');
  await again.unmount();
}

{
  const left = trackPath('queue-left');
  const right = trackPath('queue-right');
  const view = await mount(createElement(
    'div',
    null,
    createElement(Probe, { path: left, marker: 'left' }),
    createElement(Probe, { path: right, marker: 'right' }),
  ));
  const first = await nextCall();
  assert.equal(first.path, left);
  assert.equal(pending.length, 0);
  await act(async () => {
    first.reject(busyMessage);
  });
  const second = await nextCall();
  assert.equal(second.path, right);
  assert.equal(pending.filter((call) => call.path === left).length, 0);
  await act(async () => {
    second.resolve(artworkFor('RightReady'));
  });
  await settle();
  assert.equal(readProbe(view.host, 'right').getAttribute('data-title'), 'RightReady');
  assert.equal(readProbe(view.host, 'left').getAttribute('data-status'), 'loading');
  const retry = await nextCall(visualRetryDelayMs(1) + 500);
  assert.equal(retry.path, left);
  await act(async () => {
    retry.resolve(artworkFor('LeftReady'));
  });
  await settle();
  assert.equal(readProbe(view.host, 'left').getAttribute('data-title'), 'LeftReady');
  await view.unmount();
}

{
  const path = trackPath('duplicate');
  const view = await mount(createElement(
    'div',
    null,
    createElement(Probe, { path, marker: 'one', enabled: true }),
    createElement(Probe, { path, marker: 'two', enabled: true }),
  ));
  const call = await nextCall();
  assert.equal(call.path, path);
  await delay(30);
  assert.equal(pending.length, 0);
  await act(async () => {
    call.resolve(artworkFor('Shared'));
  });
  await settle();
  assert.equal(readProbe(view.host, 'one').getAttribute('data-art'), 'data:image/png;base64,Shared');
  assert.equal(readProbe(view.host, 'two').getAttribute('data-art'), 'data:image/png;base64,Shared');
  await view.unmount();
}

{
  const hidden = trackPath('offscreen');
  const shown = trackPath('shown');
  const view = await mount(createElement(TrackCells, { path: hidden, title: 'Hidden title', artist: 'Fixture' }));
  await delay(20);
  assert.equal(pending.length, 0);
  assert.equal(view.host.querySelector('.tl-title-text')?.textContent, 'Hidden title');
  await act(async () => {
    observers.at(-1).intersect(true);
  });
  const hiddenCall = await nextCall();
  assert.equal(hiddenCall.path, hidden);
  await view.render(createElement(TrackCells, { path: shown, title: 'Shown title', artist: 'Fixture' }));
  await act(async () => {
    hiddenCall.resolve(artworkFor('StaleHidden'));
  });
  await settle();
  assert.equal(view.host.querySelector('img'), null);
  assert.equal(view.host.querySelector('.tl-title-text')?.textContent, 'Shown title');
  const shownCall = await nextCall();
  assert.equal(shownCall.path, shown);
  await act(async () => {
    shownCall.resolve(artworkFor('ShownArt'));
  });
  await settle();
  assert.equal(view.host.querySelector('img')?.getAttribute('src'), 'data:image/png;base64,ShownArt');
  assert.equal(view.host.querySelector('.tl-title-text')?.textContent, 'ShownArt');
  await view.unmount();
}

{
  const path = trackPath('off-then-on');
  function Toggle() {
    const [enabled, setEnabled] = useState(false);
    globalThis.__VISUAL_ENABLE = setEnabled;
    return createElement(Probe, { path, enabled });
  }
  const view = await mount(createElement(Toggle));
  await delay(20);
  assert.equal(pending.length, 0);
  await act(async () => {
    globalThis.__VISUAL_ENABLE(true);
  });
  const call = await nextCall();
  assert.equal(call.path, path);
  await act(async () => {
    call.reject(busyMessage);
    globalThis.__VISUAL_ENABLE(false);
  });
  await delay(visualRetryDelayMs(1) + 40);
  assert.equal(pending.length, 0);
  await act(async () => {
    globalThis.__VISUAL_ENABLE(true);
  });
  const retry = await nextCall();
  assert.equal(retry.path, path);
  await act(async () => {
    retry.resolve(artworkFor('VisibleAgain'));
  });
  await settle();
  assert.equal(readProbe(view.host).getAttribute('data-title'), 'VisibleAgain');
  await view.unmount();
}

console.log('Library visuals: busy retry, genuine failure, cancellation, deduplicated queue, and row reuse passed');
await server.close();
