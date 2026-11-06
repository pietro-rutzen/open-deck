type LoopBounds = { active: boolean; inFrame: number; outFrame: number };

/** Source-frame distances are independent of tempo and beatgrid anchor. */
export function nextHotCue(
  cues: readonly (number | null)[], frame: number, sourceRate: number,
  beatInterval: number, loop: LoopBounds,
) {
  if (!Number.isFinite(frame) || frame < 0 || !Number.isFinite(sourceRate) ||
      sourceRate <= 0 || !Number.isFinite(beatInterval) || beatInterval <= 0.05) return null;
  const framesPerBeat = beatInterval * sourceRate;
  const willWrap = loop.active && loop.inFrame >= 0 &&
    loop.inFrame < loop.outFrame && frame < loop.outFrame;
  let nearest = Infinity;
  let slot = -1;
  for (let i = 0; i < cues.length; i++) {
    const cue = cues[i];
    if (cue === null || !Number.isFinite(cue) || cue < 0) continue;
    if (willWrap && cue >= loop.outFrame) continue;
    let distance = cue - frame;
    if (willWrap && distance <= 0 && cue >= loop.inFrame) {
      distance += loop.outFrame - loop.inFrame;
    }
    if (distance > 0 && distance < nearest) {
      nearest = distance;
      slot = i;
    }
  }
  // Snapped cues are rounded to whole source frames; keep exact beat counts
  // when a fractional beat boundary rounds upward by less than half a frame.
  return slot < 0 ? null : { slot, beats: Math.max(1, Math.ceil((nearest - 0.5) / framesPerBeat)) };
}

export function formatCueDistance(beats: number) {
  const bars = Math.floor(beats / 4);
  const rest = beats % 4;
  const beatText = rest === 1 ? '1 beat' : rest + ' beats';
  if (!bars) return beatText;
  const barText = bars === 1 ? '1 bar' : bars + ' bars';
  return rest ? barText + ' + ' + beatText : barText;
}
