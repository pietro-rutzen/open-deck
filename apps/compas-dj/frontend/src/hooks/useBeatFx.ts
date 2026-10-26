import { useRef, useState } from "react";
import type { DeckController } from "./useDeck";

export const BEAT_FX_EFFECTS = ["echo", "flanger", "reverb", "crusher"] as const;
export type BeatFxEffect = (typeof BEAT_FX_EFFECTS)[number];

export const BEAT_FX_LABELS: Record<BeatFxEffect, string> = {
  echo: "Echo",
  flanger: "Flanger",
  reverb: "Reverb",
  crusher: "Crusher",
};

const BEAT_STEPS: Partial<Record<BeatFxEffect, readonly number[]>> = {
  echo: [0.25, 0.5, 1, 2],
  flanger: [1, 2, 4, 8],
};

export interface BeatFxSelection {
  effect: BeatFxEffect;
  deck: number;
}

const isEffectActive = (deck: DeckController, effect: BeatFxEffect) => deck.state[effect].active;

const toggleEffect = (deck: DeckController, effect: BeatFxEffect) => {
  const toggles: Record<BeatFxEffect, () => void> = {
    echo: deck.actions.toggleEcho,
    flanger: deck.actions.toggleFlanger,
    reverb: deck.actions.toggleReverb,
    crusher: deck.actions.toggleCrusher,
  };
  toggles[effect]();
};

const setEffectLevel = (deck: DeckController, effect: BeatFxEffect, level: number) => {
  const setters: Record<BeatFxEffect, (value: number) => void> = {
    echo: deck.actions.setEchoDepth,
    flanger: deck.actions.setFlangerDepth,
    reverb: deck.actions.setReverbMix,
    crusher: deck.actions.setCrusherCrush,
  };
  setters[effect](Math.max(0, Math.min(1, level)));
};

const stepEffectBeats = (deck: DeckController, effect: BeatFxEffect, direction: 1 | -1) => {
  const steps = BEAT_STEPS[effect];
  if (!steps) return;
  const currentBeats = effect === "echo" ? deck.state.echo.beats : deck.state.flanger.beats;
  const currentIndex = steps.indexOf(currentBeats);
  const nextIndex = Math.max(0, Math.min(steps.length - 1, (currentIndex < 0 ? 0 : currentIndex) + direction));
  if (effect === "echo") deck.actions.setEchoBeats(steps[nextIndex]);
  else deck.actions.setFlangerBeats(steps[nextIndex]);
};

const cycleEffect = (effect: BeatFxEffect, direction: 1 | -1): BeatFxEffect => {
  const count = BEAT_FX_EFFECTS.length;
  return BEAT_FX_EFFECTS[(BEAT_FX_EFFECTS.indexOf(effect) + direction + count) % count];
};

/**
 * Pioneer-style Beat FX unit driven by controller input: one selected effect type applied to one
 * selected channel, mapped onto the existing per-deck echo/flanger/reverb/crusher inserts.
 */
export function useBeatFx() {
  const [selection, setSelection] = useState<BeatFxSelection>({ effect: "echo", deck: 0 });
  const selectionRef = useRef(selection);

  const select = (next: BeatFxSelection) => {
    selectionRef.current = next;
    setSelection(next);
  };

  /** Returns true when `control` belongs to the Beat FX unit. */
  const handle = (control: string, value: number, decks: DeckController[]): boolean => {
    if (!control.startsWith("fx.")) return false;
    const current = selectionRef.current;
    const pressed = value >= 0.5;
    const targetDeck = decks[current.deck];
    const channelMatch = control.match(/^fx\.(channel|toggle)\.(\d)$/);
    if (channelMatch) {
      const deckIndex = Number(channelMatch[2]);
      const deck = decks[deckIndex];
      if (!pressed || !deck) return true;
      select({ ...current, deck: deckIndex });
      if (channelMatch[1] === "toggle") toggleEffect(deck, current.effect);
      return true;
    }
    switch (control) {
      case "fx.level":
        if (targetDeck) setEffectLevel(targetDeck, current.effect, value);
        return true;
      case "fx.next":
      case "fx.prev": {
        if (!pressed) return true;
        const nextEffect = cycleEffect(current.effect, control === "fx.next" ? 1 : -1);
        decks
          .filter((deck) => isEffectActive(deck, current.effect))
          .forEach((deck) => {
            toggleEffect(deck, current.effect);
            if (!isEffectActive(deck, nextEffect)) toggleEffect(deck, nextEffect);
          });
        select({ ...current, effect: nextEffect });
        return true;
      }
      case "fx.beat_down":
      case "fx.beat_up":
        if (pressed && targetDeck) stepEffectBeats(targetDeck, current.effect, control === "fx.beat_up" ? 1 : -1);
        return true;
      case "fx.off_all":
        if (!pressed) return true;
        decks.forEach((deck) =>
          BEAT_FX_EFFECTS.filter((effect) => isEffectActive(deck, effect)).forEach((effect) => toggleEffect(deck, effect)),
        );
        return true;
      default:
        return true;
    }
  };

  const isOn = (deck: DeckController) => isEffectActive(deck, selection.effect);

  return { selection, handle, isOn };
}
