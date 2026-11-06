import { invoke } from "@tauri-apps/api/core";

/** Paired display samples, measured bin frequencies and peak-sine dBFS, and the displayed Hz limits. */
export interface DeckMeterFrame {
  left: number[];
  right: number[];
  spectrum: number[];
  spectrumHz: number[];
  minHz: number;
  maxHz: number;
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

const pending = new Set<number>();

/** A read also arms one subsequent short capture. No polling means no ongoing capture. */
export async function readDeckMeters(
  deck: number,
  spectrum: boolean,
): Promise<DeckMeterFrame | null> {
  if (pending.has(deck)) return null;
  pending.add(deck);
  try {
    return await invoke<DeckMeterFrame | null>("read_deck_meters", {
      deck,
      spectrum,
    });
  } finally {
    pending.delete(deck);
  }
}
