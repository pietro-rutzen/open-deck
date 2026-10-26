/** Channel trim law, matching `Behavior::Trim` in the engine control bus.
 *  Full counter-clockwise is silence, 12 o'clock is unity, full clockwise is +18 dB. */
export const TRIM_MAX = 8;
export const TRIM_UNITY = 1;

export function trimGain(norm: number): number {
  const n = Math.min(1, Math.max(0, norm));
  if (n <= 0.5) {
    const travel = n / 0.5;
    return travel * travel;
  }
  const travel = (n - 0.5) / 0.5;
  return TRIM_MAX ** travel;
}

export function trimNorm(gain: number): number {
  const g = Math.min(TRIM_MAX, Math.max(0, gain));
  if (g <= TRIM_UNITY) return 0.5 * Math.sqrt(g);
  return 0.5 + 0.5 * (Math.log(g) / Math.log(TRIM_MAX));
}
