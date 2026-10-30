import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  configureCueOutput, cueSession, inTauri, listOutputDeviceDetails, onCueSession,
  setCueMix, setCueVolume, setDeckCue, startCueOutput, stopCueOutput,
  type CueSession, type OutputDeviceInfo,
} from "../lib/ipc";

/** Native routing is authoritative; selecting PFL starts only a validated cue route. */
export interface CueApi {
  devices: string[];
  details: OutputDeviceInfo[];
  device: string | null;
  setDevice: (name: string | null) => void;
  firstChannel: number;
  setChannel: (firstChannel: number) => void;
  rescan: () => void;
  enabled: boolean;
  connectedName: string | null;
  error: string | null;
  toggle: () => Promise<void>;
  mix: number;
  setMix: (v: number) => void;
  volume: number;
  setVolume: (v: number) => void;
  cued: Set<number>;
  toggleDeckCue: (deck: number) => void;
}
const INITIAL: CueSession = {
  enabled: false, device: null, first_channel: 0, connected_name: null,
  mix: 0, volume: 0.8, cued: [], error: null,
};

export function useCue(): CueApi {
  const [state, setState] = useState<CueSession>(INITIAL);
  const [details, setDetails] = useState<OutputDeviceInfo[]>([]);
  const [actionError, setActionError] = useState<{ key: string; message: string } | null>(null);
  const live = useRef(true);
  const version = useRef(0);
  const readRequest = useRef(0);
  const scanRequest = useRef(0);
  // At most eight pending keys: route, enabled, mix, volume and four deck PFL states.
  const pending = useRef(new Map<string, () => Promise<unknown>>());
  const activeKey = useRef<string | null>(null);
  const running = useRef<Promise<void> | null>(null);
  const desired = useRef({ enabled: false, device: null as string | null, channel: 0, cued: new Set<number>() });
  const busy = (key: string) => activeKey.current === key || pending.current.has(key);
  const apply = useCallback((session: CueSession) => {
    if (!live.current) return;
    if (!busy("enabled")) desired.current.enabled = session.enabled;
    if (!busy("route")) {
      desired.current.device = session.device;
      desired.current.channel = session.first_channel;
    }
    for (let deck = 0; deck < 4; deck++) {
      if (busy(`deck${deck}`)) continue;
      if (session.cued.includes(deck)) desired.current.cued.add(deck);
      else desired.current.cued.delete(deck);
    }
    setState(session);
  }, []);
  const refresh = useCallback(async () => {
    if (!inTauri()) return;
    const started = version.current;
    const request = ++readRequest.current;
    try {
      const session = await cueSession();
      if (started === version.current && request === readRequest.current) {
        apply(session);
        if (live.current) setActionError((previous) => previous?.key === "status" ? null : previous);
      }
    } catch (error) {
      if (live.current) setActionError({ key: "status", message: `Cannot read headphone status: ${String(error)}` });
    }
  }, [apply]);
  const rescan = useCallback(() => {
    if (!inTauri()) return;
    const request = ++scanRequest.current;
    void listOutputDeviceDetails().then((devices) => {
      if (live.current && request === scanRequest.current) {
        setDetails(devices);
        setActionError((previous) => previous?.key === "devices" ? null : previous);
      }
    }).catch((error) => {
      if (live.current) setActionError({ key: "devices", message: `Cannot list audio outputs: ${String(error)}` });
    });
    void refresh();
  }, [refresh]);
  useEffect(() => {
    live.current = true;
    let cancelled = false;
    if (!inTauri()) return () => { live.current = false; };
    const unlisten = onCueSession((session) => { if (!cancelled) { version.current++; apply(session); } });
    void unlisten.then(() => { if (!cancelled) rescan(); }).catch((error) => {
      if (!cancelled) { setActionError({ key: "events", message: `Cannot follow headphone status: ${String(error)}` }); rescan(); }
    });
    return () => {
      cancelled = true;
      live.current = false;
      pending.current.clear();
      void unlisten.then((stop) => stop()).catch(() => {});
    };
  }, [apply, rescan]);

  const enqueue = useCallback((key: string, command: () => Promise<unknown>): Promise<void> => {
    if (!inTauri()) return Promise.resolve();
    pending.current.set(key, command);
    if (!running.current) {
      const drain = async () => {
        while (pending.current.size && live.current) {
          const [nextKey, next] = pending.current.entries().next().value!;
          pending.current.delete(nextKey);
          activeKey.current = nextKey;
          try {
            await next();
            if (live.current) setActionError((previous) => previous?.key === nextKey ? null : previous);
          } catch (error) {
            if (live.current) setActionError({ key: nextKey, message: `Headphones: ${String(error)}` });
          }
          activeKey.current = null;
          await refresh();
        }
      };
      running.current = drain().finally(() => { running.current = null; });
    }
    return running.current;
  }, [refresh]);
  const configure = useCallback((device: string | null, channel: number) => {
    desired.current.device = device;
    desired.current.channel = channel;
    void enqueue("route", async () => { await configureCueOutput(device, channel); });
  }, [enqueue]);
  const setDevice = useCallback((device: string | null) => {
    const channels = details.find((entry) => entry.name === device)?.channels ?? 0;
    const headphonePair = device?.toLowerCase().includes("ddj-flx4") && channels >= 4 ? 2 : 0;
    configure(device, headphonePair);
  }, [configure, details]);
  const setChannel = useCallback((channel: number) => {
    if (Number.isInteger(channel) && channel >= 0) configure(desired.current.device, channel);
  }, [configure]);
  const toggle = useCallback(() => {
    const enabled = !desired.current.enabled;
    desired.current.enabled = enabled;
    return enqueue("enabled", () => enabled ? startCueOutput() : stopCueOutput());
  }, [enqueue]);
  const setMix = useCallback((value: number) => {
    if (Number.isFinite(value)) void enqueue("mix", () => setCueMix(Math.max(0, Math.min(1, value))));
  }, [enqueue]);
  const setVolume = useCallback((value: number) => {
    if (Number.isFinite(value)) void enqueue("volume", () => setCueVolume(Math.max(0, Math.min(1, value))));
  }, [enqueue]);
  const toggleDeckCue = useCallback((deck: number) => {
    if (!Number.isInteger(deck) || deck < 0 || deck > 3) return;
    const active = !desired.current.cued.has(deck);
    if (active) desired.current.cued.add(deck); else desired.current.cued.delete(deck);
    void enqueue(`deck${deck}`, () => setDeckCue(deck, active));
  }, [enqueue]);
  // Keep feedback dependencies stable when native snapshots repeat the same deck selection.
  const cueKey = [...state.cued].sort((a, b) => a - b).join(",");
  const cued = useMemo(() => new Set(cueKey ? cueKey.split(",").map(Number) : []), [cueKey]);
  return {
    devices: details.map((device) => device.name), details, device: state.device, setDevice,
    firstChannel: state.first_channel, setChannel, rescan,
    enabled: state.enabled, connectedName: state.connected_name, error: actionError?.message ?? state.error,
    toggle, mix: state.mix, setMix, volume: state.volume, setVolume, cued, toggleDeckCue,
  };
}
