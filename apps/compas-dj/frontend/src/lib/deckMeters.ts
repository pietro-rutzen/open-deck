import { invoke } from "@tauri-apps/api/core";

/** Paired display samples, a dBFS spectrum at evenly spaced log frequencies, and its Hz limits. */
export interface DeckMeterFrame {
  left: number[];
  right: number[];
  spectrum: number[];
  minHz: number;
  maxHz: number;
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
