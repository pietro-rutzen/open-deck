// Per-slot hot-cue colors (Serato-style 8-color palette). Hex so an alpha suffix (`${c}30`) is
// valid CSS — a CSS var like `var(--accent)` can't take an appended alpha.
export const CUE_COLORS = [
  '#ff5b4c',
  '#ff8c1a',
  '#ffcf3a',
  '#3ddc97',
  '#28e0ff',
  '#2ea6ff',
  '#9b6bff',
  '#ff5bbf',
];

/** Project source-frame cues into the same stretched time axis as the scrolling audio. */
export function hotCueMarkers(cues: readonly (number | null)[], sourceRate: number, stretch: number, origin: number, view: number, width: number, span: number) {
  if (![sourceRate, stretch, view, width, span].every(value => Number.isFinite(value) && value > 0) || !Number.isFinite(origin)) return [];
  return cues.flatMap((frame, slot) => {
    if (frame == null || !Number.isFinite(frame) || frame < 0) return [];
    const seconds = frame / sourceRate / stretch;
    if (seconds < origin || seconds > origin + span) return [];
    return [{ slot, x: (seconds - origin) * width / view, color: CUE_COLORS[slot % CUE_COLORS.length] }];
  });
}

export interface HotCueLoopSpan {
  slot: number;
  x: number;
  width: number;
  color: string;
}

/** Loop range stored on a hot cue, in the same stretched axis as the cue marker. */
export function hotCueLoopSpans(
  cues: readonly (number | null)[],
  outs: readonly (number | null)[],
  sourceRate: number,
  stretch: number,
  origin: number,
  view: number,
  width: number,
): HotCueLoopSpan[] {
  if (![sourceRate, stretch, view, width].every(value => Number.isFinite(value) && value > 0) || !Number.isFinite(origin)) return [];
  const scale = width / view;
  return cues.flatMap((frame, slot) => {
    const out = outs[slot];
    if (frame == null || out == null || !Number.isFinite(frame) || !Number.isFinite(out) || out <= frame) return [];
    const start = frame / sourceRate / stretch;
    const end = out / sourceRate / stretch;
    const x = (start - origin) * scale;
    const span = (end - start) * scale;
    if (x + span <= 0 || x >= width) return [];
    return [{ slot, x, width: span, color: CUE_COLORS[slot % CUE_COLORS.length] }];
  });
}
