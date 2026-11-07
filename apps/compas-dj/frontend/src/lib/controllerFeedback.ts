export const BEAT_LOOP_PADS = [0.25, 0.5, 1, 2, 4, 8, 16, 32] as const;
export const BEAT_JUMP_PADS = [-1, 1, -2, 2, -4, 4, -8, 8] as const;
/** Pioneer key-shift pad layout: absolute semitones; pads 1 and 8 reset. */
export const KEY_SHIFT_PADS = [4, 5, 6, 7, 0, 1, 2, 3] as const;

export const BEAT_FX_FEEDBACK_EFFECTS = ["echo", "flanger", "reverb", "crusher"] as const;
export type BeatFxFeedbackEffect = (typeof BEAT_FX_FEEDBACK_EFFECTS)[number];

export interface FeedbackDeckState {
  hotCues: readonly (number | null)[];
  loop: { active: boolean; beats: number | null; armed?: boolean };
  meta: unknown;
  keyPad: number | null;
  gain: number;
  trim: number;
  volume: number;
  filter: number;
  eq: { low: number; mid: number; hi: number };
  tempo: number;
  playing: boolean;
  keylock: boolean;
  synced: boolean;
  echo: { active: boolean };
  flanger: { active: boolean };
  reverb: { active: boolean };
  crusher: { active: boolean };
}

export interface FeedbackDeck {
  deck: number;
  state: FeedbackDeckState;
}

export type ControllerFeedbackEntry = [string, number];

export interface ControllerFeedbackInput<Deck extends FeedbackDeck> {
  decks: readonly Deck[];
  effectOn: (deck: Deck) => boolean;
  cued: ReadonlySet<number>;
  cueMix: number;
  cueVolume: number;
  samplerNameAt: (slot: number) => string | null;
  samplerBank: number;
  samplerGain: number;
  crossfader: number;
}

let controllerFeedbackScans = 0;

export function controllerFeedbackScanCount(): number {
  return controllerFeedbackScans;
}

export function resetControllerFeedbackScans(): void {
  controllerFeedbackScans = 0;
}

/**
 * LED and motor values for the current software state. Tempo is a ratio on the deck
 * and ±percent on the control bus. Master gain has no on-screen control, so it stays
 * on the controller-echo path only.
 */
export function collectControllerFeedback<Deck extends FeedbackDeck>(
  input: ControllerFeedbackInput<Deck>,
): ControllerFeedbackEntry[] {
  controllerFeedbackScans += 1;
  const { decks, effectOn, cued, samplerNameAt, samplerBank } = input;
  return [
    ...decks.flatMap((deck): ControllerFeedbackEntry[] => {
      const state = deck.state;
      const prefix = `deck.${deck.deck}`;
      return [
        ...state.hotCues.map((hotCue, index): ControllerFeedbackEntry => [
          `${prefix}.hotcue.${index}`,
          hotCue == null ? 0 : 1,
        ]),
        ...BEAT_LOOP_PADS.map((beats, index): ControllerFeedbackEntry => [
          `${prefix}.beatloop.${index}`,
          state.loop.active && state.loop.beats === beats ? 1 : 0,
        ]),
        ...BEAT_JUMP_PADS.map((_, index): ControllerFeedbackEntry => [
          `${prefix}.beatjump.${index}`,
          state.meta ? 1 : 0,
        ]),
        ...KEY_SHIFT_PADS.map((_, index): ControllerFeedbackEntry => [
          `${prefix}.key_shift.${index}`,
          state.meta && state.keyPad === index ? 1 : 0,
        ]),
        [`${prefix}.loop.in`, state.loop.active || state.loop.armed ? 1 : 0],
        [`${prefix}.loop.out`, state.loop.active ? 1 : 0],
        [`${prefix}.loop.toggle`, state.loop.active ? 1 : 0],
        [`${prefix}.gain`, state.gain],
        [`${prefix}.trim`, state.trim],
        [`${prefix}.volume`, state.volume],
        [`${prefix}.headphone_cue`, cued.has(deck.deck) ? 1 : 0],
        [`${prefix}.filter`, state.filter],
        [`${prefix}.eq.low`, state.eq.low],
        [`${prefix}.eq.mid`, state.eq.mid],
        [`${prefix}.eq.high`, state.eq.hi],
        [`${prefix}.tempo`, (state.tempo - 1) * 100],
        [`${prefix}.play`, state.playing ? 1 : 0],
        [`${prefix}.keylock`, state.keylock ? 1 : 0],
        [`${prefix}.sync`, state.synced ? 1 : 0],
        [`fx.toggle.${deck.deck}`, effectOn(deck) ? 1 : 0],
      ];
    }),
    ...Array.from({ length: 16 }, (_, local): ControllerFeedbackEntry => [
      `sampler.${local}.trigger`,
      samplerNameAt(samplerBank * 16 + local) ? 1 : 0,
    ]),
    ["mixer.crossfader", input.crossfader],
    ["mixer.cue_mix", input.cueMix],
    ["mixer.cue_volume", input.cueVolume],
    ["sampler.gain", input.samplerGain],
  ];
}

export function deliverControllerFeedback(
  values: readonly ControllerFeedbackEntry[],
  force: boolean,
  sent: Map<string, number>,
  emit: (control: string, value: number) => void,
): void {
  for (const [control, value] of values) {
    if (!force && sent.get(control) === value) continue;
    sent.set(control, value);
    emit(control, value);
  }
}
