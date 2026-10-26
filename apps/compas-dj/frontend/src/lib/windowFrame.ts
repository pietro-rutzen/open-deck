import { useEffect } from "react";
import { LogicalPosition, LogicalSize, currentMonitor, getCurrentWindow } from "@tauri-apps/api/window";
import { inTauri } from "./ipc";

const WINDOW_KEY = "opendeck.window.v1";

type Frame = { width: number; height: number; x: number; y: number };

const readFrame = (): Frame | null => {
  try {
    const raw = localStorage.getItem(WINDOW_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<Frame>;
    const width = parsed?.width;
    const height = parsed?.height;
    if (typeof width !== "number" || typeof height !== "number" || !Number.isFinite(width) || !Number.isFinite(height)) return null;
    const x = parsed.x;
    const y = parsed.y;
    return {
      width,
      height,
      x: typeof x === "number" && Number.isFinite(x) ? x : 0,
      y: typeof y === "number" && Number.isFinite(y) ? y : 0,
    };
  } catch {
    return null;
  }
};

const writeFrame = (frame: Frame) => {
  try { localStorage.setItem(WINDOW_KEY, JSON.stringify(frame)); } catch { /* Preference is optional. */ }
};

const fit = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

/** Restore the last window frame, or open large enough for the decks, mixer, and library. */
export function useWindowFrame() {
  useEffect(() => {
    if (!inTauri()) return;
    const win = getCurrentWindow();
    let ignore = true;
    let alive = true;
    const stops: Array<() => void> = [];

    const persist = async () => {
      if (ignore || !alive) return;
      const factor = await win.scaleFactor();
      const size = (await win.innerSize()).toLogical(factor);
      const position = (await win.outerPosition()).toLogical(factor);
      writeFrame({ width: size.width, height: size.height, x: position.x, y: position.y });
    };

    const restore = async () => {
      const monitor = await currentMonitor();
      const scale = monitor?.scaleFactor ?? 1;
      const workOrigin = monitor?.workArea.position.toLogical(scale);
      const workSize = monitor?.workArea.size.toLogical(scale);
      const maxW = workSize?.width ?? 1600;
      const maxH = workSize?.height ?? 1000;
      const saved = readFrame();
      const width = fit(saved?.width ?? Math.min(1720, maxW), Math.min(1280, maxW), maxW);
      const height = fit(saved?.height ?? maxH, Math.min(800, maxH), maxH);
      await win.setSize(new LogicalSize(width, height));
      if (workOrigin && workSize) {
        const x = saved
          ? fit(saved.x, workOrigin.x, workOrigin.x + Math.max(0, workSize.width - width))
          : workOrigin.x + Math.max(0, (workSize.width - width) / 2);
        const y = saved
          ? fit(saved.y, workOrigin.y, workOrigin.y + Math.max(0, workSize.height - height))
          : workOrigin.y;
        await win.setPosition(new LogicalPosition(x, y));
      }
      ignore = false;
      await persist();
    };

    void restore().catch(() => { ignore = false; });
    void win.onResized(() => { void persist(); }).then((stop) => { if (alive) stops.push(stop); else stop(); }).catch(() => {});
    void win.onMoved(() => { void persist(); }).then((stop) => { if (alive) stops.push(stop); else stop(); }).catch(() => {});
    return () => {
      alive = false;
      stops.forEach((stop) => stop());
    };
  }, []);
}
