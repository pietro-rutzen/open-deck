import { useEffect, useRef, useState } from "react";

/** Match drawing coordinates to CSS pixels rather than stretching SVG annotations. */
export function useElementSize<T extends Element>(width = 1, height = 1) {
  const ref = useRef<T>(null);
  const [size, setSize] = useState({ width, height });
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const observer = new ResizeObserver(([entry]) => {
      if (!entry) return;
      const width = Math.max(1, Math.round(entry.contentRect.width));
      const height = Math.max(1, Math.round(entry.contentRect.height));
      setSize((previous) => previous.width === width && previous.height === height ? previous : { width, height });
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, []);
  return [ref, size] as const;
}
