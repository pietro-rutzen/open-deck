export interface MemoryLoopPoint {
  slot: number;
  in_frame: number;
  out_frame: number;
  active: boolean;
  point?: boolean;
}

export interface MemoryLoopSpan {
  slot: number;
  x: number;
  width: number;
  active: boolean;
}

export interface MemoryLoopFraction {
  slot: number;
  left: number;
  width: number;
  active: boolean;
}

const SAVED_LOOP_COLOR = "#e6b325";
const ACTIVE_LOOP_COLOR = "#e23b3b";

export function memoryLoopColor(active: boolean): string {
  return active ? ACTIVE_LOOP_COLOR : SAVED_LOOP_COLOR;
}

export function memoryMatchesLoop(
  loops: readonly MemoryLoopPoint[],
  inFrame: number,
  outFrame: number,
): boolean {
  return loops.some(
    (loop) => Math.abs(loop.in_frame - inFrame) < 1 && Math.abs(loop.out_frame - outFrame) < 1,
  );
}

function usableLoop(loop: MemoryLoopPoint): boolean {
  return Number.isFinite(loop.in_frame) && Number.isFinite(loop.out_frame) && loop.out_frame > loop.in_frame;
}

export function memoryLoopSpans(
  loops: readonly MemoryLoopPoint[],
  sampleRate: number,
  stretch: number,
  origin: number,
  view: number,
  width: number,
): MemoryLoopSpan[] {
  if (!(sampleRate > 0) || !(stretch > 0) || !(view > 0) || !(width > 0)) return [];
  const scale = width / view;
  return loops.flatMap((loop) => {
    if (!usableLoop(loop)) return [];
    const start = loop.in_frame / sampleRate / stretch;
    const end = loop.out_frame / sampleRate / stretch;
    const x = (start - origin) * scale;
    const span = (end - start) * scale;
    if (x + span <= 0 || x >= width) return [];
    return [{ slot: loop.slot, x, width: span, active: loop.active }];
  });
}

export interface MemoryCueMark {
  slot: number;
  x: number;
}

export interface MemoryCueFraction {
  slot: number;
  left: number;
}

export function isMemoryCue(loop: MemoryLoopPoint): boolean {
  return loop.point === true && Number.isFinite(loop.in_frame);
}

export function memoryCueMarks(
  loops: readonly MemoryLoopPoint[],
  sampleRate: number,
  stretch: number,
  origin: number,
  view: number,
  width: number,
): MemoryCueMark[] {
  if (!(sampleRate > 0) || !(stretch > 0) || !(view > 0) || !(width > 0) || !Number.isFinite(origin)) return [];
  const scale = width / view;
  return loops.flatMap((loop) => {
    if (!isMemoryCue(loop)) return [];
    const seconds = loop.in_frame / sampleRate / stretch;
    if (seconds < origin || seconds > origin + view) return [];
    return [{ slot: loop.slot, x: (seconds - origin) * scale }];
  });
}

export function memoryCueFractions(
  loops: readonly MemoryLoopPoint[],
  frames: number,
): MemoryCueFraction[] {
  if (!(frames > 0)) return [];
  return loops.flatMap((loop) => {
    if (!isMemoryCue(loop)) return [];
    return [{ slot: loop.slot, left: Math.max(0, Math.min(1, loop.in_frame / frames)) }];
  });
}

export function memoryLoopFractions(
  loops: readonly MemoryLoopPoint[],
  frames: number,
): MemoryLoopFraction[] {
  if (!(frames > 0)) return [];
  return loops.flatMap((loop) => {
    if (!usableLoop(loop)) return [];
    const left = Math.max(0, Math.min(1, loop.in_frame / frames));
    const right = Math.max(0, Math.min(1, loop.out_frame / frames));
    if (right <= left) return [];
    return [{ slot: loop.slot, left, width: right - left, active: loop.active }];
  });
}
