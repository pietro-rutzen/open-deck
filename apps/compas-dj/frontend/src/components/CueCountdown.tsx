import { formatCueDistance, SHOW_CUE_COUNTDOWN, type NextHotCue } from '../lib/cueCountdown';

export function CueCountdownReadout({ cue, color }: { cue: NextHotCue | null; color: string }) {
  if (!SHOW_CUE_COUNTDOWN || cue == null) return null;
  return (
    <div className="tile" style={{ color }}>
      <span className="overline" style={{ color: 'inherit' }}>CUE {cue.slot + 1} IN</span>
      <span className="mono tile-val small" style={{ color: 'inherit' }}>
        {formatCueDistance(cue.beats)}
      </span>
    </div>
  );
}
