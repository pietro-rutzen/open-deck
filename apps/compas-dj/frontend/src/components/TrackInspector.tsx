import { useEffect, useState, type KeyboardEvent } from "react";
import {
  formatKey,
  inTauri,
  trackFileFacts,
  type DbTrack,
  type KeyNotation,
  type TrackFileFacts,
} from "../lib/ipc";
import { readQueuedPaths } from "../lib/nextQueue";
import {
  fileKind,
  fileStem,
  formatBitrate,
  formatBytes,
  formatChannels,
  formatDuration,
  formatFileDate,
  formatGain,
  formatGridOffset,
  formatIndex,
  formatPlayedAt,
  formatSampleRate,
  parentFolderName,
  pendingFact,
  type FileFactStatus,
} from "../lib/trackFacts";
import { MiniWaveform, useTrackVisuals } from "./LibraryVisuals";

const PANES = [
  { id: "summary", label: "Summary" },
  { id: "info", label: "Info" },
  { id: "artwork", label: "Artwork" },
] as const;

type InspectorPane = (typeof PANES)[number]["id"];

const useTrackFileFacts = (path: string) => {
  const [result, setResult] = useState<{
    path: string;
    status: FileFactStatus;
    facts: TrackFileFacts | null;
  } | null>(null);
  useEffect(() => {
    if (!inTauri()) {
      setResult({ path, status: "unavailable", facts: null });
      return;
    }
    let current = true;
    trackFileFacts(path)
      .then((facts) => {
        if (current) setResult({ path, status: "ready", facts });
      })
      .catch(() => {
        if (current) setResult({ path, status: "unavailable", facts: null });
      });
    return () => {
      current = false;
    };
  }, [path]);
  return result?.path === path
    ? result
    : { status: "loading" as const, facts: null };
};

const FactRow = ({
  label,
  value,
  title,
  note = false,
}: {
  label: string;
  value: string;
  title?: string;
  note?: boolean;
}) => (
  <div className={note ? "inspector-note" : undefined}>
    <dt>{label}</dt>
    <dd title={title}>{value}</dd>
  </div>
);

const FactGroup = ({
  title,
  rows,
}: {
  title: string;
  rows: { label: string; value: string; title?: string; note?: boolean }[];
}) => {
  if (rows.length === 0) return null;
  return (
    <section className="inspector-group">
      <div className="overline">{title}</div>
      <dl className="inspector-rows">
        {rows.map((row) => (
          <FactRow key={row.label} {...row} />
        ))}
      </dl>
    </section>
  );
};

export const TrackInspector = ({
  track,
  keyNotation,
  showPlayCount,
  decks,
  loadedPaths,
  nextDecks,
  tagging,
  tagInput,
  onTagInput,
  onBeginTag,
  onCommitTag,
  onCancelTag,
  onDropTag,
  onLoad,
  onQueue,
  onRemove,
}: {
  track: DbTrack;
  keyNotation: KeyNotation;
  showPlayCount: boolean;
  decks: readonly { letter: string; color: string }[];
  loadedPaths: readonly (string | undefined)[];
  nextDecks: readonly { index: number; letter: string }[];
  tagging: boolean;
  tagInput: string;
  onTagInput: (value: string) => void;
  onBeginTag: () => void;
  onCommitTag: () => void;
  onCancelTag: () => void;
  onDropTag: (tag: string) => void;
  onLoad: (deck: number) => void;
  onQueue: (deck: number) => void;
  onRemove: () => void;
}) => {
  const [pane, setPane] = useState<InspectorPane>("summary");
  const visual = useTrackVisuals(track.path, true, true, 2);
  const file = useTrackFileFacts(track.path);
  const artwork = visual.data?.artwork_data_url ?? null;
  const title = visual.data?.title || track.title || fileStem(track.path);
  const artist = visual.data?.artist || track.artist || "Unknown artist";
  const album = visual.data?.album || file.facts?.album || null;
  const albumLabel =
    album ??
    (visual.status === "loading" || file.status === "loading" ? "…" : "Not tagged");
  const artworkLabel =
    visual.status === "loading"
      ? "Loading artwork…"
      : visual.status === "unavailable"
        ? "Artwork unavailable"
        : "No embedded artwork";
  const tempo = track.bpm ? `${track.bpm.toFixed(2)} BPM` : "Not analyzed";
  const key = track.key_camelot
    ? formatKey(track.key_camelot, track.key_name, keyNotation)
    : "Not analyzed";
  const created = file.facts?.created_ms;
  const modified = file.facts?.modified_ms;
  const summaryDate =
    created != null
      ? { label: "Created", value: formatFileDate(created) }
      : modified != null
        ? { label: "Modified", value: formatFileDate(modified) }
        : { label: "Modified", value: pendingFact(file.status, null) };
  const summaryRows = [
    { label: "Time", value: formatDuration(track.duration_ms) },
    { label: "Tempo", value: tempo },
    { label: "Key", value: key },
    { label: "Kind", value: fileKind(track.path) },
    {
      label: "Size",
      value: pendingFact(file.status, file.facts ? formatBytes(file.facts.size_bytes) : null),
    },
    {
      label: "Sample rate",
      value: pendingFact(
        file.status,
        file.facts?.sample_rate_hz != null ? formatSampleRate(file.facts.sample_rate_hz) : null,
      ),
    },
    { label: "Bitrate", value: formatBitrate(file.facts, track.duration_ms, file.status) },
    summaryDate,
    ...(showPlayCount ? [{ label: "Plays", value: String(track.play_count) }] : []),
    { label: "Location", value: parentFolderName(track.path), title: track.path },
  ];
  const trackIndex = formatIndex(
    file.facts?.track_number ?? null,
    file.facts?.track_total ?? null,
  );
  const discIndex = formatIndex(
    file.facts?.disc_number ?? null,
    file.facts?.disc_total ?? null,
  );
  const songRows = [
    { label: "Title", value: title },
    { label: "Artist", value: artist },
    { label: "Album", value: albumLabel },
    ...(file.facts?.album_artist ? [{ label: "Album artist", value: file.facts.album_artist }] : []),
    ...(file.facts?.genre ? [{ label: "Genre", value: file.facts.genre }] : []),
    ...(file.facts?.recorded_date ? [{ label: "Recorded", value: file.facts.recorded_date }] : []),
    ...(trackIndex ? [{ label: "Track", value: trackIndex }] : []),
    ...(discIndex ? [{ label: "Disc", value: discIndex }] : []),
  ];
  const creditRows = [
    ...(file.facts?.original_artist
      ? [{ label: "Original artist", value: file.facts.original_artist }]
      : []),
    ...(file.facts?.composer ? [{ label: "Composer", value: file.facts.composer }] : []),
    ...(file.facts?.lyricist ? [{ label: "Lyricist", value: file.facts.lyricist }] : []),
    ...(file.facts?.remixer ? [{ label: "Remixer", value: file.facts.remixer }] : []),
    ...(file.facts?.label ? [{ label: "Label", value: file.facts.label }] : []),
    ...(file.facts?.comment
      ? [{ label: "Comment", value: file.facts.comment, note: true }]
      : []),
  ];
  const gain = formatGain(track.gain);
  const gridOffset = formatGridOffset(track.grid_offset_sec);
  const performanceRows = [
    { label: "Tempo", value: tempo },
    { label: "Key", value: key },
    ...(showPlayCount ? [{ label: "Plays", value: String(track.play_count) }] : []),
    ...(track.last_played_at != null
      ? [{ label: "Last played", value: formatPlayedAt(track.last_played_at) }]
      : []),
    ...(gain ? [{ label: "Gain", value: gain }] : []),
    ...(gridOffset ? [{ label: "Beatgrid", value: gridOffset }] : []),
  ];
  const fileRows = [
    { label: "Kind", value: fileKind(track.path) },
    {
      label: "Size",
      value: pendingFact(file.status, file.facts ? formatBytes(file.facts.size_bytes) : null),
    },
    {
      label: "Sample rate",
      value: pendingFact(
        file.status,
        file.facts?.sample_rate_hz != null ? formatSampleRate(file.facts.sample_rate_hz) : null,
      ),
    },
    {
      label: "Channels",
      value: pendingFact(
        file.status,
        file.facts?.channels != null ? formatChannels(file.facts.channels) : null,
      ),
    },
    { label: "Bitrate", value: formatBitrate(file.facts, track.duration_ms, file.status) },
    ...(created != null ? [{ label: "Created", value: formatFileDate(created) }] : []),
    ...(modified != null ? [{ label: "Modified", value: formatFileDate(modified) }] : []),
    { label: "Location", value: track.path, note: true },
  ];
  const onTabKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const index = PANES.findIndex((item) => item.id === pane);
    const nextIndex =
      event.key === "ArrowRight"
        ? (index + 1) % PANES.length
        : event.key === "ArrowLeft"
          ? (index - 1 + PANES.length) % PANES.length
          : event.key === "Home"
            ? 0
            : event.key === "End"
              ? PANES.length - 1
              : null;
    if (nextIndex == null) return;
    event.preventDefault();
    const next = PANES[nextIndex];
    setPane(next.id);
    event.currentTarget.querySelectorAll("button")[nextIndex]?.focus();
  };

  return (
    <>
      <div
        className="inspector-segments"
        role="tablist"
        aria-label="Track details"
        onKeyDown={onTabKeyDown}
      >
        {PANES.map((item) => (
          <button
            key={item.id}
            type="button"
            role="tab"
            id={`inspector-tab-${item.id}`}
            aria-selected={pane === item.id}
            aria-controls={`inspector-panel-${item.id}`}
            tabIndex={pane === item.id ? 0 : -1}
            onClick={() => setPane(item.id)}
          >
            {item.label}
          </button>
        ))}
      </div>
      <div
        className="inspector-body"
        role="tabpanel"
        id={`inspector-panel-${pane}`}
        aria-labelledby={`inspector-tab-${pane}`}
      >
        {pane === "summary" && (
          <>
            <div className="inspector-identity">
              <Artwork art={artwork} title={title} label={artworkLabel} size="summary" />
              <div>
                <h3>{title}</h3>
                <p>{artist}</p>
                {album && <p className="inspector-album">{album}</p>}
              </div>
            </div>
            <dl className="inspector-rows">
              {summaryRows.map((row) => (
                <FactRow key={row.label} {...row} />
              ))}
            </dl>
            <div className="inspector-wave">
              {visual.data?.peaks.length ? (
                <MiniWaveform peaks={visual.data.peaks} bands={visual.data.band_peaks} />
              ) : (
                <span className="mini-waveform-state">
                  {visual.status === "loading" ? "Reading waveform…" : "Waveform unavailable"}
                </span>
              )}
            </div>
          </>
        )}
        {pane === "info" && (
          <>
            <FactGroup title="Song" rows={songRows} />
            <FactGroup title="Credits" rows={creditRows} />
            <FactGroup title="Performance" rows={performanceRows} />
            <section className="inspector-group">
              <div className="overline">Tags</div>
              <div className="inspector-tags">
                {track.tags.map((tag) => (
                  <button
                    className="chip"
                    key={tag}
                    type="button"
                    onClick={() => onDropTag(tag)}
                    aria-label={`Remove tag ${tag}`}
                  >
                    {tag} ×
                  </button>
                ))}
                {tagging ? (
                  <input
                    autoFocus
                    value={tagInput}
                    placeholder="New tag"
                    aria-label="New tag"
                    onChange={(event) => onTagInput(event.target.value)}
                    onBlur={onCommitTag}
                    onKeyDown={(event) => {
                      if (event.key === "Enter") onCommitTag();
                      else if (event.key === "Escape") onCancelTag();
                    }}
                  />
                ) : (
                  <button className="chip" type="button" onClick={onBeginTag}>
                    + Tag
                  </button>
                )}
              </div>
            </section>
            <FactGroup title="File" rows={fileRows} />
            <details className="inspector-remove">
              <summary>Library options</summary>
              <button className="chip" type="button" onClick={onRemove}>
                Remove from library
              </button>
              <p>The original file stays on disk.</p>
            </details>
          </>
        )}
        {pane === "artwork" && (
          <div className="inspector-artwork-pane">
            <Artwork art={artwork} title={title} label={artworkLabel} size="hero" />
            <p>{artwork ? "Embedded artwork" : artworkLabel}</p>
          </div>
        )}
      </div>
      <div className="inspector-dock">
        <div className="inspector-load" aria-label="Load selected track">
          {decks.map((deck, index) => (
            <button
              key={deck.letter}
              type="button"
              style={{ color: deck.color, borderColor: deck.color }}
              onClick={() => onLoad(index)}
              title={`Load ${title} onto Deck ${deck.letter}`}
            >
              Load {deck.letter}
            </button>
          ))}
        </div>
        {nextDecks.length > 0 && (
          <div className="inspector-actions">
            {nextDecks.map((deck) => {
              const loadedHere = loadedPaths[deck.index] === track.path;
              const queued = readQueuedPaths(deck.index).includes(track.path);
              return (
                <button
                  key={deck.letter}
                  type="button"
                  className="chip"
                  disabled={loadedHere || queued}
                  title={
                    loadedHere
                      ? `Already loaded on deck ${deck.letter}`
                      : queued
                        ? `Already in deck ${deck.letter}'s Next queue`
                        : `Add to deck ${deck.letter}'s Next queue, ahead of automatic suggestions`
                  }
                  onClick={() => onQueue(deck.index)}
                >
                  {queued ? `Queued ${deck.letter}` : `Queue ${deck.letter}`}
                </button>
              );
            })}
          </div>
        )}
      </div>
    </>
  );
};

const Artwork = ({
  art,
  title,
  label,
  size,
}: {
  art: string | null;
  title: string;
  label: string;
  size: "summary" | "hero";
}) => (
  <div className={`inspector-artwork inspector-artwork--${size}`}>
    {art ? (
      <img src={art} alt={`Cover art for ${title}`} />
    ) : (
      <div className="artwork-placeholder" role="img" aria-label={label}>
        <span aria-hidden="true">♫</span>
        {size === "hero" && <small>{label}</small>}
      </div>
    )}
  </div>
);
