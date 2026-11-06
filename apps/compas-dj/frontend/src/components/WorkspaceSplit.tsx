import { useEffect, useRef, useState, type ReactNode } from "react";

const clampSize = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

export function useStoredSize(key: string, min: number, max: number) {
  const [size, setSize] = useState<number | null>(() => {
    try {
      const saved = localStorage.getItem(key);
      if (saved == null || saved.trim() === "") return null;
      const parsed = Number(saved);
      return Number.isFinite(parsed) ? clampSize(parsed, min, max) : null;
    } catch {
      return null;
    }
  });
  useEffect(() => {
    if (size == null) return;
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
  const paneWidth = (divider: HTMLElement) => {
    const pane = grow === 1 ? divider.previousElementSibling : divider.nextElementSibling;
    return pane instanceof HTMLElement ? pane.getBoundingClientRect().width : value;
  };
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
        drag.current = { x: event.clientX, width: paneWidth(event.currentTarget) };
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
        apply(event.key === "Home" ? min : event.key === "End" ? max : paneWidth(event.currentTarget) + step * grow);
      }}
    ><span aria-hidden="true" /></div>
  );
}

export function MixerFrame({ children }: { children: ReactNode }) {
  const [width, setWidth] = useStoredSize("opendeck.mixer.width.v2", 320, 720);
  return (
    <>
      <PaneDivider label="Resize mixer" value={width ?? 360} min={320} max={720} fallback={360} grow={-1} onChange={setWidth} />
      <div className="mixer-size" style={width == null ? undefined : { width }}>{children}</div>
      <PaneDivider label="Resize mixer" value={width ?? 360} min={320} max={720} fallback={360} grow={1} onChange={setWidth} />
    </>
  );
}

const STORAGE_KEY = "opendeck.workspace.split.v2";
const DIVIDER = 1;
const DEFAULT_RATIO = 0.58;
const MIN_PANE = 140;
const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

const readRatio = () => {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved == null || saved.trim() === "") return null;
    const value = Number(saved);
    return Number.isFinite(value) && value > 0 && value < 1 ? value : null;
  } catch {
    return null;
  }
};

/** Resize the performance layout within usable control and library bounds. */
export function WorkspaceSplit({ upper, lower }: { upper: ReactNode; lower: ReactNode }) {
  const root = useRef<HTMLDivElement>(null);
  const dragging = useRef(false);
  const [height, setHeight] = useState(0);
  const [ratio, setRatio] = useState<number | null>(readRatio);
  useEffect(() => {
    const element = root.current;
    if (!element) return;
    const observer = new ResizeObserver(() => setHeight(element.clientHeight));
    observer.observe(element);
    setHeight(element.clientHeight);
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    if (ratio == null) {
      try { localStorage.removeItem(STORAGE_KEY); } catch { /* Preference is optional. */ }
      return;
    }
    try { localStorage.setItem(STORAGE_KEY, String(ratio)); } catch { /* Preference is optional. */ }
  }, [ratio]);

  const available = Math.max(0, height - DIVIDER);
  const min = Math.min(MIN_PANE, available);
  const max = Math.max(min, available - Math.min(MIN_PANE, available));
  // Leave enough deck height on laptop screens at the default split. An explicit
  // divider position still wins; the upper pane then scrolls as one unit.
  const preferred = ratio == null ? Math.max(available * DEFAULT_RATIO, 520) : available * ratio;
  const upperHeight = clamp(preferred, min, max);
  const moveTo = (pixels: number) => {
    if (available > 0) setRatio(clamp(pixels, min, max) / available);
  };
  return (
    <div className="workspace-split" ref={root}
      style={{ gridTemplateRows: height && upperHeight ? `${upperHeight}px ${DIVIDER}px minmax(0, 1fr)` : `max-content ${DIVIDER}px minmax(220px, 1fr)` }}>
      <section className="workspace-upper" aria-label="Decks and mixer">{upper}</section>
      <div className="workspace-divider" role="separator" tabIndex={0}
        aria-label="Resize decks and library" aria-orientation="horizontal"
        aria-valuemin={Math.round(min)} aria-valuemax={Math.round(max)} aria-valuenow={Math.round(upperHeight)}
        aria-valuetext={`Decks ${Math.round(upperHeight)} pixels, library ${Math.round(available - upperHeight)} pixels`}
        title="Drag to resize decks and library · double-click to reset"
        onDoubleClick={() => setRatio(null)}
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
