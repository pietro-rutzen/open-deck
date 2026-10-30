import { useCallback, useEffect, useRef, useState } from "react";
import {
  clearSample,
  inTauri,
  loadSample,
  onAudioOutputReset,
  pickAudioFile,
  samplerPadCount,
  setSampleLoop,
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

const DEFAULT_PADS = 8;
const emptyPad = (): Pad => ({ name: null, loop: false, loading: false });

/**
 * Sampler / performance pads. Load a short audio file onto a pad, then trigger it — one-shots
 * overlap (polyphonic), looped pads toggle. Playback runs in the engine on the master bus
 * (recordable); this hook only tracks pad assignments + the global level.
 */
export function useSampler() {
  const [pads, setPads] = useState<Pad[]>(() => Array.from({ length: DEFAULT_PADS }, emptyPad));
  const [gain, setGainState] = useState(0.9);
  const [error, setError] = useState<string | null>(null);
  const loading = useRef(false);

  useEffect(() => {
    if (!inTauri()) return;
    samplerPadCount()
      .then((n) => setPads((cur) => (n === cur.length ? cur : Array.from({ length: n }, emptyPad))))
      .catch(() => {});
  }, []);

  useEffect(() => {
    if (!inTauri()) return;
    let active = true;
    const unlisten = onAudioOutputReset(() => {
      if (!active) return;
      setPads((current) => {
        if (current.some((pad) => pad.name)) {
          setError("Speakers changed. Load sampler pads again.");
        }
        return current.map(() => emptyPad());
      });
    });
    return () => {
      active = false;
      void unlisten.then((stop) => stop()).catch(() => {});
    };
  }, []);

  const patch = (slot: number, p: Partial<Pad>) =>
    setPads((cur) => cur.map((pad, i) => (i === slot ? { ...pad, ...p } : pad)));

  const load = useCallback(async (slot: number) => {
    if (loading.current) return;
    loading.current = true;
    setError(null);
    try {
      const path = await pickAudioFile();
      if (!path) return;
      patch(slot, { loading: true });
      const r = await loadSample(slot, path);
      patch(slot, { name: r.name });
    } catch (cause) {
      setError(`Could not load sample: ${String(cause)}`);
    } finally {
      patch(slot, { loading: false });
      loading.current = false;
    }
  }, []);

  const clear = useCallback((slot: number) => {
    clearSample(slot).catch(() => {});
    patch(slot, { name: null, loop: false });
  }, []);

  const trigger = useCallback((slot: number) => {
    triggerSample(slot).catch(() => {});
  }, []);

  const stop = useCallback((slot: number) => {
    stopSample(slot).catch(() => {});
  }, []);

  const toggleLoop = useCallback((slot: number) => {
    setPads((cur) =>
      cur.map((pad, i) => {
        if (i !== slot) return pad;
        const loop = !pad.loop;
        setSampleLoop(slot, loop).catch(() => {});
        return { ...pad, loop };
      }),
    );
  }, []);

  const setGain = useCallback((g: number) => {
    setGainState(g);
    setSamplerGain(g).catch(() => {});
  }, []);

  return { pads, gain, error, load, clear, trigger, stop, toggleLoop, setGain };
}

export type SamplerController = ReturnType<typeof useSampler>;
