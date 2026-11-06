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

export const spectrumScaleLabels = [
  { hz: 100, label: "100Hz" },
  { hz: 1000, label: "1kHz" },
  { hz: 10000, label: "10kHz" },
] as const;

const spectrumGridSteps = [1, 2, 3, 4, 5, 6, 8];

export function spectrumGridFrequencies(lowHz: number, highHz: number) {
  if (!(highHz > lowHz) || !(lowHz > 0)) return [];
  const startDecade = Math.floor(Math.log10(lowHz));
  const endDecade = Math.ceil(Math.log10(highHz));
  const decades = Array.from(
    { length: endDecade - startDecade + 1 },
    (_, index) => startDecade + index,
  );
  return decades.flatMap((decade) =>
    spectrumGridSteps
      .map((step) => step * 10 ** decade)
      .filter((hz) => hz > lowHz && hz < highHz),
  );
}

export function spectrumFrequencyX(
  hz: number,
  lowHz: number,
  highHz: number,
  width: number,
) {
  const span = Math.log(highHz / lowHz);
  if (!(span > 0) || !(hz > 0) || !(width > 0)) return 0;
  return (width * Math.log(hz / lowHz)) / span;
}

export function frequencyAtPlotPosition(
  position: number,
  lowHz: number,
  highHz: number,
  size: number,
) {
  if (!(size > 0) || !(highHz > lowHz) || !(lowHz > 0)) return lowHz;
  return lowHz * (highHz / lowHz) ** (position / size);
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
