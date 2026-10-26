import type { AuxApi } from "../hooks/useAux";
import type { BoothApi } from "../hooks/useBooth";
import type { CueApi } from "../hooks/useCue";
import type { InternalClockApi } from "../hooks/useInternalClock";
import { exportDiagnostics, type KeyNotation } from "../lib/ipc";
import { Aux, Booth, InternalClock } from "./Mixer";
import { Icon } from "./icons";

export function SettingsPanel({
  aux,
  booth,
  cue,
  clock,
  contrast,
  onToggleContrast,
  keyNotation,
  onToggleKeyNotation,
  deckCount,
  onToggleDeckCount,
  onOpenKeys,
  onOpenMap,
  onOpenPads,
  onOpenControllers,
  onClose,
}: {
  aux: AuxApi;
  booth: BoothApi;
  cue: CueApi;
  clock: InternalClockApi;
  contrast: boolean;
  onToggleContrast: () => void;
  keyNotation: KeyNotation;
  onToggleKeyNotation: () => void;
  deckCount: 2 | 4;
  onToggleDeckCount: () => void;
  onOpenKeys: () => void;
  onOpenMap: () => void;
  onOpenPads: () => void;
  onOpenControllers: () => void;
  onClose: () => void;
}) {
  return (
    <div className="panel-overlay" onClick={onClose}>
      <div className="panel settings-panel" onClick={(e) => e.stopPropagation()}>
        <div className="panel-head">
          <h3>Settings</h3>
          <button className="chip" onClick={onClose}>Close</button>
        </div>
        <section className="settings-audio" aria-labelledby="settings-audio-heading">
          <div className="settings-section-head">
            <h4 id="settings-audio-heading">Audio devices</h4>
            <button
              className="chip settings-rescan"
              onClick={() => {
                aux.rescan();
                cue.rescan();
                booth.rescan();
              }}
            >
              Rescan
            </button>
          </div>
          <div className="settings-device-list">
            <div className="settings-device-row">
              <label className="settings-device-title" htmlFor="cue-output-device"><Icon name="headphones" size={16} /><span>Headphones</span></label>
              <select id="cue-output-device" value={cue.device ?? ""} disabled={cue.enabled}
                onChange={(event) => cue.setDevice(event.target.value || null)}>
                <option value="" disabled>Choose headphone output…</option>
                {cue.device && !cue.details.some((device) => device.name === cue.device) &&
                  <option value={cue.device}>{cue.device} (unavailable)</option>}
                {cue.details.map((device) => <option key={device.name} value={device.name} disabled={device.channels < 2}>
                  {device.name} · {device.channels} output channels
                </option>)}
              </select>
              <span className="settings-device-status">{cue.enabled ? `On: ${cue.connectedName ?? cue.device}` : "Off"}</span>
            </div>
            <div className="settings-device-row">
              <label className="settings-device-title" htmlFor="cue-output-pair">Headphone channels</label>
              <select id="cue-output-pair" value={cue.firstChannel} disabled={cue.enabled || !cue.device}
                onChange={(event) => cue.setChannel(Number(event.target.value))}>
                {(() => {
                  const channels = cue.details.find((device) => device.name === cue.device)?.channels ?? 0;
                  const pairs = Array.from({ length: Math.floor(channels / 2) }, (_, index) => index * 2);
                  return <>
                    {!pairs.includes(cue.firstChannel) && <option value={cue.firstChannel}>{cue.firstChannel + 1} / {cue.firstChannel + 2} (unavailable)</option>}
                    {pairs.map((first) => <option key={first} value={first}>{first + 1} / {first + 2}</option>)}
                  </>;
                })()}
              </select>
              <button className={`chip ${cue.enabled ? "chip--on" : ""}`} disabled={!cue.device} onClick={cue.toggle}>
                {cue.enabled ? "Stop headphones" : "Start headphones"}
              </button>
            </div>
            <p className="ctrl-hint">Channel numbers refer to the selected audio device. Master playback stays on the system output. Select a separate headphone pair or a separate device.</p>
            {cue.error && <p role="alert" className="ctrl-hint">{cue.error}</p>}
            <DevicePicker
              icon="sliders"
              label="Booth"
              value={booth.device}
              devices={booth.devices}
              defaultLabel="Default output"
              disabled={booth.enabled}
              status={booth.enabled ? `On: ${booth.connectedName ?? "selected output"}` : "Off"}
              onChange={booth.setDevice}
            />
            <DevicePicker
              icon="mic"
              label="Aux input"
              value={aux.device}
              devices={aux.devices}
              defaultLabel="Default input"
              disabled={aux.enabled}
              status={aux.enabled ? `On: ${aux.connectedName ?? "selected input"}` : "Off"}
              onChange={aux.setDevice}
            />
          </div>
        </section>
        <section className="settings-audio" aria-labelledby="settings-io-heading">
          <div className="settings-section-head">
            <h4 id="settings-io-heading">Input &amp; monitoring</h4>
          </div>
          <div className="settings-io-list">
            <Booth booth={booth} />
            <Aux aux={aux} />
            <InternalClock clock={clock} />
          </div>
        </section>
        <div className="settings-grid">
          <button className={`settings-tile ${contrast ? "settings-tile--on" : ""}`} onClick={onToggleContrast}>
            <Icon name="sun" size={18} />
            <span>High contrast</span>
          </button>
          <button className="settings-tile" onClick={onToggleKeyNotation}>
            <Icon name="music" size={18} />
            <span>Key: {keyNotation === "camelot" ? "Camelot (8A)" : "Musical (C#m)"}</span>
          </button>
          <button className="settings-tile" onClick={onToggleDeckCount}>
            <Icon name="sliders" size={18} />
            <span>Decks: {deckCount === 2 ? "2" : "4"}</span>
          </button>
          <button className="settings-tile" onClick={onOpenKeys}>
            <Icon name="music" size={18} />
            <span>Synth</span>
          </button>
          <button className="settings-tile" onClick={onOpenMap}>
            <Icon name="sliders" size={18} />
            <span>MIDI map</span>
          </button>
          <button className="settings-tile" onClick={onOpenPads}>
            <Icon name="pads" size={18} />
            <span>Sampler</span>
          </button>
          <button className="settings-tile" onClick={onOpenControllers}>
            <Icon name="sliders" size={18} />
            <span>Controllers</span>
          </button>
          <button
            className="settings-tile"
            onClick={() => void exportDiagnostics({ keyNotation, deckCount, highContrast: contrast })}
            title="Save a diagnostics bundle (.zip) for a bug report — no audio"
          >
            <Icon name="sliders" size={18} />
            <span>Diagnostics</span>
          </button>
        </div>
      </div>
    </div>
  );
}

function DevicePicker({
  icon,
  label,
  value,
  devices,
  defaultLabel,
  disabled,
  status,
  onChange,
}: {
  icon: "headphones" | "mic" | "sliders";
  label: string;
  value: string | null;
  devices: string[];
  defaultLabel: string;
  disabled: boolean;
  status: string;
  onChange: (name: string | null) => void;
}) {
  return (
    <label className="settings-device-row">
      <span className="settings-device-title">
        <Icon name={icon} size={16} />
        <span>{label}</span>
      </span>
      <select
        value={value ?? ""}
        onChange={(e) => onChange(e.target.value || null)}
        disabled={disabled}
      >
        <option value="">{defaultLabel}</option>
        {devices.map((device) => (
          <option key={device} value={device}>{device}</option>
        ))}
      </select>
      <span className="settings-device-status">{status}</span>
    </label>
  );
}
