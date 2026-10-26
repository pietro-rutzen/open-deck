import { useSyncExternalStore } from "react";
import {
  controllerSession, inTauri, midiConnect, midiDisconnect, midiListPorts,
  onControllerSession, type ControllerSession,
} from "../lib/ipc";

/** One native MIDI session shared by every panel. React never opens ports automatically. */
export interface MidiApi {
  ports: string[];
  connected: string | null;
  portIdx: number;
  setPortIdx: (i: number) => void;
  rescan: () => void;
  toggle: () => Promise<void>;
}

type MidiSnapshot = Pick<MidiApi, "ports" | "connected" | "portIdx">;
let snapshot: MidiSnapshot = { ports: [], connected: null, portIdx: 0 };
let selectedName: string | null = null;
const subscribers = new Set<() => void>();
let started = false;
let sessionVersion = 0;
let snapshotRequest = 0;
let portsRequest = 0;
let toggling = false;

function publish(ports: string[], connected = snapshot.connected) {
  // A session event can precede enumeration. Never display an unrelated keyboard as connected.
  const visible = connected && !ports.includes(connected) ? [...ports, connected] : ports;
  const desired = connected ?? selectedName;
  const index = desired ? visible.indexOf(desired) : -1;
  snapshot = { ports: visible, connected, portIdx: index >= 0 ? index : 0 };
  subscribers.forEach((notify) => notify());
}

async function refreshPorts() {
  if (!inTauri()) return;
  const request = ++portsRequest;
  try {
    const ports = await midiListPorts();
    if (request === portsRequest) publish(ports);
  } catch { /* Retain the last authoritative connection when enumeration fails. */ }
}

function applySession(session: ControllerSession) {
  if (session.connected) selectedName = session.connected;
  publish(snapshot.ports, session.connected);
  void refreshPorts();
}

async function refreshSession() {
  if (!inTauri()) return;
  const version = sessionVersion;
  const request = ++snapshotRequest;
  try {
    const session = await controllerSession();
    // A later event or request always wins over an older in-flight command result.
    if (version === sessionVersion && request === snapshotRequest) applySession(session);
  } catch { /* A failed read is not evidence that the physical session disconnected. */ }
}

function startSession() {
  if (started || !inTauri()) return;
  started = true;
  // This single listener intentionally lives for the app lifetime, shared by all hook instances.
  void onControllerSession((session) => {
    sessionVersion++;
    applySession(session);
  }).then(() => refreshSession()).catch(() => {
    started = false;
    void refreshSession();
  });
  void refreshPorts();
}

function subscribe(notify: () => void) {
  subscribers.add(notify);
  startSession();
  return () => { subscribers.delete(notify); };
}
const getSnapshot = () => snapshot;
const setPortIdx = (index: number) => {
  if (snapshot.connected || !Number.isInteger(index) || index < 0 || index >= snapshot.ports.length) return;
  selectedName = snapshot.ports[index];
  publish(snapshot.ports);
};
const rescan = () => { void refreshPorts(); void refreshSession(); };
const toggle = async () => {
  if (!inTauri() || toggling) return;
  toggling = true;
  try {
    if (snapshot.connected) {
      await midiDisconnect();
    } else {
      const name = snapshot.ports[snapshot.portIdx];
      if (!name) return;
      // Resolve the selection by name against fresh enumeration, not an obsolete port index.
      const ports = await midiListPorts();
      const index = ports.indexOf(name);
      if (index >= 0) await midiConnect(index);
    }
  } catch { /* Read native state below; do not manufacture a disconnected session on failure. */ }
  finally {
    await refreshSession();
    toggling = false;
  }
};

export function useMidi(): MidiApi {
  const state = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  return { ...state, setPortIdx, rescan, toggle };
}
