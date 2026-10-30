import { useEffect, useState } from "react";
import type { AuxApi } from "../hooks/useAux";
import type { BoothApi } from "../hooks/useBooth";
import type { CueApi } from "../hooks/useCue";
import type { InternalClockApi } from "../hooks/useInternalClock";
import { useMasterOutput } from "../hooks/useMasterOutput";
import { engineStatus, headphoneDelay, setHeadphoneDelay, type EngineStatus, exportDiagnostics, type KeyNotation, type OutputDeviceInfo } from "../lib/ipc";
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
  const speakers = useMasterOutput();
  const speakerStatus = speakers.switching
    ? "Switching…"
    : speakers.fallback
      ? `Using ${speakers.connectedName ?? "macOS default"}`
      : speakers.connectedName
        ? speakers.embeddedHeadphones
          ? `On: ${speakers.connectedName} · 1/2`
          : `On: ${speakers.connectedName}`
        : "Offline";
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
                speakers.rescan();
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
              <label className="settings-device-title" htmlFor="master-output-device"><Icon name="speaker" size={16} /><span>Speakers</span></label>
              <select
                id="master-output-device"
                value={speakers.device ?? ""}
                disabled={speakers.switching}
                onChange={(event) => void speakers.setDevice(event.target.value || null)}
              >
                <option value="">Same as the Mac</option>
                {speakers.device && !speakers.devices.some((device) => device.name === speakers.device) && (
                  <option value={speakers.device}>{speakers.device} (unavailable)</option>
                )}
                {speakers.devices.map((device) => (
                  <option key={device.name} value={device.name}>{speakerOptionLabel(device)}</option>
                ))}
              </select>
              <span className="settings-device-status">{speakerStatus}</span>
            </div>
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
            <HeadphoneDelayControl />
            <p className="ctrl-hint">Raise headphone delay until the cans line up with the speakers. The speaker path keeps its own delay.</p>
            <p className="ctrl-hint">Speakers follow the Mac’s current output until you pick another device. Choose DDJ-FLX4 to send the master mix to the controller’s outputs 1/2, with headphones on 3/4. MacBook speakers and other outputs are listed separately. Choose speakers before the set: switching restarts playback and clears the loaded decks.</p>
            {speakers.error && <p role="alert" className="ctrl-hint">{speakers.error}</p>}
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
        <AudioHealth headphonesEnabled={cue.enabled} />
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
  icon: "headphones" | "mic" | "sliders" | "speaker";
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

function speakerOptionLabel(device: OutputDeviceInfo): string {
  const controller = device.name.toLowerCase().includes("ddj-flx4");
  if (controller && device.channels >= 4) return `${device.name} · speakers 1/2, headphones 3/4`;
  if (device.channels >= 2) return `${device.name} · ${device.channels} channels`;
  return device.name;
}

function HeadphoneDelayControl() {
  const [milliseconds, setMilliseconds] = useState(0);
  useEffect(() => {
    let disposed = false;
    void headphoneDelay()
      .then((value) => {
        if (!disposed && Number.isFinite(value)) setMilliseconds(value);
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
    };
  }, []);
  return (
    <div className="settings-headphone-delay">
      <label className="settings-device-title" htmlFor="headphone-delay">Headphone delay</label>
      <input
        id="headphone-delay"
        type="range"
        min={0}
        max={200}
        step={1}
        value={milliseconds}
        onChange={(event) => {
          const next = Number(event.target.value);
          setMilliseconds(next);
          void setHeadphoneDelay(next);
        }}
      />
      <span className="settings-device-status">{milliseconds} ms</span>
    </div>
  );
}

function AudioHealth({ headphonesEnabled }: { headphonesEnabled: boolean }) {
  const [status, setStatus] = useState<EngineStatus | null>(null);
  const [error, setError] = useState(false);
  useEffect(() => {
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const next = await engineStatus();
        if (!disposed) { setStatus(next); setError(false); }
      } catch { if (!disposed) setError(true); }
      if (!disposed) timer = setTimeout(poll, 1000);
    };
    void poll();
    return () => { disposed = true; clearTimeout(timer); };
  }, []);
  return <details className="ctrl-hint" style={{ marginTop: 16 }}>
    <summary>Audio health</summary>
    {error ? <p role="alert">Audio status unavailable.</p> : status ? <>
      <p>{status.audio_online ? "Online" : "Offline"} · {status.master_device ?? "No speaker output"} · {status.sample_rate.toLocaleString()} Hz</p>
      {status.audio_error && <p role="alert">{status.audio_error}</p>}
      <p>Output restarts: {status.audio_restarts}. A restarted output requires tracks to be reloaded and routing checked.</p>
      <p>{!headphonesEnabled ? "Headphones off." : status.master_embedded_headphones ? "Headphones share the FLX4 output stream (3/4)." : `Separate headphone stream: ${status.cue_underruns} buffer shortages, ${status.cue_discarded_frames} discarded frames. Counts are since this headphone stream opened; increasing counts during playback need attention.`}</p>
      <p>Controller software delay: {status.controller_probe_count > 0
        ? `${(status.controller_probe_last_us / 1000).toFixed(1)} ms latest · ${(status.controller_probe_max_us / 1000).toFixed(1)} ms highest · ${status.controller_probe_count} samples`
        : "Move a channel fader or press a transport button to collect samples."}</p>
      <p>Measured from MIDI receipt to the audio callback. Excludes USB input, speaker latency, jog wheels and headphone controls; this is not a physical latency measurement.</p>
    </> : <p>Reading audio status…</p>}
  </details>;
}
