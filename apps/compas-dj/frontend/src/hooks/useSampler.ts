import { useCallback, useEffect, useState } from "react";
import {
  clearSample,
  inTauri,
  loadSample,
  onAudioOutputReset,
  pickAudioFile,
  samplerBank,
  setSampleLoop,
  setSamplerBank,
  setSamplerGain,
  stopSample,
  triggerSample,
} from "../lib/ipc";

export interface Pad {
  /** Display name when a sample is loaded, else null (empty pad). */
  name: string | null;
  /** Loop mode: triggering toggles play/stop instead of firing a one-shot. */
  loop: boolean;
  /** True while a file is decoding into this pad. */
  loading: boolean;
}

const BANKS = 4;
const SLOTS_PER_BANK = 16;
const SLOT_COUNT = BANKS * SLOTS_PER_BANK;
const VISIBLE_PADS = 8;
const emptyNames = (): (string | null)[] => Array.from({ length: SLOT_COUNT }, () => null);
const emptyLoops = (): boolean[] => Array.from({ length: SLOT_COUNT }, () => false);

/**
 * Sampler slots. The panel shows the active bank's first eight (deck 1). Deck 2's eight
 * are the next slots in that bank and are loaded from the controller. Playback is in the engine.
 */
export function useSampler() {
  const [names, setNames] = useState<(string | null)[]>(emptyNames);
  const [loops, setLoops] = useState<boolean[]>(emptyLoops);
  const [loadingSlot, setLoadingSlot] = useState<number | null>(null);
  const [bank, setBank] = useState(0);
  const [gain, setGainState] = useState(0.9);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!inTauri()) return;
    samplerBank()
      .then((current) => setBank(Math.max(0, Math.min(BANKS - 1, current))))
      .catch(() => {});
  }, []);

  useEffect(() => {
    if (!inTauri()) return;
    let active = true;
    const unlisten = onAudioOutputReset(() => {
      if (!active) return;
      setNames((current) => {
        if (current.some((name) => name)) {
          setError("Speakers changed. Load sampler pads again.");
        }
        return emptyNames();
      });
      setLoops(emptyLoops());
    });
    return () => {
      active = false;
      void unlisten.then((stop) => stop()).catch(() => {});
    };
  }, []);

  const pads: Pad[] = Array.from({ length: VISIBLE_PADS }, (_, index) => {
    const slot = bank * SLOTS_PER_BANK + index;
    return {
      name: names[slot] ?? null,
      loop: loops[slot] ?? false,
      loading: loadingSlot === slot,
    };
  });

  const loadPath = useCallback(async (slot: number, path: string) => {
    if (slot < 0 || slot >= SLOT_COUNT) {
      setError("That sampler slot is not available.");
      return;
    }
    setError(null);
    setLoadingSlot(slot);
    try {
      const loaded = await loadSample(slot, path);
      setNames((current) => current.map((name, index) => (index === slot ? loaded.name : name)));
    } catch (cause) {
      setError(`Could not load sample: ${String(cause)}`);
    } finally {
      setLoadingSlot((current) => (current === slot ? null : current));
    }
  }, []);

  const load = useCallback(
    async (visible: number) => {
      setError(null);
      try {
        const path = await pickAudioFile();
        if (!path) return;
        await loadPath(bank * SLOTS_PER_BANK + visible, path);
      } catch (cause) {
        setError(`Could not load sample: ${String(cause)}`);
      }
    },
    [bank, loadPath],
  );

  const clear = useCallback(
    (visible: number) => {
      const slot = bank * SLOTS_PER_BANK + visible;
      clearSample(slot).catch(() => {});
      setNames((current) => current.map((name, index) => (index === slot ? null : name)));
      setLoops((current) => current.map((loop, index) => (index === slot ? false : loop)));
    },
    [bank],
  );

  const trigger = useCallback(
    (visible: number) => {
      triggerSample(bank * SLOTS_PER_BANK + visible).catch(() => {});
    },
    [bank],
  );

  const stop = useCallback(
    (visible: number) => {
      stopSample(bank * SLOTS_PER_BANK + visible).catch(() => {});
    },
    [bank],
  );

  const toggleLoop = useCallback(
    (visible: number) => {
      const slot = bank * SLOTS_PER_BANK + visible;
      setLoops((current) =>
        current.map((loop, index) => {
          if (index !== slot) return loop;
          const next = !loop;
          setSampleLoop(slot, next).catch(() => {});
          return next;
        }),
      );
    },
    [bank],
  );

  const setGain = useCallback((g: number) => {
    setGainState(g);
    setSamplerGain(g).catch(() => {});
  }, []);

  const cycleBank = useCallback(() => {
    const next = (bank + 1) % BANKS;
    setBank(next);
    setSamplerBank(next).catch(() => {});
  }, [bank]);

  const nameAt = useCallback((slot: number) => names[slot] ?? null, [names]);

  return { pads, gain, error, bank, load, loadPath, clear, trigger, stop, toggleLoop, setGain, cycleBank, nameAt };
}

export type SamplerController = ReturnType<typeof useSampler>;
