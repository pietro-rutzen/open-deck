import { useCallback, useEffect, useState } from "react";
import {
  inTauri,
  listOutputDeviceDetails,
  masterSession,
  setMasterOutput,
  type MasterSession,
  type OutputDeviceInfo,
} from "../lib/ipc";

export interface MasterOutputApi {
  devices: OutputDeviceInfo[];
  device: string | null;
  connectedName: string | null;
  embeddedHeadphones: boolean;
  fallback: boolean;
  switching: boolean;
  error: string | null;
  setDevice: (device: string | null) => Promise<void>;
  rescan: () => void;
}

export function useMasterOutput(): MasterOutputApi {
  const [devices, setDevices] = useState<OutputDeviceInfo[]>([]);
  const [session, setSession] = useState<MasterSession | null>(null);
  const [switching, setSwitching] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  const rescan = useCallback(() => {
    if (!inTauri()) return;
    listOutputDeviceDetails().then(setDevices).catch(() => {});
    masterSession()
      .then((next) => {
        setSession(next);
        setActionError(null);
      })
      .catch((reason: unknown) => setActionError(String(reason)));
  }, []);

  useEffect(() => {
    rescan();
  }, [rescan]);

  const setDevice = useCallback(async (device: string | null) => {
    if (!inTauri()) return;
    setSwitching(true);
    setActionError(null);
    try {
      setSession(await setMasterOutput(device));
    } catch (reason) {
      setActionError(String(reason));
    } finally {
      setSwitching(false);
    }
  }, []);

  return {
    devices,
    device: session?.requested_device ?? null,
    connectedName: session?.connected_device ?? null,
    embeddedHeadphones: session?.embedded_headphones ?? false,
    fallback: session?.fallback ?? false,
    switching,
    error: actionError ?? session?.error ?? null,
    setDevice,
    rescan,
  };
}
