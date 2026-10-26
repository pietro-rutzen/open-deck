import { forwardRef, memo, useCallback, useEffect, useImperativeHandle, useMemo, useRef, useState, type DragEvent } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { PhysicalPosition } from "@tauri-apps/api/dpi";
import {
  addWatchFolder,
  dbAddTag,
  dbRemoveTag,
  dbResetPlayCounts,
  inTauri,
  listWatchFolders,
  moveIntoFolder,
  onLibraryChanged,
  onLibraryPathsMoved,
  onControllerUpdate,
  pickFolder,
  removeWatchFolder,
  rescanWatchFolders,
  reorderFolderTracks,
  dbPlanNext,
  dbSearch,
  loadTrack,
  deckPlay,
  onDeckError,
  onDeckLoaded,
  formatKey,
  prioritizeAnalysis,
  type DbTrack,
  type KeyNotation,
} from "../lib/ipc";
import { parseLibrarySort, sortTracks, SORT_STORAGE_KEY, type LibrarySort, type SortColumn } from "../lib/librarySort";
import { ascendFolder, childFolders, compareFileNames, directTracks, folderCrumbs, folderPlayCounts, isInside, playProgressTone, reorderPaths } from "../lib/libraryFolders";
import { listeningModeActive, setListeningMode } from "../lib/listeningMode";
import { PaneDivider, useStoredSize } from "./WorkspaceSplit";
import { useLibrary } from "../hooks/useLibrary";
import { Icon } from "./icons";
import { MiniWaveform, TrackCells, useTrackVisuals } from "./LibraryVisuals";

const LIBRARY_ACCENT = "var(--accent)";

const pointerOverElement = (element: Element, position: PhysicalPosition) => {
  const point = position.toLogical(window.devicePixelRatio);
  const bounds = element.getBoundingClientRect();
  return point.x >= bounds.left && point.x <= bounds.right && point.y >= bounds.top && point.y <= bounds.bottom;
};

const REORDER_SCROLL_EDGE = 42;

const reorderScrollStep = (pointerY: number, body: HTMLElement) => {
  const bounds = body.getBoundingClientRect();
  const maxScroll = body.scrollHeight - body.clientHeight;
  const distanceFromTop = pointerY - bounds.top;
  if (distanceFromTop < REORDER_SCROLL_EDGE && body.scrollTop > 0) {
    const closeness = Math.min(1, (REORDER_SCROLL_EDGE - distanceFromTop) / REORDER_SCROLL_EDGE);
    return -Math.max(1, Math.round(closeness * 8));
  }
  const distanceFromBottom = bounds.bottom - pointerY;
  if (distanceFromBottom < REORDER_SCROLL_EDGE && body.scrollTop < maxScroll) {
    const closeness = Math.min(1, (REORDER_SCROLL_EDGE - distanceFromBottom) / REORDER_SCROLL_EDGE);
    return Math.max(1, Math.round(closeness * 8));
  }
  return 0;
};

const firstVisibleTrackPath = (body: HTMLElement) => {
  const top = body.getBoundingClientRect().top;
  for (const row of body.querySelectorAll<HTMLElement>("[data-track-path]")) {
    if (row.getBoundingClientRect().bottom > top + 1) return row.dataset.trackPath ?? null;
  }
  return null;
};

const trackPathAtReorderPoint = (body: HTMLElement, x: number, y: number) => {
  const hit = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-track-path]");
  if (hit?.dataset.trackPath) return hit.dataset.trackPath;
  if (y <= body.getBoundingClientRect().top + REORDER_SCROLL_EDGE) return firstVisibleTrackPath(body);
  return null;
};
// Per-deck load targets (must match App's DECK_COLORS / DECK_LETTERS).
const DECKS = [
  { letter: "A", color: "var(--deck-a)" },
  { letter: "B", color: "var(--deck-b)" },
  { letter: "C", color: "var(--deck-c)" },
  { letter: "D", color: "var(--deck-d)" },
];

function fmtMs(ms: number): string {
  const s = Math.round(ms / 1000);
  return `${Math.floor(s / 60)}:${(s % 60).toString().padStart(2, "0")}`;
}

const playCountClass = (played: number, total: number) => {
  const tone = playProgressTone(played, total);
  return tone === "quiet" ? "play-count" : `play-count play-count--${tone}`;
};

/** `loadedPaths[0]` = Deck A's file path, `[1]` = Deck B's — for A/B row tags. */
export const Library = memo(forwardRef<
  HTMLElement,
  {
    loadedPaths: (string | undefined)[];
    keyNotation?: KeyNotation;
    focusTarget?: "library" | "crates" | null;
    focusSeq?: number;
  }
>(function Library({ loadedPaths, keyNotation = "camelot", focusTarget = null, focusSeq = 0 }, ref) {
  const lib = useLibrary();
  const [q, setQ] = useState("");
  const [sort, setSort] = useState<LibrarySort>(() => {
    try { return parseLibrarySort(localStorage.getItem(SORT_STORAGE_KEY)); }
    catch { return parseLibrarySort(null); }
  });
  useEffect(() => {
    try { localStorage.setItem(SORT_STORAGE_KEY, JSON.stringify(sort)); } catch { /* Sorting still works without persistence. */ }
  }, [sort]);
  const changeSort = (column: SortColumn) => setSort((previous) => ({
    column,
    direction: previous.column === column && previous.direction === "asc" ? "desc" : "asc",
  }));
  const sortHeader = (column: SortColumn, label: string) => {
    const active = sort.column === column;
    const next = active && sort.direction === "asc" ? "descending" : "ascending";
    const current = active ? `, sorted ${sort.direction === "asc" ? "ascending" : "descending"}` : "";
    const spokenLabel = column === "order" ? "Original order" : label;
    return <button type="button" className={`tl-sort${active ? " tl-sort--active" : ""}${column === "bpm" ? " tl-sort--bpm" : ""}`}
      onClick={() => changeSort(column)} aria-label={`${spokenLabel}${current}. Sort ${next}`}
      title={column === "order" ? (canReorder ? "Folder order. Drag a track onto another to rearrange the files." : browsingFolder ? "Folder order. Turn on Reorder before dragging tracks into a new order." : "Original list order") : `Sort by ${label.toLowerCase()}`}>
      <span>{label}</span><span className="tl-sort-arrow" aria-hidden="true">{active ? sort.direction === "asc" ? "↑" : "↓" : ""}</span>
    </button>;
  };
  const rootRef = useRef<HTMLElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const [cratesPulse, setCratesPulse] = useState(false);
  useImperativeHandle(ref, () => rootRef.current as HTMLElement);
  // Search results (engine grammar) and a "suggest next" override; null = show the full library.
  const [results, setResults] = useState<DbTrack[] | null>(null);
  const [suggestFor, setSuggestFor] = useState<string | null>(null);
  const [planPath, setPlanPath] = useState<string | null>(null);
  const [queue, setQueue] = useState<DbTrack[]>([]);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  useEffect(() => {
    if (selectedPath && inTauri()) prioritizeAnalysis(selectedPath).catch(() => {});
  }, [selectedPath]);
  const inspectorRef = useRef<HTMLElement>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const viewGeneration = useRef(0);
  const openedInitialFolder = useRef(false);
  const reportError = (error: unknown) => setActionError(String(error));
  const load = async (deck: number, path: string) => {
    setActionError(null);
    try { await loadTrack(deck, path); return true; }
    catch (error) { reportError(error); return false; }
  };
  const playOnDeckA = useRef<string | null>(null);
  useEffect(() => {
    if (!inTauri()) return;
    let alive = true;
    let stopLoaded: (() => void) | undefined;
    let stopError: (() => void) | undefined;
    onDeckLoaded((event) => {
      if (event.deck !== 0 || playOnDeckA.current !== event.path) return;
      playOnDeckA.current = null;
      deckPlay(0).catch((error) => reportError(error));
    }).then((unlisten) => {
      if (alive) stopLoaded = unlisten;
      else unlisten();
    }).catch(() => {});
    onDeckError((event) => {
      if (event.deck === 0) playOnDeckA.current = null;
    }).then((unlisten) => {
      if (alive) stopError = unlisten;
      else unlisten();
    }).catch(() => {});
    return () => {
      alive = false;
      stopLoaded?.();
      stopError?.();
    };
  }, []);

  // Debounced search via the engine query grammar (bpm:120-128 key:8A artist:foo -live); falls
  // back to a client-side title/artist filter outside Tauri.
  useEffect(() => {
    const generation = ++viewGeneration.current;
    const query = q.trim();
    if (!inTauri()) {
      setResults(query ? lib.tracks.filter((t) => `${t.title} ${t.artist}`.toLowerCase().includes(query.toLowerCase())) : null);
      return;
    }
    const nextPath = planPath;
    if (!query && !nextPath) { setResults(null); return; }
    const id = setTimeout(() => {
      const request = query ? dbSearch(query) : dbPlanNext(nextPath ?? "", 12);
      request.then((tracks) => { if (viewGeneration.current === generation) setResults(tracks); }).catch((error) => { if (viewGeneration.current === generation) reportError(error); });
    }, query ? 180 : 0);
    return () => { clearTimeout(id); if (viewGeneration.current === generation) viewGeneration.current++; };
  }, [q, planPath, lib.tracks]);

  const queueTrack = (t: DbTrack) => {
    setQueue((cur) => (cur.some((q) => q.path === t.path) ? cur : [...cur, t]));
  };
  const loadNextQueued = async () => {
    const [next] = queue;
    if (!next) return;
    const emptyDeck = loadedPaths.findIndex((p) => !p);
    const targetDeck = emptyDeck >= 0 ? emptyDeck : 1;
    if (await load(targetDeck, next.path)) setQueue((cur) => cur.filter((t) => t.path !== next.path));
  };

  // Auto-mix planner: ranked next-track suggestions after `t`.
  const suggestNext = (t: DbTrack) => {
    setQ("");
    setSuggestFor(t.title);
    setPlanPath(t.path);
  };
  const [folderPath, setFolderPath] = useState<string | null>(null);
  const [reorderArmed, setReorderArmed] = useState(false);
  const [listening, setListening] = useState(listeningModeActive);
  const [listeningBusy, setListeningBusy] = useState(false);
  const [sourcesWidth, setSourcesWidth] = useStoredSize("opendeck.library.sources.v1", 186, 140, 420);
  const [inspectorWidth, setInspectorWidth] = useStoredSize("opendeck.library.inspector.v1", 238, 180, 520);
  const [fileDropActive, setFileDropActive] = useState(false);
  const folderPathRef = useRef(folderPath);
  folderPathRef.current = folderPath;
  const [dropKey, setDropKey] = useState<string | null>(null);
  const [reordering, setReordering] = useState(false);
  const draggingPath = useRef<string | null>(null);
  const movedTo = useRef(new Map<string, string>());
  const clearView = () => {
    openedInitialFolder.current = true;
    viewGeneration.current++;
    setFolderPath(null);
    setResults(null);
    setSuggestFor(null);
    setPlanPath(null);
    setQ("");
  };
  const openFolder = (path: string) => {
    openedInitialFolder.current = true;
    viewGeneration.current++;
    setFolderPath(path);
    setResults(null);
    setSuggestFor(null);
    setPlanPath(null);
    setQ("");
    setSelectedPath(null);
  };

  // Tagging: an inline tag editor opens for one track row at a time.
  const [taggingPath, setTaggingPath] = useState<string | null>(null);
  const [tagInput, setTagInput] = useState("");
  // Re-pull whatever view is showing so a tag edit is reflected immediately.
  const reloadView = lib.refresh;
  const commitTag = (path: string) => {
    const t = tagInput.trim();
    setTaggingPath(null);
    setTagInput("");
    if (t) dbAddTag(path, t).then(reloadView).catch(() => {});
  };
  const dropTag = (path: string, tag: string) => {
    dbRemoveTag(path, tag).then(reloadView).catch(() => {});
  };

  // Watched folders (auto-import).
  const [folders, setFolders] = useState<string[]>([]);
  const [scanning, setScanning] = useState(false);
  const refreshFolders = useCallback(() => {
    if (!inTauri()) return;
    listWatchFolders().then(setFolders).catch(() => {});
  }, []);
  useEffect(() => refreshFolders(), [refreshFolders]);
  useEffect(() => {
    if (openedInitialFolder.current || folders.length !== 1) return;
    openedInitialFolder.current = true;
    setFolderPath(folders[0]);
  }, [folders]);
  // A background scan (on launch or add) imported tracks → refresh the library + folder list.
  useEffect(() => {
    if (!inTauri()) return;
    const un = onLibraryChanged(() => {
      lib.refresh();
      refreshFolders();
    });
    return () => {
      un.then((u) => u());
    };
  }, [lib.refresh, refreshFolders]);
  useEffect(() => {
    if (!inTauri()) return;
    let stopListening: (() => void) | undefined;
    let alive = true;
    let finderPaths: string[] = [];
    getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === "enter") finderPaths = event.payload.paths;
      if (event.payload.type === "leave") {
        finderPaths = [];
        setFileDropActive(false);
        return;
      }
      if (finderPaths.length === 0) return;
      const list = rootRef.current?.querySelector(".tracklist");
      const hovering = list != null && pointerOverElement(list, event.payload.position);
      if (event.payload.type === "over") {
        setFileDropActive(hovering && folderPathRef.current != null);
        return;
      }
      if (event.payload.type !== "drop") return;
      const paths = finderPaths;
      finderPaths = [];
      setFileDropActive(false);
      if (!hovering) return;
      const folder = folderPathRef.current;
      if (!folder) {
        setActionError("Open a folder before dropping files into it");
        return;
      }
      moveIntoFolder(folder, paths)
        .then(() => setActionError(null))
        .catch((error: unknown) => setActionError(String(error)));
    }).then((stop) => {
      if (alive) stopListening = stop;
      else stop();
    }).catch((error: unknown) => setActionError(String(error)));
    return () => {
      alive = false;
      stopListening?.();
    };
  }, []);
  useEffect(() => {
    if (!inTauri()) return;
    const un = onLibraryPathsMoved((moves) => {
      const moved = new Map(moves.map((move) => [move.from, move.to]));
      movedTo.current = moved;
      setSelectedPath((path) => (path && moved.get(path)) || path);
      setPlanPath((path) => (path && moved.get(path)) || path);
      setQueue((current) => current.map((track) => {
        const to = moved.get(track.path);
        return to ? { ...track, path: to } : track;
      }));
    });
    return () => { un.then((stop) => stop()); };
  }, []);
  const addFolder = async () => {
    const dir = await pickFolder().catch(() => null);
    if (!dir) return;
    setScanning(true);
    try {
      await addWatchFolder(dir);
      refreshFolders();
      lib.refresh();
      reloadView();
      openFolder(dir);
    } catch (error) { reportError(error); } finally {
      setScanning(false);
    }
  };
  const rescanFolders = async () => {
    setScanning(true);
    try {
      await rescanWatchFolders();
      lib.refresh();
      reloadView();
    } catch (error) { reportError(error); } finally {
      setScanning(false);
    }
  };
  const dropFolder = (dir: string) => {
    removeWatchFolder(dir).then(() => {
      refreshFolders();
      if (folderPath && isInside(dir, folderPath)) setFolderPath(null);
    }).catch(() => {});
  };
  const baseName = (p: string) => p.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || p;

  const sourceTracks = results ?? lib.tracks;
  const filtered = useMemo(() => sortTracks(sourceTracks, sort), [sourceTracks, sort]);
  const browsingFolder = Boolean(folderPath) && !q.trim() && !planPath;
  const folderEntries = useMemo(
    () => (browsingFolder && folderPath ? childFolders(lib.tracks, folderPath) : []),
    [browsingFolder, folderPath, lib.tracks],
  );
  const folderTracks = useMemo(() => {
    if (!browsingFolder || !folderPath) return filtered;
    const direct = directTracks(lib.tracks, folderPath);
    if (sort.column !== "order") return sortTracks(direct, sort);
    const byName = [...direct].sort((left, right) => compareFileNames(baseName(left.path), baseName(right.path)) || left.path.localeCompare(right.path));
    return sort.direction === "asc" ? byName : byName.reverse();
  }, [browsingFolder, folderPath, lib.tracks, sort, filtered]);
  useEffect(() => {
    if (!browsingFolder) setReorderArmed(false);
  }, [browsingFolder]);
  const canReorder = browsingFolder && reorderArmed && !reordering;
  const reorderTrack = (from: string, to: string) => {
    if (!canReorder) return;
    const next = reorderPaths(folderTracks.map((track) => track.path), from, to);
    if (!next) return;
    setReordering(true);
    setActionError(null);
    reorderFolderTracks(next)
      .then(() => setSort({ column: "order", direction: "asc" }))
      .catch((error) => reportError(error))
      .finally(() => { setReordering(false); setDropKey(null); });
  };
  const reorderPointer = useRef<{ x: number; y: number } | null>(null);
  const reorderScrollFrame = useRef<number | null>(null);
  const stopReorderScroll = () => {
    if (reorderScrollFrame.current != null) cancelAnimationFrame(reorderScrollFrame.current);
    reorderScrollFrame.current = null;
  };
  const scrollReorderList = () => {
    const pointer = reorderPointer.current;
    const body = rootRef.current?.querySelector<HTMLElement>(".tl-body");
    if (!pointer || !body || !draggingPath.current || !canReorder) {
      reorderScrollFrame.current = null;
      return;
    }
    const step = reorderScrollStep(pointer.y, body);
    const before = body.scrollTop;
    if (step !== 0) body.scrollTop = before + step;
    const target = trackPathAtReorderPoint(body, pointer.x, pointer.y);
    if (target && target !== draggingPath.current) {
      const nextKey = `track:${target}`;
      setDropKey((current) => (current === nextKey ? current : nextKey));
    }
    if (step === 0 || body.scrollTop === before) {
      reorderScrollFrame.current = null;
      return;
    }
    reorderScrollFrame.current = requestAnimationFrame(scrollReorderList);
  };
  const noteReorderPointer = (event: { clientX: number; clientY: number }) => {
    if (!canReorder || !draggingPath.current) return;
    reorderPointer.current = { x: event.clientX, y: event.clientY };
    const body = rootRef.current?.querySelector<HTMLElement>(".tl-body");
    if (!body || reorderScrollStep(event.clientY, body) === 0 || reorderScrollFrame.current != null) return;
    reorderScrollFrame.current = requestAnimationFrame(scrollReorderList);
  };
  const dropReorderAtPoint = (event: DragEvent<HTMLElement>) => {
    if (!canReorder || !draggingPath.current) return;
    event.preventDefault();
    const from = event.dataTransfer.getData("application/x-compas-track") || draggingPath.current;
    const body = rootRef.current?.querySelector<HTMLElement>(".tl-body");
    const target = body ? trackPathAtReorderPoint(body, event.clientX, event.clientY) : null;
    stopReorderScroll();
    setDropKey(null);
    if (from && target && from !== target) reorderTrack(from, target);
  };
  const libraryCounts = useMemo(() => folderPlayCounts(lib.tracks, null), [lib.tracks]);
  const viewCounts = useMemo(
    () => (browsingFolder && folderPath ? folderPlayCounts(lib.tracks, folderPath) : libraryCounts),
    [browsingFolder, folderPath, lib.tracks, libraryCounts],
  );
  const resetPlayed = () => {
    dbResetPlayCounts(browsingFolder ? folderPath : null).catch((error) => reportError(error));
  };
  const toggleListening = async () => {
    if (listening) {
      setListeningMode(false);
      setListening(false);
      return;
    }
    setListeningBusy(true);
    try {
      if (inTauri()) await dbResetPlayCounts(null);
      setListeningMode(true);
      setListening(true);
    } catch (error) {
      reportError(error);
    } finally {
      setListeningBusy(false);
    }
  };
  const playedLabel = (played: number, total: number) => (listening ? String(total) : `${played}/${total}`);
  const playedClass = (played: number, total: number) => (listening ? "" : playCountClass(played, total));
  type BrowseRow =
    | { kind: "up"; path: string | null; name: string; key: string }
    | { kind: "folder"; path: string; name: string; trackCount: number; playedCount: number; key: string }
    | { kind: "track"; track: DbTrack; key: string };
  const browseRows = useMemo<BrowseRow[]>(() => {
    if (!browsingFolder || !folderPath) return folderTracks.map((track) => ({ kind: "track", track, key: `track:${track.path}` }));
    const parent = ascendFolder(folderPath, folders);
    const up: BrowseRow = parent
      ? { kind: "up", path: parent, name: baseName(parent), key: `up:${parent}` }
      : { kind: "up", path: null, name: "All tracks", key: "up:library" };
    return [
      up,
      ...folderEntries.map((entry) => ({ kind: "folder" as const, ...entry, key: `folder:${entry.path}` })),
      ...folderTracks.map((track) => ({ kind: "track" as const, track, key: `track:${track.path}` })),
    ];
  }, [browsingFolder, folderPath, folders, folderEntries, folderTracks]);
  const [browseKey, setBrowseKey] = useState<string | null>(null);
  const selected = folderTracks.find((track) => track.path === selectedPath) ?? null;
  const selectedBrowse = browseRows.find((row) => row.key === browseKey) ?? null;
  const visual = useTrackVisuals(selected?.path ?? null);
  const selectedVisual = visual.status === "ready" ? visual.data : undefined;
  useEffect(() => {
    if (!selectedPath) return;
    if (folderTracks.some((track) => track.path === selectedPath)) {
      movedTo.current.clear();
      return;
    }
    if ([...movedTo.current.values()].includes(selectedPath)) return;
    setSelectedPath(null);
  }, [folderTracks, selectedPath]);
  const browseIndexRef = useRef(-1);
  const sourceIndexRef = useRef(0);
  const browseRowsRef = useRef(browseRows);
  browseRowsRef.current = browseRows;
  useEffect(() => {
    browseIndexRef.current = browseRows.findIndex((row) => row.key === browseKey || (row.kind === "track" && row.track.path === selectedPath));
  }, [browseRows, browseKey, selectedPath]);
  useEffect(() => {
    if (folderPath) sourceIndexRef.current = 1 + Math.max(0, folders.findIndex((folder) => isInside(folder, folderPath)));
    else sourceIndexRef.current = 0;
  }, [folderPath, folders]);
  const revealRow = (key: string) => requestAnimationFrame(() => {
    const row = rootRef.current?.querySelector<HTMLElement>(`[data-browse-key="${CSS.escape(key)}"]`);
    row?.focus();
    row?.scrollIntoView({ block: "nearest" });
  });
  const selectBrowse = (index: number, focus = false) => {
    const row = browseRows[index];
    if (!row) return;
    browseIndexRef.current = index;
    setBrowseKey(row.key);
    setSelectedPath(row.kind === "track" ? row.track.path : null);
    if (focus) revealRow(row.key);
  };
  const enterBrowse = (row: BrowseRow) => {
    if (row.kind === "track") return;
    if (row.path) openFolder(row.path);
    else clearView();
  };


  useEffect(() => {
    if (!focusTarget) return;
    rootRef.current?.scrollIntoView({ block: "nearest" });
    if (focusTarget === "library") {
      searchRef.current?.focus();
      searchRef.current?.select();
      return;
    }
    setCratesPulse(true);
    const id = window.setTimeout(() => setCratesPulse(false), 850);
    return () => window.clearTimeout(id);
  }, [focusSeq, focusTarget]);
  const [browseTarget, setBrowseTarget] = useState<"tracks" | "sources">("tracks");
  const browseSources = (steps: number) => {
    const lastSourceIndex = folders.length;
    const nextSourceIndex = Math.max(0, Math.min(lastSourceIndex, sourceIndexRef.current + steps));
    if (nextSourceIndex === sourceIndexRef.current) return;
    sourceIndexRef.current = nextSourceIndex;
    if (nextSourceIndex === 0) clearView();
    else openFolder(folders[nextSourceIndex - 1]);
  };
  const hardwareRef = useRef<(control: string, value: number) => void>(() => {});
  hardwareRef.current = (control, value) => {
    if (control === "library.focus" && value > 0) {
      const row = browseRowsRef.current[browseIndexRef.current];
      if (browseTarget === "tracks" && row && row.kind !== "track") {
        enterBrowse(row);
        return;
      }
      setBrowseTarget((target) => (target === "tracks" ? "sources" : "tracks"));
    }
    if (control === "library.browse" && value !== 0) {
      if (browseTarget === "sources") browseSources(Math.trunc(value));
      else {
        const rows = browseRowsRef.current;
        const next = Math.max(0, Math.min(rows.length - 1, browseIndexRef.current + Math.trunc(value)));
        selectBrowse(next, true);
      }
    }
    const target = control.match(/^library\.load\.([01])$/);
    const row = browseRowsRef.current[browseIndexRef.current];
    if (target && row?.kind === "track" && value > 0) load(Number(target[1]), row.track.path);
  };
  useEffect(() => {
    if (!inTauri()) return;
    const un = onControllerUpdate(u => hardwareRef.current(u.control, u.value));
    return () => { un.then(fn => fn()); };
  }, []);


  return (
    <section className="library" ref={rootRef}>
      <aside className={`sources ${cratesPulse || browseTarget === "sources" ? "sources--focus" : ""}`} style={{ width: sourcesWidth, flex: "none" }}>
        <div className="overline src-group">SOURCES</div>
        <button className={`src-row ${!folderPath ? "src-row--active" : ""}`} onClick={clearView}>
          <span className="src-dot" style={{ background: LIBRARY_ACCENT }} />
          <span className="src-name">Local Library</span>
          <span className={`ctrl-tag ${playedClass(libraryCounts.played, libraryCounts.total)}`}>
            {playedLabel(libraryCounts.played, libraryCounts.total)}
          </span>
        </button>

        <div className="overline src-group src-group--crates">
          FOLDERS
          <span style={{ display: "flex", gap: 4 }}>
            <button className="crate-add" onClick={rescanFolders} disabled={scanning || folders.length === 0} title="Re-scan watched folders for new files">⟳</button>
            <button className="crate-add" onClick={addFolder} disabled={scanning} title="Watch a folder — auto-imports its audio files">＋</button>
          </span>
        </div>
        {folders.length === 0 && (
          <div className="src-row src-row--muted"><span className="src-name">{scanning ? "Scanning…" : "No watched folders"}</span></div>
        )}
        {folders.map((f) => {
          const counts = folderPlayCounts(lib.tracks, f);
          return (
          <div
            key={f}
            className={`src-row ${folderPath && isInside(f, folderPath) ? "src-row--active" : ""}`}
            title={f}
            onClick={() => openFolder(f)}
            role="button"
            tabIndex={0}
            onKeyDown={(e) => { if (e.target !== e.currentTarget) return; if (e.key === "Enter" || e.key === " ") { e.preventDefault(); openFolder(f); } }}
          >
            <Icon name="folder" size={13} />
            <span className="src-name">{baseName(f)}</span>
            <span className={`mono src-play-count ${playedClass(counts.played, counts.total)}`}>{playedLabel(counts.played, counts.total)}</span>
            <button className="crate-add" onClick={(e) => { e.stopPropagation(); dropFolder(f); }} title="Stop watching this folder">✕</button>
          </div>
          );
        })}
      </aside>
      <PaneDivider label="Resize folders" value={sourcesWidth} min={140} max={420} fallback={186} grow={1} onChange={setSourcesWidth} />

      <div className={`tracklist${fileDropActive ? " tracklist--filedrop" : ""}`}>
        <div className="tl-toolbar">
          <div className="search">
            <Icon name="search" size={14} />
            <input
              ref={searchRef}
              value={q}
              onChange={(e) => { setQ(e.target.value); setPlanPath(null); setSuggestFor(null); }}
              placeholder="Search tracks, artists, BPM or key"
              aria-label="Search library"
              title="Grammar: bpm:120-128 (range), key:8A, artist:/title: (fuzzy), bare word = title or artist, - to exclude. Terms AND together; 'OR' (or '|') starts a new group, e.g. artist:daft OR artist:justice"
            />
          </div>
          <button className="add-btn" onClick={lib.add} disabled={lib.busy}>
            {lib.busy ? "Adding…" : "+ Add tracks"}
          </button>
          <button className="add-btn" onClick={loadNextQueued} disabled={queue.length === 0}>
            {queue.length ? `Load next (${queue.length})` : "Queue empty"}
          </button>
          {browsingFolder && folderPath && (
            <nav className="tl-crumbs" aria-label="Folder">
              {folderCrumbs(folderPath, folders).map((crumb, crumbIndex, crumbs) => (
                <span key={crumb.path} className="tl-crumb">
                  {crumbIndex > 0 && <span aria-hidden="true">/</span>}
                  <button type="button" aria-current={crumbIndex === crumbs.length - 1 ? "page" : undefined} onClick={() => openFolder(crumb.path)}>{crumb.name}</button>
                </span>
              ))}
            </nav>
          )}
          <span className={`mono tl-count ${playedClass(viewCounts.played, viewCounts.total)}`} title={listening ? "Tracks in this view" : "Played tracks / tracks in this view"}>{playedLabel(viewCounts.played, viewCounts.total)}</span>
          <button
            type="button"
            className={`add-btn${listening ? " add-btn--on" : ""}`}
            aria-pressed={listening}
            disabled={listeningBusy}
            title={listening ? "Return to performance mode. Played marks count again, and deck A stops auto queue and play." : "Reset played marks, ignore them on folders and the list, and turn on deck A queue and play."}
            onClick={() => { void toggleListening(); }}
          >
            {listening ? "Performance mode" : "Listening mode"}
          </button>
          <button
            type="button"
            className={`add-btn${canReorder ? " add-btn--on" : ""}`}
            aria-pressed={canReorder}
            disabled={!folderPath || !browsingFolder || reordering}
            title={canReorder ? "Reorder is on. Drag a track onto another row. Click again to drag onto decks only." : folderPath ? "Turn on to rearrange this folder. Off, dragging a track only loads a deck." : "Open a folder to rearrange its tracks."}
            onClick={() => {
              stopReorderScroll();
              setDropKey(null);
              setReorderArmed((armed) => !armed);
            }}
          >
            Reorder
          </button>
          <button className="add-btn" onClick={resetPlayed} disabled={viewCounts.played === 0} title={browsingFolder ? "Clear played marks in this folder" : "Clear played marks for the whole library"}>Reset played</button>
        </div>

        {queue.length > 0 && (
          <div className="tl-banner tl-banner--queue">
            <span>
              <strong>Play queue</strong>
              {" · "}
              {queue.slice(0, 4).map((t) => t.title).join(" → ")}
              {queue.length > 4 ? ` +${queue.length - 4}` : ""}
            </span>
            <button className="chip" onClick={loadNextQueued}>load next</button>
            <button className="chip" onClick={() => setQueue([])}>clear</button>
          </div>
        )}

        {suggestFor && (
          <div className="tl-banner">
            <span>✨ Suggested next after <strong>{suggestFor}</strong> (harmonic + tempo)</span>
            <button className="chip" onClick={clearView}>clear</button>
          </div>
        )}

        {actionError && <div className="tl-banner" role="alert"><span>{actionError}</span><button className="chip" onClick={() => setActionError(null)}>Dismiss</button></div>}
        <div className="tl-head tl-grid" aria-label="Sort tracks"
          onDragOver={(event) => { if (!canReorder || !draggingPath.current) return; event.preventDefault(); event.dataTransfer.dropEffect = "move"; noteReorderPointer(event); }}
          onDrop={dropReorderAtPoint}>
          {sortHeader("order", "#")}{sortHeader("title", "Title")}{sortHeader("artist", "Artist")}{sortHeader("bpm", "BPM")}{sortHeader("key", "Key")}
          <span className="tl-spacer" aria-hidden="true" />
          <span>Waveform</span>{sortHeader("duration", "Time")}
        </div>
        <div className="tl-body" role="listbox" aria-label="Tracks" aria-describedby="library-keyboard-help" tabIndex={browseRows.length ? -1 : 0}
          onDragOver={(event) => { if (!canReorder || !draggingPath.current) return; event.preventDefault(); noteReorderPointer(event); }}
          onDrop={dropReorderAtPoint}
          onKeyDown={(e) => { if (e.key === "Backspace" && browsingFolder) { e.preventDefault(); const parent = folderPath ? ascendFolder(folderPath, folders) : null; if (parent) openFolder(parent); else clearView(); } }}>
          {browseRows.length === 0 ? (
            <div className="tl-empty">
              {q.trim() ? "No matching tracks." : "Add local tracks to start your collection."}
              <p>Select a track to view its details and load it onto a deck.</p>
            </div>
          ) : browseRows.map((row, index) => {
            const selectedRow = row.key === browseKey || (row.kind === "track" && row.track.path === selectedPath);
            const move = (next: number) => selectBrowse(Math.max(0, Math.min(browseRows.length - 1, next)), true);
            if (row.kind !== "track") {
              return (
                <div
                  key={row.key}
                  className={`tl-row tl-row--folder ${selectedRow ? "tl-row--selected" : ""}`}
                  role="option"
                  aria-selected={selectedRow}
                  aria-label={row.kind === "up" ? `Back to ${row.name}` : `${row.name}, ${row.playedCount} of ${row.trackCount} played`}
                  tabIndex={selectedRow || (!browseKey && index === 0) ? 0 : -1}
                  data-browse-key={row.key}
                  onClick={() => { selectBrowse(index); enterBrowse(row); }}
                  onKeyDown={(e) => {
                    if (e.key === "ArrowDown") { e.preventDefault(); move(index + 1); }
                    else if (e.key === "ArrowUp") { e.preventDefault(); move(index - 1); }
                    else if (e.key === "Home") { e.preventDefault(); move(0); }
                    else if (e.key === "End") { e.preventDefault(); move(browseRows.length - 1); }
                    else if (e.key === "Enter") { e.preventDefault(); enterBrowse(row); }
                  }}
                >
                  <Icon name={row.kind === "up" && row.path == null ? "list" : "folder"} size={14} />
                  <span className="tl-folder-name">{row.name}</span>
                  <span className={`mono tl-folder-count ${row.kind === "folder" ? playedClass(row.playedCount, row.trackCount) : ""}`}>{row.kind === "folder" ? playedLabel(row.playedCount, row.trackCount) : ""}</span>
                </div>
              );
            }
            const t = row.track;
            const loadedDecks = DECKS.filter((_, i) => loadedPaths[i] === t.path);
            const loadedAs = loadedDecks[0];
            const trackNumber = browseRows.slice(0, index).filter((item) => item.kind === "track").length + 1;
            return (
              <div
                key={row.key}
                className={`tl-row tl-grid ${!listening && t.play_count > 0 ? "tl-row--played" : ""} ${selectedRow ? "tl-row--selected" : ""} ${canReorder ? "tl-row--reorder" : ""} ${dropKey === row.key ? "tl-row--drop" : ""}`}
                style={loadedAs ? { borderLeftColor: loadedAs.color } : undefined}
                role="option"
                aria-selected={selectedRow}
                aria-label={`${t.title}, ${t.artist || "Unknown artist"}${t.bpm != null && t.bpm > 0 ? `, ${t.bpm.toFixed(2)} BPM` : ""}${t.key_camelot ? `, ${formatKey(t.key_camelot, t.key_name, keyNotation)}` : ""}${loadedDecks.length ? `, loaded on deck ${loadedDecks.map((d) => d.letter).join(", ")}` : ""}`}
                tabIndex={selectedRow || (!selected && !browseKey && index === 0) ? 0 : -1}
                data-browse-key={row.key}
                data-track-index={index}
                data-track-path={t.path}
                draggable
                onDragStart={(e) => {
                  draggingPath.current = t.path;
                  e.dataTransfer.setData("application/x-compas-track", t.path);
                  e.dataTransfer.setData("text/plain", t.path);
                  e.dataTransfer.effectAllowed = "copyMove";
                }}
                onDragEnd={() => { draggingPath.current = null; stopReorderScroll(); setDropKey(null); }}
                onDragEnter={canReorder ? (e) => {
                  const from = draggingPath.current;
                  if (!from || from === t.path) return;
                  e.preventDefault();
                  e.dataTransfer.dropEffect = "move";
                } : undefined}
                onDragOver={canReorder ? (e) => {
                  const from = draggingPath.current;
                  if (!from || from === t.path) return;
                  e.preventDefault();
                  e.dataTransfer.dropEffect = "move";
                  if (dropKey !== row.key) setDropKey(row.key);
                } : undefined}
                onDrop={canReorder ? (e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  const from = e.dataTransfer.getData("application/x-compas-track") || draggingPath.current;
                  stopReorderScroll();
                  setDropKey(null);
                  if (from && from !== t.path) reorderTrack(from, t.path);
                } : undefined}
                onClick={() => selectBrowse(index)}
                onFocus={() => { setBrowseKey(row.key); setSelectedPath(t.path); }}
                onDoubleClick={() => {
                  selectBrowse(index);
                  if (listening) {
                    playOnDeckA.current = t.path;
                    void load(0, t.path).then((loaded) => {
                      if (!loaded && playOnDeckA.current === t.path) playOnDeckA.current = null;
                    });
                    return;
                  }
                  inspectorRef.current?.focus();
                }}
                onKeyDown={(e) => {
                  if (e.key === "ArrowDown") { e.preventDefault(); move(index + 1); }
                  else if (e.key === "ArrowUp") { e.preventDefault(); move(index - 1); }
                  else if (e.key === "Home") { e.preventDefault(); move(0); }
                  else if (e.key === "End") { e.preventDefault(); move(browseRows.length - 1); }
                  else if (e.key === "Enter") { e.preventDefault(); inspectorRef.current?.focus(); }
                }}
                title={canReorder ? "Drag onto the row this track should take. Files in the folder are renamed to match." : listening ? "Double-click loads this track on deck A and starts it" : "Select for details · Enter opens inspector · drag onto a deck to load"}
              >
                <span className="tl-tag" style={loadedAs ? { color: loadedAs.color } : undefined}>{loadedDecks.length ? loadedDecks.map((d) => d.letter).join("·") : String(trackNumber).padStart(2, "0")}</span>
                <TrackCells
                  path={t.path}
                  title={t.title || baseName(t.path)}
                  artist={t.artist}
                  color={loadedAs?.color}
                  afterArtist={<span className="mono tl-bpm" title={t.bpm == null ? "Queued for background BPM analysis; select to prioritize" : t.bpm > 0 ? "Original tempo" : "No tempo detected"}>{t.bpm == null ? "…" : t.bpm > 0 ? t.bpm.toFixed(2) : "—"}</span>}
                  beforeWave={<span className="mono tl-key" title={t.key_name ?? undefined}>{t.key_camelot ? formatKey(t.key_camelot, t.key_name, keyNotation) : "—"}</span>}
                />
                <span className="mono">{fmtMs(t.duration_ms)}</span>
              </div>
            );
          })}
        </div>
        <div className="tl-footer" id="library-keyboard-help">↑ ↓ Select · Enter opens a folder · Browse press enters · Load sends a track{folderPath ? " · Drop audio files here to move them into this folder" : ""}</div>
      </div>
      <PaneDivider label="Resize track details" value={inspectorWidth} min={180} max={520} fallback={238} grow={-1} onChange={setInspectorWidth} />
      <aside className="track-inspector" ref={inspectorRef} tabIndex={-1} aria-label="Track details" style={{ width: inspectorWidth, flex: "none" }}>
        {selected ? <>
          <div className="overline inspector-label">Track details</div>
          <div className="inspector-artwork">
            {selectedVisual?.artwork_data_url ? <img src={selectedVisual.artwork_data_url} alt={`Cover art for ${selected.title}`} /> : <div className="artwork-placeholder"><span aria-hidden="true">♫</span><small>{visual.status === "loading" ? "Loading artwork…" : visual.status === "unavailable" ? "Artwork unavailable" : "No embedded artwork"}</small></div>}
          </div>
          <div className="inspector-heading"><h3>{selectedVisual?.title || selected.title || baseName(selected.path)}</h3><p>{selectedVisual?.artist || selected.artist || "Unknown artist"}</p>{selectedVisual?.album && <p className="inspector-album">{selectedVisual.album}</p>}</div>
          <div className="inspector-wave">{selectedVisual?.peaks.length ? <MiniWaveform peaks={selectedVisual.peaks} bands={selectedVisual.band_peaks} /> : <span className="mini-waveform-state">{visual.status === "loading" ? "Reading waveform…" : "Waveform unavailable"}</span>}</div>
          <dl className="inspector-meta">
            <div><dt>Tempo</dt><dd>{selected.bpm ? `${selected.bpm.toFixed(2)} BPM` : "Not analyzed"}</dd></div>
            <div><dt>Key</dt><dd>{selected.key_camelot ? formatKey(selected.key_camelot, selected.key_name, keyNotation) : "Not analyzed"}</dd></div>
            <div><dt>Duration</dt><dd>{fmtMs(selected.duration_ms)}</dd></div>
            {!listening && <div><dt>Plays</dt><dd>{selected.play_count}</dd></div>}
            <div><dt>Format</dt><dd>{baseName(selected.path).includes(".") ? baseName(selected.path).split(".").pop()?.toUpperCase() : "Unknown"}</dd></div>
          </dl>
          <div className="inspector-load" aria-label="Load selected track">
            {DECKS.map((deck, i) => <button key={deck.letter} style={{ color: deck.color, borderColor: deck.color }} onClick={() => load(i, selected.path)} title={`Load ${selected.title} onto Deck ${deck.letter}`}>Load {deck.letter}</button>)}
          </div>
          <div className="inspector-actions">
            <button className="chip" onClick={() => queueTrack(selected)}>Add to queue</button>
            <button className="chip" onClick={() => suggestNext(selected)}>Find compatible</button>
          </div>
          <div className="inspector-tags">
            <div className="overline">Tags</div>
            {selected.tags.map((tag) => <button className="chip" key={tag} onClick={() => dropTag(selected.path, tag)} aria-label={`Remove tag ${tag}`}>{tag} ×</button>)}
            {taggingPath === selected.path ? <input autoFocus value={tagInput} placeholder="New tag" aria-label="New tag" onChange={(e) => setTagInput(e.target.value)} onBlur={() => commitTag(selected.path)} onKeyDown={(e) => { if (e.key === "Enter") commitTag(selected.path); else if (e.key === "Escape") { setTaggingPath(null); setTagInput(""); } }} /> : <button className="chip" onClick={() => { setTaggingPath(selected.path); setTagInput(""); }}>+ Tag</button>}
          </div>
          <div className="inspector-file"><div className="overline">File</div><p title={selected.path}>{selected.path}</p></div>
          <details className="inspector-remove"><summary>Library options</summary><button className="chip" onClick={() => { lib.remove(selected.path); setSelectedPath(null); }}>Remove from library</button><p>The original file stays on disk.</p></details>
        </> : selectedBrowse?.kind === "folder" ? <div className="inspector-empty"><Icon name="folder" size={28} /><h3>{selectedBrowse.name}</h3><p>{selectedBrowse.trackCount} tracks in this folder.</p></div> : <div className="inspector-empty"><span aria-hidden="true">♫</span><h3>Your music, in detail</h3><p>Select a track to see artwork, waveform and file information.</p></div>}
      </aside>
    </section>
  );
}));
