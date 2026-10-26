const listeners = new Set<() => void>();
let listening = false;

export const listeningModeActive = () => listening;

export const setListeningMode = (active: boolean) => {
  if (listening === active) return;
  listening = active;
  listeners.forEach((listener) => listener());
};

export const subscribeListeningMode = (listener: () => void) => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};
