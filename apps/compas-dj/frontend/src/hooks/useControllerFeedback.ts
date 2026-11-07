import { useCallback, useEffect, useMemo, useRef } from "react";
import {
  collectControllerFeedback,
  deliverControllerFeedback,
  type BeatFxFeedbackEffect,
  type ControllerFeedbackEntry,
  type FeedbackDeck,
} from "../lib/controllerFeedback";

interface ControllerFeedbackController<Deck extends FeedbackDeck> {
  decks: readonly Deck[];
  beatFx: {
    selection: { effect: BeatFxFeedbackEffect };
    isOn: (deck: Deck) => boolean;
  };
  cue: { cued: ReadonlySet<number>; mix: number; volume: number };
  sampler: { nameAt: (slot: number) => string | null; bank: number; gain: number };
  crossfader: number;
  emit: (control: string, value: number) => void;
}

// These are the values collectControllerFeedback and beatFx.isOn read. Playhead frame,
// level, rate, and latency stay off this list: the deck object and its state object
// are new on those ticks, and listing them would rebuild the device feedback every frame.
function feedbackInputs<Deck extends FeedbackDeck>(
  input: ControllerFeedbackController<Deck>,
): readonly unknown[] {
  const effect = input.beatFx.selection.effect;
  const inputs: unknown[] = [];
  for (const deck of input.decks) {
    const state = deck.state;
    inputs.push(
      state.hotCues,
      state.loop,
      state.meta != null,
      state.keyPad,
      state.gain,
      state.trim,
      state.volume,
      state.filter,
      state.eq,
      state.tempo,
      state.playing,
      state.keylock,
      state.synced,
      state[effect].active,
    );
  }
  inputs.push(
    effect,
    input.cue.cued,
    input.cue.mix,
    input.cue.volume,
    input.sampler.nameAt,
    input.sampler.bank,
    input.sampler.gain,
    input.crossfader,
  );
  return inputs;
}

export function useControllerFeedback<Deck extends FeedbackDeck>(
  input: ControllerFeedbackController<Deck>,
): { resend: () => void } {
  const inputs = feedbackInputs(input);
  const values = useMemo(
    () => collectControllerFeedback({
      decks: input.decks,
      effectOn: (deck) => input.beatFx.isOn(deck),
      cued: input.cue.cued,
      cueMix: input.cue.mix,
      cueVolume: input.cue.volume,
      samplerNameAt: input.sampler.nameAt,
      samplerBank: input.sampler.bank,
      samplerGain: input.sampler.gain,
      crossfader: input.crossfader,
    }),
    inputs,
  );
  const valuesRef = useRef(values);
  valuesRef.current = values;
  const sentRef = useRef(new Map<string, number>());
  const emitRef = useRef(input.emit);
  emitRef.current = input.emit;
  const send = useCallback((next: readonly ControllerFeedbackEntry[], force: boolean) => {
    deliverControllerFeedback(next, force, sentRef.current, (control, value) => {
      emitRef.current(control, value);
    });
  }, []);
  useEffect(() => {
    send(values, false);
  }, [values, send]);
  const resend = useCallback(() => {
    send(valuesRef.current, true);
  }, [send]);
  useEffect(() => {
    const onResync = () => resend();
    window.addEventListener("controller:resync", onResync);
    return () => window.removeEventListener("controller:resync", onResync);
  }, [resend]);
  return { resend };
}
