import { invoke } from "@tauri-apps/api/core";

/** Paired display samples, measured bin frequencies and peak-sine dBFS, and the displayed Hz limits. */
export interface DeckMeterFrame {
  left: number[];
  right: number[];
  spectrum: number[];
  spectrumHz: number[];
  minHz: number;
  maxHz: number;
  /** Advances when the captured window moves. An unchanged value is not a new reading. */
  sequence: number;
}

/**
 * Goniometer pixel for one PCM pair.
 * Sum and difference stay in sample units. A full-scale pair (|left±right| = 2)
 * lands on the plot edge; half-scale stays halfway there.
 */
export function stereoImagePoint(
  left: number,
  right: number,
  centerX: number,
  centerY: number,
  halfWidth: number,
  halfHeight: number,
) {
  return {
    x: centerX + ((left - right) * halfWidth) / 2,
    y: centerY - ((left + right) * halfHeight) / 2,
  };
}

export const spectrumThresholds = [
  { hz: 100, label: "100Hz" },
  { hz: 1000, label: "1kHz" },
  { hz: 10000, label: "10kHz" },
] as const;

/** Analyzer floor. 0 dBFS is the top of the spectrum plot; this level is the bottom. */
export const spectrumFloorDb = -72;

export function spectrumPlotX(
  hz: number,
  lowHz: number,
  highHz: number,
  width: number,
) {
  const span = Math.log(highHz / lowHz);
  if (!(span > 0) || !(hz > 0) || !(width > 0)) return 0;
  return (width * Math.log(hz / lowHz)) / span;
}

export function spectrumPlotY(db: number, top: number, bottom: number) {
  const span = bottom - top;
  if (!(span > 0)) return top;
  const fraction = Math.max(0, Math.min(1, (0 - db) / (0 - spectrumFloorDb)));
  return top + span * fraction;
}

type MeterRead = {
  deck: number;
  spectrum: boolean;
  since: number;
  resolve: (frame: DeckMeterFrame | null) => void;
  reject: (reason: unknown) => void;
};

const pendingReads: MeterRead[] = [];
let flushScheduled = false;

function flushMeterReads() {
  flushScheduled = false;
  const batch = pendingReads.splice(0, pendingReads.length);
  if (!batch.length) return;
  void invoke<Array<DeckMeterFrame | null>>("read_deck_meters", {
    queries: batch.map(({ deck, spectrum, since }) => ({ deck, spectrum, since })),
  })
    .then((frames) => {
      batch.forEach((read, index) => {
        read.resolve(frames[index] ?? null);
      });
    })
    .catch((error: unknown) => {
      batch.forEach((read) => {
        read.reject(error);
      });
    });
}

const frameListeners = new Set<() => void>();
let meterFrame = 0;

function runMeterFrame() {
  for (const listener of frameListeners) listener();
  if (frameListeners.size === 0) {
    meterFrame = 0;
    return;
  }
  meterFrame = requestAnimationFrame(runMeterFrame);
}

/** One display callback shared by every visible meter strip. */
export function onMeterFrame(listener: () => void) {
  frameListeners.add(listener);
  if (!meterFrame) meterFrame = requestAnimationFrame(runMeterFrame);
  return () => {
    frameListeners.delete(listener);
    if (frameListeners.size === 0 && meterFrame) {
      cancelAnimationFrame(meterFrame);
      meterFrame = 0;
    }
  };
}

/** A read arms capture briefly. Strips started in the same frame share one command. */
export function readDeckMeters(
  deck: number,
  spectrum: boolean,
  since = 0,
): Promise<DeckMeterFrame | null> {
  return new Promise((resolve, reject) => {
    pendingReads.push({ deck, spectrum, since, resolve, reject });
    if (!flushScheduled) {
      flushScheduled = true;
      queueMicrotask(flushMeterReads);
    }
  });
}
