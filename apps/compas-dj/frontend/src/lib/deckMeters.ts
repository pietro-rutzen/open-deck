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
