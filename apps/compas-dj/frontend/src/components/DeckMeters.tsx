import { memo, useCallback, useEffect, useRef, useState } from "react";
import {
  readDeckMeters,
  spectrumFloorDb,
  spectrumPlotX,
  spectrumPlotY,
  spectrumThresholds,
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

function paintSpectrum(
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  frame: DeckMeterFrame | null,
) {
  const low = frame?.minHz ?? 20;
  const high = frame?.maxHz ?? 20000;
  const top = 13;
  const bottom = h - 2;
  const axis = high > low && low > 0;
  ctx.lineWidth = 1;
  ctx.font = "9px system-ui";
  ctx.strokeStyle = "rgba(255,255,255,0.22)";
  ctx.beginPath();
  if (axis) {
    for (const marker of spectrumThresholds) {
      if (marker.hz <= low || marker.hz >= high) continue;
      const x = spectrumPlotX(marker.hz, low, high, w);
      ctx.moveTo(x, 0);
      ctx.lineTo(x, h);
    }
  }
  ctx.stroke();
  ctx.fillStyle = "#99a7b3";
  ctx.textAlign = "left";
  if (axis) {
    for (const marker of spectrumThresholds) {
      if (marker.hz <= low || marker.hz >= high) continue;
      const x = spectrumPlotX(marker.hz, low, high, w);
      const labelWidth = ctx.measureText(marker.label).width;
      const toTheRight = x + 4;
      const labelX =
        toTheRight + labelWidth <= w - 2 ? toTheRight : Math.max(2, x - labelWidth - 3);
      ctx.fillText(marker.label, labelX, 11);
    }
  }
  const spectrum = frame?.spectrum ?? [];
  const spectrumHz = frame?.spectrumHz ?? [];
  if (spectrum.length < 2 || spectrumHz.length !== spectrum.length || !axis) return;
  ctx.strokeStyle = "#b2c8db";
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
    ctx.textAlign = "left";
  }
}

function paint(
  canvas: HTMLCanvasElement,
  kind: number,
  frame: DeckMeterFrame | null,
) {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  const w = canvas.clientWidth,
    h = canvas.clientHeight;
  if (!w || !h) return;
  const dpr = Math.min(2, window.devicePixelRatio || 1);
  const width = Math.round(w * dpr),
    height = Math.round(h * dpr);
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
  }
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  if (kind === 4) {
    paintSpectrum(ctx, w, h, frame);
    return;
  }
  const top = 13,
    bottom = h - 2,
    mid = (top + bottom) / 2,
    radius = (bottom - top) / 2;
  ctx.lineWidth = 1;
  ctx.strokeStyle = "#30353c";
  ctx.beginPath();
  ctx.moveTo(0, mid);
  ctx.lineTo(w, mid);
  if (kind === 1) {
    ctx.moveTo(w / 2, top);
    ctx.lineTo(w / 2, bottom);
  }
  ctx.stroke();
  if (!frame) return;
  ctx.strokeStyle = kind === 1 ? "#f5cc6d" : "#b2c8db";
  ctx.beginPath();
  const n = frame.left.length;
  for (let i = 0; i < n; i++) {
    const l = frame.left[i],
      r = frame.right[i];
    let x = (i * w) / Math.max(1, n - 1),
      y;
    if (kind === 1) {
      const side = Math.min(radius, w / 2);
      x = w / 2 + ((l - r) * side) / 2;
      y = mid - ((l + r) * side) / 2;
    } else {
      y = mid - l * radius;
    }
    if (i === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  }
  ctx.stroke();
  if (kind === 2) {
    ctx.beginPath();
    ctx.strokeStyle = "rgba(110,173,218,0.65)";
    frame.right.forEach((r, i) => {
      const x = (i * w) / Math.max(1, n - 1),
        y = mid - r * radius;
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    });
    ctx.stroke();
  }
}

export const DeckMeterStrip = memo(function DeckMeterStrip({
  deck,
  mask,
  active,
  track,
}: {
  deck: number;
  mask: number;
  active: boolean;
  track: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const root = ref.current;
    if (!root) return;
    const canvases = Array.from(root.querySelectorAll("canvas"));
    const draw = (frame: DeckMeterFrame | null) =>
      canvases.forEach((c) => paint(c, Number(c.dataset.bit), frame));
    let disposed = false,
      visible = false,
      busy = false,
      timer = 0,
      generation = 0;
    const enabled = () => !disposed && visible && active && !document.hidden;
    const poll = async () => {
      if (!enabled() || busy) return;
      busy = true;
      const id = generation;
      try {
        const frame = await readDeckMeters(deck, !!(mask & 4));
        if (enabled() && generation === id && frame) draw(frame);
      } catch {
        if (enabled()) draw(null);
      } finally {
        busy = false;
        if (enabled()) timer = window.setTimeout(poll, 65);
      }
    };
    const update = () => {
      generation++;
      window.clearTimeout(timer);
      draw(null);
      if (enabled()) void poll();
    };
    const observer = new IntersectionObserver(([entry]) => {
      visible = entry.isIntersecting;
      update();
    });
    observer.observe(root);
    document.addEventListener("visibilitychange", update);
    const resize = new ResizeObserver(() => draw(null));
    resize.observe(root);
    draw(null);
    return () => {
      disposed = true;
      window.clearTimeout(timer);
      observer.disconnect();
      resize.disconnect();
      document.removeEventListener("visibilitychange", update);
    };
  }, [deck, mask, active, track]);
  return (
    <div className={`deck-meter-strip${mask & 4 ? " deck-meter-strip--spectrum" : ""}`} ref={ref}>
      {widgets
        .filter((w) => mask & w.bit)
        .map((w) => (
          <div className="deck-meter-panel" key={w.bit}>
            <span>{w.name}</span>
            <canvas aria-label={w.name} data-bit={w.bit} role="img" />
          </div>
        ))}
    </div>
  );
});
