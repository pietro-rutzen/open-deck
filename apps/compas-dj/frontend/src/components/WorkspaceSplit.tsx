import { useEffect, useRef, useState, type ReactNode } from "react";

const clampSize = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

export function useStoredSize(key: string, fallback: number, min: number, max: number) {
  const [size, setSize] = useState(() => {
    try {
      const saved = Number(localStorage.getItem(key));
      return Number.isFinite(saved) ? clampSize(saved, min, max) : fallback;
    } catch {
      return fallback;
    }
  });
  useEffect(() => {
    try { localStorage.setItem(key, String(size)); } catch { /* Preference is optional. */ }
  }, [key, size]);
  return [size, setSize] as const;
}

/** Drag the edge of a fixed-width column. `grow` is 1 when dragging right makes it wider. */
export function PaneDivider({
  label,
  value,
  min,
  max,
  fallback,
  grow,
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  fallback: number;
  grow: 1 | -1;
  onChange: (next: number) => void;
}) {
  const drag = useRef<{ x: number; width: number } | null>(null);
  const apply = (next: number) => onChange(clampSize(next, min, max));
  return (
    <div
      className="pane-divider"
      role="separator"
      tabIndex={0}
      aria-orientation="vertical"
      aria-label={label}
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={Math.round(value)}
      title={`${label} · double-click to reset`}
      onDoubleClick={() => apply(fallback)}
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        event.preventDefault();
        event.currentTarget.focus();
        event.currentTarget.setPointerCapture(event.pointerId);
        drag.current = { x: event.clientX, width: value };
      }}
      onPointerMove={(event) => {
        const start = drag.current;
        if (!start) return;
        apply(start.width + (event.clientX - start.x) * grow);
      }}
      onPointerUp={(event) => {
        drag.current = null;
        if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
      }}
      onPointerCancel={() => { drag.current = null; }}
      onLostPointerCapture={() => { drag.current = null; }}
      onKeyDown={(event) => {
        if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
        event.preventDefault();
        const step = event.key === "ArrowRight" ? 16 : -16;
        apply(event.key === "Home" ? min : event.key === "End" ? max : value + step * grow);
      }}
    ><span aria-hidden="true" /></div>
  );
}

export function MixerFrame({ children }: { children: ReactNode }) {
  const [width, setWidth] = useStoredSize("opendeck.mixer.width.v1", 268, 220, 640);
  return (
    <>
      <PaneDivider label="Resize mixer" value={width} min={220} max={640} fallback={268} grow={-1} onChange={setWidth} />
      <div className="mixer-size" style={{ width }}>{children}</div>
      <PaneDivider label="Resize mixer" value={width} min={220} max={640} fallback={268} grow={1} onChange={setWidth} />
    </>
  );
}

const STORAGE_KEY = "opendeck.workspace.split.v1";
const DEFAULT_RATIO = 0.8;
const DIVIDER = 1;
const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

/** Resize the performance layout within usable control and library bounds. */
export function WorkspaceSplit({ upper, lower }: { upper: ReactNode; lower: ReactNode }) {
  const root = useRef<HTMLDivElement>(null);
  const dragging = useRef(false);
  const [height, setHeight] = useState(0);
  const [ratio, setRatio] = useState(() => {
    try {
      const saved = localStorage.getItem(STORAGE_KEY);
      const value = saved === null ? NaN : Number(saved);
      return Number.isFinite(value) && value > 0 && value < 1 ? value : DEFAULT_RATIO;
    } catch { return DEFAULT_RATIO; }
  });
  useEffect(() => {
    const element = root.current;
    if (!element) return;
    const observer = new ResizeObserver(() => setHeight(element.clientHeight));
    observer.observe(element);
    setHeight(element.clientHeight);
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    try { localStorage.setItem(STORAGE_KEY, String(ratio)); } catch { /* Preference is optional. */ }
  }, [ratio]);

  const available = Math.max(0, height - DIVIDER);
  const min = Math.min(500, Math.max(available / 2, available - 160));
  const max = available - Math.min(160, available / 2);
  const upperHeight = clamp(available * ratio, min, max);
  const moveTo = (pixels: number) => {
    if (available > 0) setRatio(clamp(pixels, min, max) / available);
  };
  return (
    <div className="workspace-split" ref={root}
      style={{ gridTemplateRows: height ? `${upperHeight}px ${DIVIDER}px minmax(0, 1fr)` : `${DEFAULT_RATIO}fr ${DIVIDER}px ${1 - DEFAULT_RATIO}fr` }}>
      <section className="workspace-upper" aria-label="Decks and mixer">{upper}</section>
      <div className="workspace-divider" role="separator" tabIndex={0}
        aria-label="Resize decks and library" aria-orientation="horizontal"
        aria-valuemin={Math.round(min)} aria-valuemax={Math.round(max)} aria-valuenow={Math.round(upperHeight)}
        aria-valuetext={`Decks ${Math.round(upperHeight)} pixels, library ${Math.round(available - upperHeight)} pixels`}
        title="Drag to resize decks and library · double-click to reset"
        onDoubleClick={() => setRatio(DEFAULT_RATIO)}
        onPointerDown={(event) => {
          if (event.button !== 0) return;
          event.preventDefault();
          event.currentTarget.focus();
          event.currentTarget.setPointerCapture(event.pointerId);
          dragging.current = true;
        }}
        onPointerMove={(event) => {
          if (!dragging.current || !root.current) return;
          moveTo(event.clientY - root.current.getBoundingClientRect().top - DIVIDER / 2);
        }}
        onPointerUp={(event) => {
          dragging.current = false;
          if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
        }}
        onPointerCancel={() => { dragging.current = false; }}
        onLostPointerCapture={() => { dragging.current = false; }}
        onKeyDown={(event) => {
          if (!['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return;
          event.preventDefault();
          moveTo(event.key === 'Home' ? min : event.key === 'End' ? max : upperHeight + (event.key === 'ArrowUp' ? -24 : 24));
        }}><span aria-hidden="true" /></div>
      <section className="workspace-lower" aria-label="Track library">{lower}</section>
    </div>
  );
}
