/** Five-segment channel meter matching the FLX4 feedback thresholds.
 * Engine peak level is post channel gain, pre crossfader; 0 dBFS marks clipping risk. */
const SEGMENTS = [
  { db: 0, threshold: 1, color: "#ff453a", label: "0" },
  { db: -6, threshold: 0.5011872, color: "#ffae00", label: "−6" },
  { db: -12, threshold: 0.25118864, color: "#ffae00", label: "−12" },
  { db: -24, threshold: 0.06309573, color: "#39e58c", label: "−24" },
  { db: -36, threshold: 0.015848932, color: "#39e58c", label: "−36" },
];

export const METER_DB_LABELS = SEGMENTS.map((segment) => segment.label);

export function Meter({
  level,
  height,
  streaming = false,
  showClip = true,
  showLabels = true,
}: {
  level: number;
  height?: number;
  streaming?: boolean;
  showClip?: boolean;
  showLabels?: boolean;
}) {
  const peak = Number.isFinite(level) ? Math.max(0, level) : 0;
  const db = peak > 0 ? 20 * Math.log10(peak) : -60;
  return (
    <div className={`channel-meter ${showClip ? "" : "channel-meter--levels"}`} style={height == null ? undefined : { height }} role="meter"
      aria-label="Channel peak level" aria-valuemin={-60} aria-valuemax={0}
      aria-valuenow={Math.max(-60, Math.min(0, db))}
      aria-valuetext={peak === 0 ? "Silent" : `${db.toFixed(1)} dBFS${peak >= 1 ? ", clipping risk" : ""}`}
      title="Channel peak · dBFS · green: signal, amber: approaching maximum, red: 0 dBFS or higher">
      {showClip && <span className={`meter-clip ${peak >= 1 ? "meter-clip--on" : ""}`}>CLIP</span>}
      {SEGMENTS.map((segment) => <div className="meter-step" key={segment.db}>
        <span className="meter-segment" style={{ background: streaming ? "#58bccc" : segment.color, opacity: peak >= segment.threshold ? 1 : 0.12 }} />
        {showLabels && <span className="meter-db">{segment.label}</span>}
      </div>)}
    </div>
  );
}
