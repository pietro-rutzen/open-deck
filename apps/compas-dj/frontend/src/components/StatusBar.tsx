type AudioStatus = {
  audio_online: boolean;
  audio_restarting: boolean;
  audio_restarts: number;
  audio_error: string | null;
  library_notice: string | null;
  master_device: string | null;
  master_fallback: boolean;
  master_embedded_headphones: boolean;
  cue_device_latency_secs: number;
  cue_prime_latency_secs: number;
  booth_device_latency_secs: number;
  booth_prime_latency_secs: number;
};

export function StatusBar({ sampleRate, audioStatus, beatFxLabel }: { sampleRate: number | null; audioStatus: AudioStatus | null; beatFxLabel?: string }) {
  const ok = sampleRate != null && sampleRate > 0 && (audioStatus?.audio_online ?? true);
  const restarting = audioStatus?.audio_restarting ?? false;
  const fallback = audioStatus?.master_fallback ?? false;
  const label = restarting ? "Audio restarting" : !ok ? "No audio device" : fallback ? "Speaker fallback" : "Engine OK";
  const output = audioStatus?.master_embedded_headphones
    ? `${audioStatus.master_device ?? "Speakers"} 1/2 · phones 3/4`
    : audioStatus?.master_device ?? "Speakers";
  const ms = (v?: number) => `${(((v ?? 0) * 1000)).toFixed(1)} ms`;
  const latencyTitle = audioStatus
    ? `Cue ${ms(audioStatus.cue_device_latency_secs)} + ${ms(audioStatus.cue_prime_latency_secs)} buffer · Booth ${ms(audioStatus.booth_device_latency_secs)} + ${ms(audioStatus.booth_prime_latency_secs)} buffer`
    : undefined;
  const title = audioStatus?.audio_error || (audioStatus?.audio_restarts ? `Recovered ${audioStatus.audio_restarts} time(s) · ${latencyTitle}` : latencyTitle);
  return (
    <footer className="statusbar mono">
      <span className="st-engine" title={title}>
        <span className={`st-dot ${restarting || fallback ? "st-dot--warn" : ok ? "st-dot--ok" : "st-dot--off"}`} />
        {label}
      </span>
      <span className="st-sep">·</span>
      <span>{ok ? `${(sampleRate / 1000).toFixed(1)} kHz` : "— kHz"}</span>
      <span className="st-sep">·</span>
      <span className="st-rt" title={title}>{output}</span>
      {audioStatus?.library_notice && (
        <>
          <span className="st-sep">·</span>
          <span className="st-notice" title={audioStatus.library_notice}>{audioStatus.library_notice}</span>
        </>
      )}
      {beatFxLabel && (
        <>
          <span className="st-sep">·</span>
          <span title="Beat FX selection driven by the controller">Beat FX {beatFxLabel}</span>
        </>
      )}
      <span className="st-right">
        <span>Open Deck · Prototype</span>
      </span>
    </footer>
  );
}
