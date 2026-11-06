import { memo, useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import {
  onMeterFrame,
  readDeckMeters,
  spectrumFloorDb,
  spectrumLabelPlacements,
  resolvedDeckColor,
  spectrumPlotX,
  spectrumPlotY,
  spectrumThresholds,
  stereoImagePoint,
  type DeckMeterFrame,
} from "../lib/deckMeters";
import "./deckMeters.css";

const widgets = [
  { bit: 1, name: "Stereo image" },
  { bit: 2, name: "Oscilloscope" },
  { bit: 4, name: "Spectrum" },
] as const;

const key = (deck: number) => `opendeck.deck.${deck}.meter-mask.v1`;

export function useMeterSelection(deck: number) {
  const [masks, setMasks] = useState(() =>
    Array.from({ length: 4 }, (_, i) => {
      try {
        return Number(localStorage.getItem(key(i))) & 7;
      } catch {
        return 0;
      }
    }),
  );
  const toggle = useCallback(
    (bit: number) => {
      const next = masks[deck] ^ bit;
      setMasks(masks.map((value, i) => (i === deck ? next : value)));
      try {
        localStorage.setItem(key(deck), String(next));
      } catch {
        /* Session preference. */
      }
    },
    [deck, masks],
  );
  return { mask: masks[deck], toggle };
}

export const MeterMenu = memo(function MeterMenu({
  deck,
  mask,
  toggle,
}: {
  deck: number;
  mask: number;
  toggle: (bit: number) => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (!open) return;
    const close = (e: PointerEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    const escape = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpen(false);
        ref.current?.querySelector("button")?.focus();
      }
    };
    document.addEventListener("pointerdown", close);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("pointerdown", close);
      document.removeEventListener("keydown", escape);
    };
  }, [open]);
  return (
    <div className="deck-meter-menu" ref={ref}>
      <button
        type="button"
        className="deck-load"
        aria-label={`Deck ${"ABCD"[deck]} live meters`}
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        Meters
      </button>
      {open && (
        <div
          className="deck-meter-options"
          role="group"
          aria-label="Live meters"
        >
          {widgets.map((w) => (
            <label key={w.bit}>
              <input
                type="checkbox"
                checked={!!(mask & w.bit)}
                onChange={() => toggle(w.bit)}
              />
              {w.name}
            </label>
          ))}
        </div>
      )}
    </div>
  );
});

type MeterPanel = {
  kind: number;
  trace: HTMLCanvasElement;
  grid: HTMLCanvasElement;
  traceCtx: CanvasRenderingContext2D;
  gridCtx: CanvasRenderingContext2D;
  cssW: number;
  cssH: number;
  dpr: number;
  gridKey: string;
};

function displayScale() {
  return Math.min(2, window.devicePixelRatio || 1);
}

function resizePanel(panel: MeterPanel) {
  const host = panel.trace.parentElement;
  const cssW = host instanceof HTMLElement ? host.clientWidth : 0;
  const cssH = host instanceof HTMLElement ? host.clientHeight : 0;
  const dpr = displayScale();
  if (!cssW || !cssH) return false;
  const width = Math.max(1, Math.round(cssW * dpr));
  const height = Math.max(1, Math.round(cssH * dpr));
  const changed =
    panel.cssW !== cssW ||
    panel.cssH !== cssH ||
    panel.dpr !== dpr ||
    panel.trace.width !== width ||
    panel.trace.height !== height ||
    panel.grid.width !== width ||
    panel.grid.height !== height;
  panel.cssW = cssW;
  panel.cssH = cssH;
  panel.dpr = dpr;
  if (panel.trace.width !== width || panel.trace.height !== height) {
    panel.trace.width = width;
    panel.trace.height = height;
  }
  if (panel.grid.width !== width || panel.grid.height !== height) {
    panel.grid.width = width;
    panel.grid.height = height;
  }
  if (changed) panel.gridKey = "";
  return changed;
}

function prepare(ctx: CanvasRenderingContext2D, canvas: HTMLCanvasElement, panel: MeterPanel) {
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  ctx.setTransform(panel.dpr, 0, 0, panel.dpr, 0, 0);
  ctx.lineWidth = 1;
  ctx.font = "9px system-ui";
}

function drawStatic(panel: MeterPanel, frame: DeckMeterFrame | null) {
  const { cssW: w, cssH: h, kind } = panel;
  const low = frame?.minHz ?? 20;
  const high = frame?.maxHz ?? 20000;
  const key = `${kind}:${w}:${h}:${panel.dpr}:${low}:${high}`;
  if (panel.gridKey === key) return;
  panel.gridKey = key;
  const ctx = panel.gridCtx;
  prepare(ctx, panel.grid, panel);
  if (kind === 4) {
    const axis = high > low && low > 0;
    ctx.strokeStyle = "rgba(255,255,255,0.22)";
    ctx.beginPath();
    ctx.fillStyle = "#99a7b3";
    ctx.textAlign = "left";
    if (axis) {
      for (const marker of spectrumThresholds) {
        if (marker.hz <= low || marker.hz >= high) continue;
        const x = spectrumPlotX(marker.hz, low, high, w);
        ctx.moveTo(x, 0);
        ctx.lineTo(x, h);
      }
    }
    ctx.stroke();
    if (axis) {
      const titleWidth = ctx.measureText("Spectrum").width;
      const placements = spectrumLabelPlacements(
        spectrumThresholds,
        low,
        high,
        w,
        (label) => ctx.measureText(label).width,
        [{ left: 0, right: 16 + titleWidth }],
      );
      for (const placement of placements) {
        ctx.fillText(placement.label, placement.x, 11);
      }
    }
    return;
  }
  const top = 13;
  const bottom = h - 2;
  const mid = (top + bottom) / 2;
  ctx.strokeStyle = "#30353c";
  ctx.beginPath();
  ctx.moveTo(0, mid);
  ctx.lineTo(w, mid);
  if (kind === 1) {
    ctx.moveTo(w / 2, top);
    ctx.lineTo(w / 2, bottom);
  }
  ctx.stroke();
}

function drawTrace(panel: MeterPanel, frame: DeckMeterFrame | null, traceColor: string) {
  const { cssW: w, cssH: h, kind } = panel;
  const ctx = panel.traceCtx;
  prepare(ctx, panel.trace, panel);
  if (!frame || !w || !h) return;
  const top = 13;
  const bottom = h - 2;
  if (kind === 4) {
    const low = frame.minHz;
    const high = frame.maxHz;
    const spectrum = frame.spectrum;
    const spectrumHz = frame.spectrumHz;
    if (
      !(high > low && low > 0) ||
      spectrum.length < 2 ||
      spectrumHz.length !== spectrum.length
    ) {
      return;
    }
    ctx.strokeStyle = traceColor;
    ctx.beginPath();
    let started = false;
    for (let i = 0; i < spectrum.length; i++) {
      const hz = spectrumHz[i];
      const db = spectrum[i];
      if (!(hz > 0) || !Number.isFinite(db)) continue;
      const x = spectrumPlotX(hz, low, high, w);
      const y = spectrumPlotY(db, top, bottom);
      if (!started) {
        ctx.moveTo(x, y);
        started = true;
      } else ctx.lineTo(x, y);
    }
    ctx.stroke();
    let peak = 0;
    for (let i = 1; i < spectrum.length; i++) {
      if (spectrum[i] > spectrum[peak]) peak = i;
    }
    if (w >= 170 && spectrum[peak] > spectrumFloorDb) {
      ctx.fillStyle = "#e7eef4";
      ctx.textAlign = "right";
      ctx.fillText(
        `${spectrumHz[peak].toFixed(1)} Hz  ${spectrum[peak].toFixed(1)} dBFS`,
        w - 5,
        h - 4,
      );
    }
    return;
  }
  const mid = (top + bottom) / 2;
  const radius = (bottom - top) / 2;
  const n = frame.left.length;
  const last = Math.max(1, n - 1);
  ctx.strokeStyle = traceColor;
  ctx.beginPath();
  for (let i = 0; i < n; i++) {
    const left = frame.left[i];
    const right = frame.right[i];
    let x = (i * w) / last;
    let y: number;
    if (kind === 1) {
      const point = stereoImagePoint(left, right, w / 2, mid, w / 2, radius);
      x = point.x;
      y = point.y;
    } else {
      y = mid - left * radius;
    }
    if (i === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  }
  ctx.stroke();
  if (kind === 2) {
    ctx.beginPath();
    ctx.strokeStyle = traceColor;
    for (let i = 0; i < frame.right.length; i++) {
      const x = (i * w) / last;
      const y = mid - frame.right[i] * radius;
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.stroke();
  }
}

function paintPanels(panels: MeterPanel[], frame: DeckMeterFrame | null, traceColor: string) {
  for (const panel of panels) {
    if (!panel.cssW || !panel.cssH) continue;
    drawStatic(panel, frame);
    drawTrace(panel, frame, traceColor);
  }
}

export const DeckMeterStrip = memo(function DeckMeterStrip({
  deck,
  mask,
  active,
  track,
  color,
}: {
  deck: number;
  mask: number;
  active: boolean;
  track: string;
  color: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const root = ref.current;
    if (!root) return;
    const panels: MeterPanel[] = [];
    for (const trace of root.querySelectorAll<HTMLCanvasElement>("canvas.meter-trace")) {
      const grid = trace.previousElementSibling;
      if (!(grid instanceof HTMLCanvasElement)) continue;
      const traceCtx = trace.getContext("2d");
      const gridCtx = grid.getContext("2d");
      if (!traceCtx || !gridCtx) continue;
      panels.push({
        kind: Number(trace.dataset.bit),
        trace,
        grid,
        traceCtx,
        gridCtx,
        cssW: 0,
        cssH: 0,
        dpr: 1,
        gridKey: "",
      });
    }
    let disposed = false;
    let visible = false;
    let busy = false;
    let listening = false;
    let unsubscribe = () => {};
    let latest: DeckMeterFrame | null = null;
    let painted = 0;
    const traceColor = resolvedDeckColor(color, (name) =>
      getComputedStyle(document.documentElement).getPropertyValue(name),
    );
    const enabled = () => !disposed && visible && active && !document.hidden;
    const repaint = () => paintPanels(panels, latest, traceColor);
    const fit = () => {
      let changed = false;
      for (const panel of panels) {
        if (resizePanel(panel)) changed = true;
      }
      const unpainted = panels.some((panel) => panel.cssW > 0 && panel.gridKey === "");
      if (!changed && !unpainted) return;
      repaint();
      if (latest) painted = latest.sequence;
    };
    const tick = () => {
      if (!enabled()) {
        stop();
        return;
      }
      if (panels.some((panel) => panel.cssW === 0 || panel.dpr !== displayScale())) fit();
      if (latest && latest.sequence !== painted) {
        repaint();
        painted = latest.sequence;
      }
      if (!busy) {
        busy = true;
        const since = latest?.sequence ?? 0;
        void readDeckMeters(deck, (mask & 4) !== 0, since)
          .then((frame) => {
            busy = false;
            if (!enabled()) return;
            if (frame && frame.sequence !== painted) latest = frame;
          })
          .catch(() => {
            busy = false;
          });
      }
    };
    const stop = () => {
      if (!listening) return;
      listening = false;
      unsubscribe();
      unsubscribe = () => {};
    };
    const start = () => {
      if (listening || !enabled()) return;
      listening = true;
      unsubscribe = onMeterFrame(tick);
    };
    const onVisibility = () => {
      if (enabled()) start();
      else stop();
    };
    const observer = new IntersectionObserver(([entry]) => {
      visible = entry.isIntersecting;
      onVisibility();
    });
    observer.observe(root);
    document.addEventListener("visibilitychange", onVisibility);
    const resize = new ResizeObserver(() => fit());
    resize.observe(root);
    for (const panel of panels) {
      const host = panel.trace.parentElement;
      if (host) resize.observe(host);
    }
    window.addEventListener("resize", fit);
    let scaleMedia = window.matchMedia(`(resolution: ${window.devicePixelRatio || 1}dppx)`);
    const onScale = () => {
      scaleMedia.removeEventListener("change", onScale);
      fit();
      scaleMedia = window.matchMedia(`(resolution: ${window.devicePixelRatio || 1}dppx)`);
      scaleMedia.addEventListener("change", onScale);
    };
    scaleMedia.addEventListener("change", onScale);
    fit();
    const retry = panels.every((panel) => panel.cssW > 0)
      ? 0
      : requestAnimationFrame(() => fit());
    start();
    return () => {
      disposed = true;
      stop();
      if (retry) cancelAnimationFrame(retry);
      observer.disconnect();
      resize.disconnect();
      window.removeEventListener("resize", fit);
      scaleMedia.removeEventListener("change", onScale);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [deck, mask, active, track, color]);
  return (
    <div className="deck-meter-strip" ref={ref}>
      {widgets
        .filter((w) => mask & w.bit)
        .map((w) => (
          <div className="deck-meter-panel" key={w.bit}>
            <canvas className="meter-grid" aria-hidden="true" />
            <canvas
              className="meter-trace"
              aria-label={w.name}
              data-bit={w.bit}
              role="img"
            />
            <span>{w.name}</span>
          </div>
        ))}
    </div>
  );
});
