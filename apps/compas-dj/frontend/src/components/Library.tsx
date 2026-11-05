import {
  forwardRef,
  memo,
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
  type DragEvent,
  type MouseEvent,
} from 'react';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import type { PhysicalPosition } from '@tauri-apps/api/dpi';
import { ask } from '@tauri-apps/plugin-dialog';
import {
  addWatchFolder,
  dbAddTag,
  dbRemoveTag,
  dbResetPlayCounts,
  deleteLibraryFolder,
  inTauri,
  listWatchFolders,
  moveIntoFolder,
  renameLibraryFolder,
  onLibraryChanged,
  onLibraryPathsMoved,
  onControllerUpdate,
  pickFolder,
  removeFromFolder,
  removeWatchFolder,
  rescanWatchFolders,
  reorderFolderTracks,
  loadTrack,
  deckPlay,
  onDeckError,
  onDeckLoaded,
  formatKey,
  prioritizeAnalysis,
  type DbTrack,
  type FolderEdit,
  type KeyNotation,
} from '../lib/ipc';
import {
  parseLibrarySort,
  sortTracks,
  SORT_STORAGE_KEY,
  type LibrarySort,
  type SortColumn,
} from '../lib/librarySort';
import {
  ascendFolder,
  childFolders,
  directoryOf,
  directTracks,
  folderAfterDelete,
  folderCrumbs,
  folderOrderComparator,
  folderOrdersAfterDelete,
  folderOrdersAfterRename,
  folderPlayCounts,
  isInside,
  retargetFolderPath,
  playProgressTone,
  readFolderOrders,
  reorderPaths,
  writeFolderOrders,
} from '../lib/libraryFolders';
import { controllerLoadPath, selectedControllerRow, stepControllerRow } from '../lib/libraryController';
import { filterLibraryTracks } from '../lib/librarySearch';
import { listeningModeActive, setListeningMode } from '../lib/listeningMode';
import { enqueueOnDeck, forgetQueuedPaths, readQueuedPaths, retargetNextQueuePaths, subscribeNextQueue } from '../lib/nextQueue';
import { PaneDivider, useStoredSize } from './WorkspaceSplit';
import { FolderNamePrompt, TrackMenu, type TrackMenuItem } from './TrackMenu';
import { useLibrary } from '../hooks/useLibrary';
import { Icon } from './icons';
import { TrackCells } from './LibraryVisuals';
import { TrackInspector } from './TrackInspector';

const LIBRARY_ACCENT = 'var(--accent)';
const VIEW_FOLDER_KEY = 'opendeck.library.folder.v1';
const PLAYING_FOLDER_KEY = 'opendeck.library.playingFolder.v1';

const readStoredFolder = (key: string) => {
  try {
    const value = localStorage.getItem(key);
    return value && value.trim() && value !== 'library' ? value : null;
  } catch {
    return null;
  }
};

const readStoredView = (): 'unset' | 'library' | string => {
  try {
    const value = localStorage.getItem(VIEW_FOLDER_KEY);
    if (value == null) return 'unset';
    if (value === '' || value === 'library') return 'library';
    return value;
  } catch {
    return 'unset';
  }
};

const storedFolderPath = () => {
  const view = readStoredView();
  return view !== 'unset' && view !== 'library' ? view : null;
};

const retargetStoredFolder = (key: string, from: string, to: string | null) => {
  try {
    const value = localStorage.getItem(key);
    if (!value || value === 'library') return;
    if (to == null) {
      if (isInside(from, value)) localStorage.removeItem(key);
      return;
    }
    const next = retargetFolderPath(value, from, to);
    if (next !== value) localStorage.setItem(key, next);
  } catch {
    /* The open folder still updates on screen. */
  }
};

const retargetBrowseKey = (key: string | null, from: string, to: string | null) => {
  if (!key) return key;
  const marker = key.startsWith('folder:')
    ? 'folder:'
    : key.startsWith('up:')
      ? 'up:'
      : null;
  if (!marker) return key;
  const path = key.slice(marker.length);
  if (to == null) return isInside(from, path) ? null : key;
  const next = retargetFolderPath(path, from, to);
  return next === path ? key : `${marker}${next}`;
};

const pointerOverElement = (element: Element, position: PhysicalPosition) => {
  const point = position.toLogical(window.devicePixelRatio);
  const bounds = element.getBoundingClientRect();
  return (
    point.x >= bounds.left &&
    point.x <= bounds.right &&
    point.y >= bounds.top &&
    point.y <= bounds.bottom
  );
};

const REORDER_SCROLL_EDGE = 42;

const reorderScrollStep = (pointerY: number, body: HTMLElement) => {
  const bounds = body.getBoundingClientRect();
  const maxScroll = body.scrollHeight - body.clientHeight;
  const distanceFromTop = pointerY - bounds.top;
  if (distanceFromTop < REORDER_SCROLL_EDGE && body.scrollTop > 0) {
    const closeness = Math.min(
      1,
      (REORDER_SCROLL_EDGE - distanceFromTop) / REORDER_SCROLL_EDGE,
    );
    return -Math.max(1, Math.round(closeness * 8));
  }
  const distanceFromBottom = bounds.bottom - pointerY;
  if (distanceFromBottom < REORDER_SCROLL_EDGE && body.scrollTop < maxScroll) {
    const closeness = Math.min(
      1,
      (REORDER_SCROLL_EDGE - distanceFromBottom) / REORDER_SCROLL_EDGE,
    );
    return Math.max(1, Math.round(closeness * 8));
  }
  return 0;
};

const firstVisibleTrackPath = (body: HTMLElement) => {
  const top = body.getBoundingClientRect().top;
  for (const row of body.querySelectorAll<HTMLElement>('[data-track-path]')) {
    if (row.getBoundingClientRect().bottom > top + 1)
      return row.dataset.trackPath ?? null;
  }
  return null;
};

const trackPathAtReorderPoint = (body: HTMLElement, x: number, y: number) => {
  const hit = document
    .elementFromPoint(x, y)
    ?.closest<HTMLElement>('[data-track-path]');
  if (hit?.dataset.trackPath) return hit.dataset.trackPath;
  if (y <= body.getBoundingClientRect().top + REORDER_SCROLL_EDGE)
    return firstVisibleTrackPath(body);
  return null;
};
// Per-deck load targets (must match App's DECK_COLORS / DECK_LETTERS).
const DECKS = [
  { letter: 'A', color: 'var(--deck-a)' },
  { letter: 'B', color: 'var(--deck-b)' },
  { letter: 'C', color: 'var(--deck-c)' },
  { letter: 'D', color: 'var(--deck-d)' },
];

function fmtMs(ms: number): string {
  const s = Math.round(ms / 1000);
  return `${Math.floor(s / 60)}:${(s % 60).toString().padStart(2, '0')}`;
}

const playCountClass = (played: number, total: number) => {
  const tone = playProgressTone(played, total);
  return tone === 'quiet' ? 'play-count' : `play-count play-count--${tone}`;
};

/** `loadedPaths[0]` = Deck A's file path, `[1]` = Deck B's — for A/B row tags. */
export const Library = memo(
  forwardRef<
    HTMLElement,
    {
      loadedPaths: (string | undefined)[];
      nextDecks: readonly { index: number; letter: string }[];
      keyNotation?: KeyNotation;
      focusTarget?: 'library' | 'crates' | null;
      focusSeq?: number;
    }
  >(function Library(
    { loadedPaths, nextDecks, keyNotation = 'camelot', focusTarget = null, focusSeq = 0 },
    ref,
  ) {
    const lib = useLibrary();
    const [q, setQ] = useState('');
    const [sort, setSort] = useState<LibrarySort>(() => {
      try {
        return parseLibrarySort(localStorage.getItem(SORT_STORAGE_KEY));
      } catch {
        return parseLibrarySort(null);
      }
    });
    useEffect(() => {
      try {
        localStorage.setItem(SORT_STORAGE_KEY, JSON.stringify(sort));
      } catch {
        /* Sorting still works without persistence. */
      }
    }, [sort]);
    const changeSort = (column: SortColumn) =>
      setSort((previous) => ({
        column,
        direction:
          previous.column === column && previous.direction === 'asc'
            ? 'desc'
            : 'asc',
      }));
    const sortHeader = (column: SortColumn, label: string) => {
      const active = sort.column === column;
      const next =
        active && sort.direction === 'asc' ? 'descending' : 'ascending';
      const current = active
        ? `, sorted ${sort.direction === 'asc' ? 'ascending' : 'descending'}`
        : '';
      const spokenLabel = column === 'order' ? 'Original order' : label;
      return (
        <button
          type="button"
          className={`tl-sort${active ? ' tl-sort--active' : ''}${column === 'bpm' ? ' tl-sort--bpm' : ''}`}
          onClick={() => changeSort(column)}
          aria-label={`${spokenLabel}${current}. Sort ${next}`}
          title={
            column === 'order'
              ? canReorder
                ? 'Folder order. Drag a track onto another. Reordering edits filenames.'
                : browsingFolder
                  ? 'Folder order. Turn on Reorder before dragging tracks into a new order. Reordering edits filenames.'
                  : 'Original list order'
              : `Sort by ${label.toLowerCase()}`
          }
        >
          <span>{label}</span>
          <span className="tl-sort-arrow" aria-hidden="true">
            {active ? (sort.direction === 'asc' ? '↑' : '↓') : ''}
          </span>
        </button>
      );
    };
    const rootRef = useRef<HTMLElement>(null);
    const searchRef = useRef<HTMLInputElement>(null);
    const [cratesPulse, setCratesPulse] = useState(false);
    useImperativeHandle(ref, () => rootRef.current as HTMLElement);
    // Search results (engine grammar) and a "suggest next" override; null = show the full library.
    const [results, setResults] = useState<DbTrack[] | null>(null);
    // Both the controller and the mouse own the same row identity. Update the ref
    // synchronously so several MIDI events before React renders cannot reuse an old index.
    const [browseKey, storeBrowseKey] = useState<string | null>(null);
    const selectionKeyRef = useRef<string | null>(null);
    const setBrowseKey = (next: string | null | ((key: string | null) => string | null)) => {
      const key = typeof next === 'function' ? next(selectionKeyRef.current) : next;
      selectionKeyRef.current = key;
      storeBrowseKey(key);
    };
    const selectedPath = browseKey?.startsWith('track:') ? browseKey.slice(6) : null;
    const setSelectedPath = (next: string | null | ((path: string | null) => string | null)) => {
      const key = selectionKeyRef.current;
      const current = key?.startsWith('track:') ? key.slice(6) : null;
      const path = typeof next === 'function' ? next(current) : next;
      if (typeof next === 'function' && path === current) return;
      setBrowseKey(path == null ? null : `track:${path}`);
    };
    const [, setNextQueueVersion] = useState(0);
    useEffect(() => subscribeNextQueue(() => setNextQueueVersion((version) => version + 1)), []);
    useEffect(() => {
      if (selectedPath && inTauri())
        prioritizeAnalysis(selectedPath).catch(() => {});
    }, [selectedPath]);
    const inspectorRef = useRef<HTMLElement>(null);
    const [actionError, setActionError] = useState<string | null>(null);
    const [trackMenu, setTrackMenu] = useState<{
      track: DbTrack;
      x: number;
      y: number;
    } | null>(null);
    const closeTrackMenu = useCallback(() => setTrackMenu(null), []);
    const [folderMenu, setFolderMenu] = useState<{
      path: string;
      name: string;
      x: number;
      y: number;
    } | null>(null);
    const [folderRename, setFolderRename] = useState<{
      path: string;
      name: string;
    } | null>(null);
    const [folderRenameError, setFolderRenameError] = useState<string | null>(
      null,
    );
    const folderEditBusy = useRef(false);
    const [folderRenameBusy, setFolderRenameBusy] = useState(false);
    const closeFolderMenu = useCallback(() => setFolderMenu(null), []);
    const openedInitialFolder = useRef(readStoredView() !== 'unset');
    const reportError = (error: unknown) => setActionError(String(error));
    const load = async (deck: number, path: string) => {
      setActionError(null);
      try {
        await loadTrack(deck, path);
        return true;
      } catch (error) {
        reportError(error);
        return false;
      }
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
      })
        .then((unlisten) => {
          if (alive) stopLoaded = unlisten;
          else unlisten();
        })
        .catch(() => {});
      onDeckError((event) => {
        if (event.deck === 0) playOnDeckA.current = null;
      })
        .then((unlisten) => {
          if (alive) stopError = unlisten;
          else unlisten();
        })
        .catch(() => {});
      return () => {
        alive = false;
        stopLoaded?.();
        stopError?.();
      };
    }, []);

    useEffect(() => {
      const query = q.trim();
      setResults(query ? filterLibraryTracks(lib.tracks, query) : null);
    }, [q, lib.tracks]);

    const queueTrack = (deck: number, path: string) => {
      if (loadedPaths[deck] === path) return;
      enqueueOnDeck(deck, path);
    };
    const [folderPath, setFolderPath] = useState<string | null>(
      storedFolderPath,
    );
    const [folderOrders, setFolderOrders] =
      useState<Record<string, string[]>>(readFolderOrders);
    const rememberFolderOrder = (folder: string, paths: string[]) => {
      setFolderOrders((current) => {
        const next = { ...current, [folder]: paths };
        writeFolderOrders(next);
        return next;
      });
    };
    const [reorderArmed, setReorderArmed] = useState(false);
    const [listening, setListening] = useState(listeningModeActive);
    const [listeningBusy, setListeningBusy] = useState(false);
    const [sourcesWidth, setSourcesWidth] = useStoredSize(
      'opendeck.library.sources.v2',
      260,
      640,
    );
    const [inspectorWidth, setInspectorWidth] = useStoredSize(
      'opendeck.library.inspector.v2',
      260,
      560,
    );
    const [fileDropActive, setFileDropActive] = useState(false);
    const folderPathRef = useRef(folderPath);
    folderPathRef.current = folderPath;
    const [dropKey, setDropKey] = useState<string | null>(null);
    const [reordering, setReordering] = useState(false);
    const draggingPath = useRef<string | null>(null);
    const movedTo = useRef(new Map<string, string>());
    const clearView = () => {
      openedInitialFolder.current = true;
      setFolderPath(null);
      setBrowseKey(null);
      setResults(null);
      setQ('');
    };
    const openFolder = (path: string) => {
      openedInitialFolder.current = true;
      setFolderPath(path);
      setResults(null);
      setQ('');
      setSelectedPath(null);
    };

    // Tagging: an inline tag editor opens for one track row at a time.
    const [taggingPath, setTaggingPath] = useState<string | null>(null);
    const [tagInput, setTagInput] = useState('');
    // Re-pull whatever view is showing so a tag edit is reflected immediately.
    const reloadView = lib.refresh;
    const commitTag = (path: string) => {
      const t = tagInput.trim();
      setTaggingPath(null);
      setTagInput('');
      if (t)
        dbAddTag(path, t)
          .then(reloadView)
          .catch(() => {});
    };
    const dropTag = (path: string, tag: string) => {
      dbRemoveTag(path, tag)
        .then(reloadView)
        .catch(() => {});
    };
    const deckCount = Math.min(DECKS.length, Math.max(loadedPaths.length, 2));
    const trackMenuGroups = (track: DbTrack): TrackMenuItem[][] => [
      DECKS.slice(0, deckCount).map((deck, index) => ({
        id: `load-${deck.letter}`,
        label:
          listening && index === 0
            ? `Play on deck ${deck.letter}`
            : `Load on deck ${deck.letter}`,
        onSelect: () => {
          if (listening && index === 0) {
            playOnDeckA.current = track.path;
            void load(0, track.path).then((loaded) => {
              if (!loaded && playOnDeckA.current === track.path)
                playOnDeckA.current = null;
            });
            return;
          }
          void load(index, track.path);
        },
      })),
      [
        ...nextDecks.flatMap((deck) => {
          if (loadedPaths[deck.index] === track.path) return [];
          if (readQueuedPaths(deck.index).includes(track.path)) return [];
          return [
            {
              id: `queue-${deck.letter}`,
              label: `Queue on deck ${deck.letter}`,
              onSelect: () => queueTrack(deck.index, track.path),
            },
          ];
        }),
        {
          id: 'tag',
          label: 'Add tag',
          onSelect: () => {
            setSelectedPath(track.path);
            setTaggingPath(track.path);
            setTagInput('');
          },
        },
        ...track.tags.map((tag) => ({
          id: `untag-${tag}`,
          label: `Remove tag ${tag}`,
          onSelect: () => dropTag(track.path, tag),
        })),
      ],
      [
        {
          id: 'remove-folder',
          label: 'Remove from folder',
          detail: 'Moves the file to Music/removed from playlists',
          danger: true,
          onSelect: () => {
            void moveTrackFileOut(track);
          },
        },
        {
          id: 'remove-library',
          label: 'Remove from library',
          detail: 'The file stays on disk',
          danger: true,
          onSelect: () => {
            lib.remove(track.path);
            setSelectedPath((current) =>
              current === track.path ? null : current,
            );
          },
        },
      ],
    ];

    // Watched folders (auto-import).
    const [folders, setFolders] = useState<string[]>([]);
    const [scanning, setScanning] = useState(false);
    const refreshFolders = useCallback(() => {
      if (!inTauri()) return;
      listWatchFolders()
        .then(setFolders)
        .catch(() => {});
    }, []);
    useEffect(() => refreshFolders(), [refreshFolders]);
    useEffect(() => {
      if (!openedInitialFolder.current) return;
      try {
        localStorage.setItem(VIEW_FOLDER_KEY, folderPath ?? 'library');
      } catch {
        /* The view still works without persistence. */
      }
    }, [folderPath]);
    const loadedPathsRef = useRef<(string | undefined)[] | null>(null);
    useEffect(() => {
      const previous = loadedPathsRef.current;
      loadedPathsRef.current = loadedPaths;
      if (previous == null || folders.length === 0) return;
      const loaded = loadedPaths.find(
        (path, index) => path && path !== previous[index],
      );
      if (!loaded) return;
      const folder = directoryOf(loaded);
      if (!folders.some((root) => isInside(root, folder))) return;
      try {
        localStorage.setItem(PLAYING_FOLDER_KEY, folder);
      } catch {
        /* Playback still works without persistence. */
      }
    }, [loadedPaths, folders]);
    useEffect(() => {
      if (openedInitialFolder.current || folders.length === 0) return;
      const insideLibrary = (path: string | null) =>
        path != null && folders.some((root) => isInside(root, path));
      const playing = readStoredFolder(PLAYING_FOLDER_KEY);
      const view = readStoredView();
      const onlyFolder = folders.length === 1 ? (folders[0] ?? null) : null;
      const browsed =
        view !== 'unset' && view !== 'library' && insideLibrary(view)
          ? view
          : null;
      const restored =
        browsed ??
        (insideLibrary(playing)
          ? playing
          : view === 'unset'
            ? onlyFolder
            : null);
      openedInitialFolder.current = true;
      if (restored) setFolderPath(restored);
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
      getCurrentWebview()
        .onDragDropEvent((event) => {
          if (event.payload.type === 'enter') finderPaths = event.payload.paths;
          if (event.payload.type === 'leave') {
            finderPaths = [];
            setFileDropActive(false);
            return;
          }
          if (finderPaths.length === 0) return;
          const list = rootRef.current?.querySelector('.tracklist');
          const hovering =
            list != null && pointerOverElement(list, event.payload.position);
          if (event.payload.type === 'over') {
            setFileDropActive(hovering && folderPathRef.current != null);
            return;
          }
          if (event.payload.type !== 'drop') return;
          const paths = finderPaths;
          finderPaths = [];
          setFileDropActive(false);
          if (!hovering) return;
          const folder = folderPathRef.current;
          if (!folder) {
            setActionError('Open a folder before dropping files into it');
            return;
          }
          moveIntoFolder(folder, paths)
            .then(() => setActionError(null))
            .catch((error: unknown) => setActionError(String(error)));
        })
        .then((stop) => {
          if (alive) stopListening = stop;
          else stop();
        })
        .catch((error: unknown) => setActionError(String(error)));
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
        setFolderOrders((current) => {
          const next = Object.fromEntries(
            Object.entries(current).map(([folder, paths]) => [
              folder,
              paths.map((path) => moved.get(path) ?? path),
            ]),
          );
          writeFolderOrders(next);
          return next;
        });
        setSelectedPath((path) => (path && moved.get(path)) || path);
        retargetNextQueuePaths(moved);
      });
      return () => {
        un.then((stop) => stop());
      };
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
      } catch (error) {
        reportError(error);
      } finally {
        setScanning(false);
      }
    };
    const rescanFolders = async () => {
      setScanning(true);
      try {
        await rescanWatchFolders();
        lib.refresh();
        reloadView();
      } catch (error) {
        reportError(error);
      } finally {
        setScanning(false);
      }
    };
    const dropFolder = (dir: string) => {
      removeWatchFolder(dir)
        .then(() => {
          refreshFolders();
          if (folderPath && isInside(dir, folderPath)) setFolderPath(null);
        })
        .catch(() => {});
    };
    const baseName = (p: string) =>
      p
        .replace(/[\\/]+$/, '')
        .split(/[\\/]/)
        .pop() || p;
    const moveTrackFileOut = async (track: DbTrack) => {
      const name = track.title || baseName(track.path);
      const accepted = await ask(
        `This moves the actual track file for “${name}” to the "removed from playlists" folder in your Music folder.`,
        {
          title: 'Remove from folder',
          kind: 'warning',
          okLabel: 'Move file',
          cancelLabel: 'Cancel',
        },
      );
      if (!accepted) return;
      try {
        await removeFromFolder(track.path);
        setActionError(null);
      } catch (error) {
        reportError(error);
      }
    };
    const followFolderEdit = (edit: FolderEdit) => {
      if (edit.to) {
        const next = edit.to;
        setFolderPath((current) =>
          current ? retargetFolderPath(current, edit.from, next) : current,
        );
        const updatedOrders = folderOrdersAfterRename(
          folderOrders,
          edit.from,
          next,
        );
        writeFolderOrders(updatedOrders);
        setFolderOrders(updatedOrders);
        setFolders((current) =>
          current.map((folder) => retargetFolderPath(folder, edit.from, next)),
        );
        setBrowseKey((key) => retargetBrowseKey(key, edit.from, next));
        retargetStoredFolder(PLAYING_FOLDER_KEY, edit.from, next);
        retargetStoredFolder(VIEW_FOLDER_KEY, edit.from, next);
      } else {
        const remaining = folders.filter(
          (folder) => !isInside(edit.from, folder),
        );
        setFolders(remaining);
        setFolderPath((current) =>
          folderAfterDelete(current, edit.from, remaining),
        );
        const updatedOrders = folderOrdersAfterDelete(folderOrders, edit.from);
        writeFolderOrders(updatedOrders);
        setFolderOrders(updatedOrders);
        setBrowseKey((key) => retargetBrowseKey(key, edit.from, null));
        retargetStoredFolder(PLAYING_FOLDER_KEY, edit.from, null);
        retargetStoredFolder(VIEW_FOLDER_KEY, edit.from, null);
        forgetQueuedPaths(edit.removed);
      }
      lib.refresh();
      refreshFolders();
    };
    const openFolderMenu = (
      path: string,
      name: string,
      event: MouseEvent,
    ) => {
      event.preventDefault();
      event.stopPropagation();
      setTrackMenu(null);
      setFolderMenu({ path, name, x: event.clientX, y: event.clientY });
    };
    const folderMenuGroups = (path: string, name: string): TrackMenuItem[][] => [
      [
        {
          id: 'rename-folder',
          label: 'Rename',
          onSelect: () => {
            setFolderRenameError(null);
            setFolderRename({ path, name });
          },
        },
      ],
      [
        {
          id: 'delete-folder',
          label: 'Delete folder',
          detail: 'Moves the folder to the Trash',
          danger: true,
          onSelect: () => {
            void deleteFolderFromDisk(path, name);
          },
        },
      ],
    ];
    const deleteFolderFromDisk = async (path: string, name: string) => {
      if (!inTauri()) {
        setActionError('Open the desktop app to change folders on disk');
        return;
      }
      const accepted = await ask(
        `Move “${name}” to the Trash? The folder and everything inside it leave the library.`,
        {
          title: 'Delete folder',
          kind: 'warning',
          okLabel: 'Move to Trash',
          cancelLabel: 'Cancel',
        },
      );
      if (!accepted || folderEditBusy.current) return;
      folderEditBusy.current = true;
      try {
        const edit = await deleteLibraryFolder(path);
        followFolderEdit(edit);
        setActionError(null);
      } catch (error) {
        reportError(error);
      } finally {
        folderEditBusy.current = false;
      }
    };
    const submitFolderRename = async (name: string) => {
      if (!folderRename || folderEditBusy.current) return;
      const trimmed = name.trim();
      if (!trimmed || trimmed === folderRename.name) {
        setFolderRename(null);
        setFolderRenameError(null);
        return;
      }
      if (!inTauri()) {
        setFolderRenameError('Open the desktop app to change folders on disk');
        return;
      }
      folderEditBusy.current = true;
      setFolderRenameBusy(true);
      setFolderRenameError(null);
      try {
        const edit = await renameLibraryFolder(folderRename.path, trimmed);
        followFolderEdit(edit);
        setFolderRename(null);
        setActionError(null);
      } catch (error) {
        setFolderRenameError(String(error));
      } finally {
        folderEditBusy.current = false;
        setFolderRenameBusy(false);
      }
    };

    const sourceTracks = results ?? lib.tracks;
    const filtered = useMemo(
      () => sortTracks(sourceTracks, sort),
      [sourceTracks, sort],
    );
    const browsingFolder = Boolean(folderPath) && !q.trim();
    const folderEntries = useMemo(
      () =>
        browsingFolder && folderPath
          ? childFolders(lib.tracks, folderPath)
          : [],
      [browsingFolder, folderPath, lib.tracks],
    );
    const folderTracks = useMemo(() => {
      if (!browsingFolder || !folderPath) return filtered;
      const direct = directTracks(lib.tracks, folderPath);
      if (sort.column !== 'order') return sortTracks(direct, sort);
      const compareOrder = folderOrderComparator(folderOrders[folderPath]);
      const bySavedOrder = [...direct].sort((left, right) =>
        compareOrder(left.path, right.path),
      );
      return sort.direction === 'asc' ? bySavedOrder : bySavedOrder.reverse();
    }, [browsingFolder, folderPath, folderOrders, lib.tracks, sort, filtered]);
    useEffect(() => {
      if (!browsingFolder) setReorderArmed(false);
    }, [browsingFolder]);
    const canReorder = browsingFolder && reorderArmed && !reordering;
    const reorderTrack = (from: string, to: string) => {
      if (!canReorder) return;
      const next = reorderPaths(
        folderTracks.map((track) => track.path),
        from,
        to,
      );
      if (!next) return;
      setReordering(true);
      setActionError(null);
      reorderFolderTracks(next)
        .then((moves) => {
          const relocated = new Map(moves.map((move) => [move.from, move.to]));
          if (folderPath)
            rememberFolderOrder(
              folderPath,
              next.map((path) => relocated.get(path) ?? path),
            );
          setSort({ column: 'order', direction: 'asc' });
        })
        .catch((error) => reportError(error))
        .finally(() => {
          setReordering(false);
          setDropKey(null);
        });
    };
    const reorderPointer = useRef<{ x: number; y: number } | null>(null);
    const reorderScrollFrame = useRef<number | null>(null);
    const stopReorderScroll = () => {
      if (reorderScrollFrame.current != null)
        cancelAnimationFrame(reorderScrollFrame.current);
      reorderScrollFrame.current = null;
    };
    const scrollReorderList = () => {
      const pointer = reorderPointer.current;
      const body = rootRef.current?.querySelector<HTMLElement>('.tl-sheet');
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
    const noteReorderPointer = (event: {
      clientX: number;
      clientY: number;
    }) => {
      if (!canReorder || !draggingPath.current) return;
      reorderPointer.current = { x: event.clientX, y: event.clientY };
      const body = rootRef.current?.querySelector<HTMLElement>('.tl-sheet');
      if (
        !body ||
        reorderScrollStep(event.clientY, body) === 0 ||
        reorderScrollFrame.current != null
      )
        return;
      reorderScrollFrame.current = requestAnimationFrame(scrollReorderList);
    };
    const dropReorderAtPoint = (event: DragEvent<HTMLElement>) => {
      if (!canReorder || !draggingPath.current) return;
      event.preventDefault();
      const from =
        event.dataTransfer.getData('application/x-compas-track') ||
        draggingPath.current;
      const body = rootRef.current?.querySelector<HTMLElement>('.tl-sheet');
      const target = body
        ? trackPathAtReorderPoint(body, event.clientX, event.clientY)
        : null;
      stopReorderScroll();
      setDropKey(null);
      if (from && target && from !== target) reorderTrack(from, target);
    };
    const libraryCounts = useMemo(
      () => folderPlayCounts(lib.tracks, null),
      [lib.tracks],
    );
    const viewCounts = useMemo(
      () =>
        browsingFolder && folderPath
          ? folderPlayCounts(lib.tracks, folderPath)
          : libraryCounts,
      [browsingFolder, folderPath, lib.tracks, libraryCounts],
    );
    const resetPlayed = () => {
      dbResetPlayCounts(browsingFolder ? folderPath : null).catch((error) =>
        reportError(error),
      );
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
    const playedLabel = (played: number, total: number) =>
      listening ? String(total) : `${played}/${total}`;
    const playedClass = (played: number, total: number) =>
      listening ? '' : playCountClass(played, total);
    type BrowseRow =
      | { kind: 'up'; path: string | null; name: string; key: string }
      | {
          kind: 'folder';
          path: string;
          name: string;
          trackCount: number;
          playedCount: number;
          key: string;
        }
      | { kind: 'track'; track: DbTrack; key: string };
    const browseRows = useMemo<BrowseRow[]>(() => {
      if (!browsingFolder || !folderPath)
        return folderTracks.map((track) => ({
          kind: 'track',
          track,
          key: `track:${track.path}`,
        }));
      const parent = ascendFolder(folderPath, folders);
      const up: BrowseRow = parent
        ? {
            kind: 'up',
            path: parent,
            name: baseName(parent),
            key: `up:${parent}`,
          }
        : { kind: 'up', path: null, name: 'All tracks', key: 'up:library' };
      return [
        up,
        ...folderEntries.map((entry) => ({
          kind: 'folder' as const,
          ...entry,
          key: `folder:${entry.path}`,
        })),
        ...folderTracks.map((track) => ({
          kind: 'track' as const,
          track,
          key: `track:${track.path}`,
        })),
      ];
    }, [browsingFolder, folderPath, folders, folderEntries, folderTracks]);
    const selected =
      folderTracks.find((track) => track.path === selectedPath) ?? null;
    const selectedBrowse =
      browseRows.find((row) => row.key === browseKey) ?? null;
    useEffect(() => {
      if (selectedPath && folderTracks.some((track) => track.path === selectedPath))
        movedTo.current.clear();
      // Keep identity through asynchronous refreshes. LOAD resolves against the
      // current visible rows and reports a missing selection instead of loading another row.
    }, [folderTracks, selectedPath]);
    const sourceIndexRef = useRef(0);
    const browseRowsRef = useRef(browseRows);
    browseRowsRef.current = browseRows;
    useEffect(() => {
      if (folderPath)
        sourceIndexRef.current =
          1 +
          Math.max(
            0,
            folders.findIndex((folder) => isInside(folder, folderPath)),
          );
      else sourceIndexRef.current = 0;
    }, [folderPath, folders]);
    const revealRow = (key: string) =>
      requestAnimationFrame(() => {
        if (selectionKeyRef.current !== key) return;
        const row = rootRef.current?.querySelector<HTMLElement>(
          `[data-browse-key="${CSS.escape(key)}"]`,
        );
        row?.focus();
        row?.scrollIntoView({ block: 'nearest' });
      });
    const selectBrowse = (index: number, focus = false) => {
      const row = browseRowsRef.current[index];
      if (!row) return;
      setBrowseKey(row.key);
      if (focus) revealRow(row.key);
    };
    const enterBrowse = (row: BrowseRow) => {
      if (row.kind === 'track') return;
      if (row.path) openFolder(row.path);
      else clearView();
    };

    useEffect(() => {
      if (!focusTarget) return;
      rootRef.current?.scrollIntoView({ block: 'nearest' });
      if (focusTarget === 'library') {
        searchRef.current?.focus();
        searchRef.current?.select();
        return;
      }
      setCratesPulse(true);
      const id = window.setTimeout(() => setCratesPulse(false), 850);
      return () => window.clearTimeout(id);
    }, [focusSeq, focusTarget]);
    const [browseTarget, setBrowseTarget] = useState<'tracks' | 'sources'>(
      'tracks',
    );
    const browseSources = (steps: number) => {
      const lastSourceIndex = folders.length;
      const nextSourceIndex = Math.max(
        0,
        Math.min(lastSourceIndex, sourceIndexRef.current + steps),
      );
      if (nextSourceIndex === sourceIndexRef.current) return;
      sourceIndexRef.current = nextSourceIndex;
      if (nextSourceIndex === 0) clearView();
      else openFolder(folders[nextSourceIndex - 1]);
    };
    const hardwareRef = useRef<(control: string, value: number) => void>(
      () => {},
    );
    hardwareRef.current = (control, value) => {
      if (control === 'library.focus' && value > 0) {
        const row = selectedControllerRow(browseRowsRef.current, selectionKeyRef.current);
        if (browseTarget === 'tracks' && row && row.kind !== 'track') {
          enterBrowse(row);
          return;
        }
        setBrowseTarget((target) =>
          target === 'tracks' ? 'sources' : 'tracks',
        );
      }
      if (control === 'library.browse' && value !== 0) {
        if (browseTarget === 'sources') browseSources(Math.trunc(value));
        else {
          const row = stepControllerRow(browseRowsRef.current, selectionKeyRef.current, value);
          if (row) {
            setBrowseKey(row.key);
            revealRow(row.key);
          }
        }
      }
      const target = control.match(/^library\.load\.([01])$/);
      if (target && value > 0) {
        try {
          const path = controllerLoadPath(browseRowsRef.current, selectionKeyRef.current);
          void load(Number(target[1]), path);
        } catch (error) { reportError(error); }
      }
    };
    useEffect(() => {
      if (!inTauri()) return;
      let alive = true;
      let stop: (() => void) | undefined;
      onControllerUpdate((u) => {
        if (alive) hardwareRef.current(u.control, u.value);
      }).then((unlisten) => {
        if (alive) stop = unlisten;
        else unlisten();
      }).catch((error) => {
        if (alive) reportError(`Controller library connection failed: ${String(error)}`);
      });
      return () => { alive = false; stop?.(); };
    }, []);

    return (
      <section className="library" ref={rootRef}>
        <aside
          className={`sources ${cratesPulse || browseTarget === 'sources' ? 'sources--focus' : ''}`}
          style={
            sourcesWidth == null
              ? { flex: 'none' }
              : { width: sourcesWidth, flex: 'none' }
          }
        >
          <div className="overline src-group">SOURCES</div>
          <button
            className={`src-row ${!folderPath ? 'src-row--active' : ''}`}
            onClick={clearView}
          >
            <span className="src-dot" style={{ background: LIBRARY_ACCENT }} />
            <span className="src-name">Local Library</span>
            <span
              className={`ctrl-tag ${playedClass(libraryCounts.played, libraryCounts.total)}`}
            >
              {playedLabel(libraryCounts.played, libraryCounts.total)}
            </span>
          </button>

          <div className="overline src-group src-group--crates">
            FOLDERS
            <span style={{ display: 'flex', gap: 4 }}>
              <button
                className="crate-add"
                onClick={rescanFolders}
                disabled={scanning || folders.length === 0}
                title="Re-scan watched folders for new files"
              >
                ⟳
              </button>
              <button
                className="crate-add"
                onClick={addFolder}
                disabled={scanning}
                title="Watch a folder — auto-imports its audio files"
              >
                ＋
              </button>
            </span>
          </div>
          {folders.length === 0 && (
            <div className="src-row src-row--muted">
              <span className="src-name">
                {scanning ? 'Scanning…' : 'No watched folders'}
              </span>
            </div>
          )}
          {folders.map((f) => {
            const counts = folderPlayCounts(lib.tracks, f);
            return (
              <div
                key={f}
                className={`src-row ${folderPath && isInside(f, folderPath) ? 'src-row--active' : ''}`}
                title={f}
                onClick={() => openFolder(f)}
                onMouseDown={(event) => {
                  if (event.button !== 2) return;
                  openFolderMenu(f, baseName(f), event);
                }}
                onContextMenu={(event) => openFolderMenu(f, baseName(f), event)}
                role="button"
                tabIndex={0}
                onKeyDown={(e) => {
                  if (e.target !== e.currentTarget) return;
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault();
                    openFolder(f);
                  }
                }}
              >
                <Icon name="folder" size={13} />
                <span className="src-name">{baseName(f)}</span>
                <span
                  className={`mono src-play-count ${playedClass(counts.played, counts.total)}`}
                >
                  {playedLabel(counts.played, counts.total)}
                </span>
                <button
                  className="crate-add"
                  onClick={(e) => {
                    e.stopPropagation();
                    dropFolder(f);
                  }}
                  title="Stop watching this folder"
                >
                  ✕
                </button>
              </div>
            );
          })}
        </aside>
        <PaneDivider
          label="Resize folders"
          value={sourcesWidth ?? 320}
          min={260}
          max={640}
          fallback={320}
          grow={1}
          onChange={setSourcesWidth}
        />

        <div
          className={`tracklist${fileDropActive ? ' tracklist--filedrop' : ''}`}
        >
          <div className="tl-toolbar">
            <div className="search">
              <Icon name="search" size={14} />
              <input
                ref={searchRef}
                value={q}
                onChange={(e) => setQ(e.target.value)}
                placeholder="Search tracks, artists, BPM or key"
                aria-label="Search library"
                title="Type a title, artist, tempo (117 or 117.69), or key (8A or Am). bpm:120-128 and key:8A work too. Terms AND together; OR starts another group."
              />
            </div>
            <button className="add-btn" onClick={lib.add} disabled={lib.busy}>
              {lib.busy ? 'Adding…' : '+ Add tracks'}
            </button>
            {browsingFolder && folderPath && (
              <nav className="tl-crumbs" aria-label="Folder">
                {folderCrumbs(folderPath, folders).map(
                  (crumb, crumbIndex, crumbs) => (
                    <span key={crumb.path} className="tl-crumb">
                      {crumbIndex > 0 && <span aria-hidden="true">/</span>}
                      <button
                        type="button"
                        aria-current={
                          crumbIndex === crumbs.length - 1 ? 'page' : undefined
                        }
                        onClick={() => openFolder(crumb.path)}
                        onContextMenu={(event) =>
                          openFolderMenu(crumb.path, crumb.name, event)
                        }
                      >
                        {crumb.name}
                      </button>
                    </span>
                  ),
                )}
              </nav>
            )}
            <span
              className={`mono tl-count ${playedClass(viewCounts.played, viewCounts.total)}`}
              title={
                listening
                  ? 'Tracks in this view'
                  : 'Played tracks / tracks in this view'
              }
            >
              {playedLabel(viewCounts.played, viewCounts.total)}
            </span>
            <button
              type="button"
              className={`add-btn${listening ? ' add-btn--on' : ''}`}
              aria-pressed={listening}
              disabled={listeningBusy}
              title={
                listening
                  ? 'Return to performance mode. Played marks count again, and deck A stops auto queue and play. Next tracks rank by BPM, then key, then folder order.'
                  : "Reset played marks, ignore them on folders and the list, and turn on deck A queue and play 'next' modes. BPM has less importance on suggestions for listening mode."
              }
              onClick={() => {
                void toggleListening();
              }}
            >
              {listening ? 'Performance mode' : 'Listening mode'}
            </button>
            <button
              type="button"
              className={`add-btn${canReorder ? ' add-btn--on' : ''}`}
              aria-pressed={canReorder}
              disabled={!folderPath || !browsingFolder || reordering}
              title={
                canReorder
                  ? 'Reorder is on. Drag a track onto another row. Reordering edits filenames so the folder matches this list. Click again to drag onto decks only.'
                  : folderPath
                    ? 'Turn on to rearrange this folder. Reordering edits filenames. Off, dragging a track only loads a deck.'
                    : 'Open a folder to rearrange its tracks. Reordering edits filenames.'
              }
              onClick={() => {
                stopReorderScroll();
                setDropKey(null);
                setReorderArmed((armed) => !armed);
              }}
            >
              Reorder
            </button>
            <button
              className="add-btn"
              onClick={resetPlayed}
              disabled={viewCounts.played === 0}
              title={
                browsingFolder
                  ? 'Clear played marks in this folder'
                  : 'Clear played marks for the whole library'
              }
            >
              Reset played
            </button>
          </div>

          {actionError && (
            <div className="tl-banner" role="alert">
              <span>{actionError}</span>
              <button className="chip" onClick={() => setActionError(null)}>
                Dismiss
              </button>
            </div>
          )}
          <div className="tl-sheet">
            <div
              className="tl-head tl-grid"
              aria-label="Sort tracks"
              onDragOver={(event) => {
                if (!canReorder || !draggingPath.current) return;
                event.preventDefault();
                event.dataTransfer.dropEffect = 'move';
                noteReorderPointer(event);
              }}
              onDrop={dropReorderAtPoint}
            >
              {sortHeader('order', '#')}
              {sortHeader('title', 'Title')}
              {sortHeader('artist', 'Artist')}
              {sortHeader('bpm', 'BPM')}
              {sortHeader('key', 'Key')}
              <span className="tl-spacer" aria-hidden="true" />
              <span>Waveform</span>
              {sortHeader('duration', 'Time')}
            </div>
            <div
              className="tl-body"
              role="listbox"
              aria-label="Tracks"
              aria-describedby="library-keyboard-help"
              tabIndex={browseRows.length ? -1 : 0}
              onDragOver={(event) => {
                if (!canReorder || !draggingPath.current) return;
                event.preventDefault();
                noteReorderPointer(event);
              }}
              onDrop={dropReorderAtPoint}
              onKeyDown={(e) => {
                if (e.key === 'Backspace' && browsingFolder) {
                  e.preventDefault();
                  const parent = folderPath
                    ? ascendFolder(folderPath, folders)
                    : null;
                  if (parent) openFolder(parent);
                  else clearView();
                }
              }}
            >
              {browseRows.length === 0 ? (
                <div className="tl-empty">
                  {q.trim()
                    ? 'No matching tracks.'
                    : 'Add local tracks to start your collection.'}
                  <p>
                    Select a track to view its details and load it onto a deck.
                  </p>
                </div>
              ) : (
                browseRows.map((row, index) => {
                  const selectedRow =
                    row.key === browseKey;
                  const move = (next: number) =>
                    selectBrowse(
                      Math.max(0, Math.min(browseRows.length - 1, next)),
                      true,
                    );
                  if (row.kind !== 'track') {
                    return (
                      <div
                        key={row.key}
                        className={`tl-row tl-row--folder ${selectedRow ? 'tl-row--selected' : ''}`}
                        role="option"
                        aria-selected={selectedRow}
                        aria-label={
                          row.kind === 'up'
                            ? `Back to ${row.name}`
                            : `${row.name}, ${row.playedCount} of ${row.trackCount} played`
                        }
                        tabIndex={
                          selectedRow || (!browseKey && index === 0) ? 0 : -1
                        }
                        data-browse-key={row.key}
                        onMouseDown={(event) => {
                          if (row.kind !== 'folder' || event.button !== 2) return;
                          selectBrowse(index);
                          openFolderMenu(row.path, row.name, event);
                        }}
                        onContextMenu={(event) => {
                          if (row.kind !== 'folder') return;
                          selectBrowse(index);
                          openFolderMenu(row.path, row.name, event);
                        }}
                        onClick={() => {
                          selectBrowse(index);
                          enterBrowse(row);
                        }}
                        onKeyDown={(e) => {
                          if (e.key === 'ArrowDown') {
                            e.preventDefault();
                            move(index + 1);
                          } else if (e.key === 'ArrowUp') {
                            e.preventDefault();
                            move(index - 1);
                          } else if (e.key === 'Home') {
                            e.preventDefault();
                            move(0);
                          } else if (e.key === 'End') {
                            e.preventDefault();
                            move(browseRows.length - 1);
                          } else if (e.key === 'Enter') {
                            e.preventDefault();
                            enterBrowse(row);
                          }
                        }}
                      >
                        <Icon
                          name={
                            row.kind === 'up' && row.path == null
                              ? 'list'
                              : 'folder'
                          }
                          size={14}
                        />
                        <span className="tl-folder-name">{row.name}</span>
                        <span
                          className={`mono tl-folder-count ${row.kind === 'folder' ? playedClass(row.playedCount, row.trackCount) : ''}`}
                        >
                          {row.kind === 'folder'
                            ? playedLabel(row.playedCount, row.trackCount)
                            : ''}
                        </span>
                      </div>
                    );
                  }
                  const t = row.track;
                  const loadedDecks = DECKS.filter(
                    (_, i) => loadedPaths[i] === t.path,
                  );
                  const loadedAs = loadedDecks[0];
                  const trackNumber =
                    browseRows
                      .slice(0, index)
                      .filter((item) => item.kind === 'track').length + 1;
                  return (
                    <div
                      key={row.key}
                      className={`tl-row tl-grid ${!listening && t.play_count > 0 ? 'tl-row--played' : ''} ${selectedRow ? 'tl-row--selected' : ''} ${canReorder ? 'tl-row--reorder' : ''} ${dropKey === row.key ? 'tl-row--drop' : ''}`}
                      style={
                        loadedAs
                          ? { borderLeftColor: loadedAs.color }
                          : undefined
                      }
                      role="option"
                      aria-selected={selectedRow}
                      aria-label={`${t.title}, ${t.artist || 'Unknown artist'}${t.bpm != null && t.bpm > 0 ? `, ${t.bpm.toFixed(2)} BPM` : ''}${t.key_camelot ? `, ${formatKey(t.key_camelot, t.key_name, keyNotation)}` : ''}${loadedDecks.length ? `, loaded on deck ${loadedDecks.map((d) => d.letter).join(', ')}` : ''}`}
                      tabIndex={
                        selectedRow || (!selected && !browseKey && index === 0)
                          ? 0
                          : -1
                      }
                      data-browse-key={row.key}
                      data-track-index={index}
                      data-track-path={t.path}
                      draggable
                      onMouseDown={(event) => {
                        if (event.button !== 2) return;
                        event.preventDefault();
                        selectBrowse(index);
                        setFolderMenu(null);
                        setTrackMenu({
                          track: t,
                          x: event.clientX,
                          y: event.clientY,
                        });
                      }}
                      onContextMenu={(event) => {
                        event.preventDefault();
                        selectBrowse(index);
                        setFolderMenu(null);
                        setTrackMenu({
                          track: t,
                          x: event.clientX,
                          y: event.clientY,
                        });
                      }}
                      onDragStart={(e) => {
                        if (e.button === 2) {
                          e.preventDefault();
                          return;
                        }
                        draggingPath.current = t.path;
                        e.dataTransfer.setData(
                          'application/x-compas-track',
                          t.path,
                        );
                        e.dataTransfer.setData('text/plain', t.path);
                        e.dataTransfer.effectAllowed = 'copyMove';
                      }}
                      onDragEnd={() => {
                        draggingPath.current = null;
                        stopReorderScroll();
                        setDropKey(null);
                      }}
                      onDragEnter={
                        canReorder
                          ? (e) => {
                              const from = draggingPath.current;
                              if (!from || from === t.path) return;
                              e.preventDefault();
                              e.dataTransfer.dropEffect = 'move';
                            }
                          : undefined
                      }
                      onDragOver={
                        canReorder
                          ? (e) => {
                              const from = draggingPath.current;
                              if (!from || from === t.path) return;
                              e.preventDefault();
                              e.dataTransfer.dropEffect = 'move';
                              if (dropKey !== row.key) setDropKey(row.key);
                            }
                          : undefined
                      }
                      onDrop={
                        canReorder
                          ? (e) => {
                              e.preventDefault();
                              e.stopPropagation();
                              const from =
                                e.dataTransfer.getData(
                                  'application/x-compas-track',
                                ) || draggingPath.current;
                              stopReorderScroll();
                              setDropKey(null);
                              if (from && from !== t.path)
                                reorderTrack(from, t.path);
                            }
                          : undefined
                      }
                      onClick={() => selectBrowse(index)}
                      onFocus={() => {
                        if (selectionKeyRef.current !== row.key) setBrowseKey(row.key);
                      }}
                      onDoubleClick={() => {
                        selectBrowse(index);
                        if (listening) {
                          playOnDeckA.current = t.path;
                          void load(0, t.path).then((loaded) => {
                            if (!loaded && playOnDeckA.current === t.path)
                              playOnDeckA.current = null;
                          });
                          return;
                        }
                        inspectorRef.current?.focus();
                      }}
                      onKeyDown={(e) => {
                        if (e.key === 'ArrowDown') {
                          e.preventDefault();
                          move(index + 1);
                        } else if (e.key === 'ArrowUp') {
                          e.preventDefault();
                          move(index - 1);
                        } else if (e.key === 'Home') {
                          e.preventDefault();
                          move(0);
                        } else if (e.key === 'End') {
                          e.preventDefault();
                          move(browseRows.length - 1);
                        } else if (e.key === 'Enter') {
                          e.preventDefault();
                          inspectorRef.current?.focus();
                        }
                      }}
                      title={
                        canReorder
                          ? 'Drag onto the row this track should take. Reordering edits filenames.'
                          : listening
                            ? 'Double-click loads this track on deck A and starts it'
                            : 'Select for details · right-click for actions · Enter opens inspector · drag onto a deck to load'
                      }
                    >
                      <span
                        className="tl-tag"
                        style={loadedAs ? { color: loadedAs.color } : undefined}
                      >
                        {loadedDecks.length
                          ? loadedDecks.map((d) => d.letter).join('·')
                          : String(trackNumber).padStart(2, '0')}
                      </span>
                      <TrackCells
                        path={t.path}
                        title={t.title || baseName(t.path)}
                        artist={t.artist}
                        color={loadedAs?.color}
                        afterArtist={
                          <span
                            className="mono tl-bpm"
                            title={
                              t.bpm == null
                                ? 'Queued for background BPM analysis; select to prioritize'
                                : t.bpm > 0
                                  ? 'Original tempo'
                                  : 'No tempo detected'
                            }
                          >
                            {t.bpm == null
                              ? '…'
                              : t.bpm > 0
                                ? t.bpm.toFixed(2)
                                : '—'}
                          </span>
                        }
                        beforeWave={
                          <span
                            className="mono tl-key"
                            title={t.key_name ?? undefined}
                          >
                            {t.key_camelot
                              ? formatKey(
                                  t.key_camelot,
                                  t.key_name,
                                  keyNotation,
                                )
                              : '—'}
                          </span>
                        }
                      />
                      <span className="mono">{fmtMs(t.duration_ms)}</span>
                    </div>
                  );
                })
              )}
            </div>
          </div>
          <div className="tl-footer" id="library-keyboard-help">
            ↑ ↓ Select · Enter opens a folder · Right-click a track for actions
            {folderPath
              ? ' · Drop audio files here to move them into this folder'
              : ''}
          </div>
        </div>
        <PaneDivider
          label="Resize track details"
          value={inspectorWidth ?? 300}
          min={260}
          max={560}
          fallback={300}
          grow={-1}
          onChange={setInspectorWidth}
        />
        <aside
          className="track-inspector"
          ref={inspectorRef}
          tabIndex={-1}
          aria-label="Track details"
          style={
            inspectorWidth == null
              ? { flex: 'none' }
              : { width: inspectorWidth, flex: 'none' }
          }
        >
          {selected ? (
            <TrackInspector
              track={selected}
              keyNotation={keyNotation}
              showPlayCount={!listening}
              decks={DECKS}
              loadedPaths={loadedPaths}
              nextDecks={nextDecks}
              tagging={taggingPath === selected.path}
              tagInput={tagInput}
              onTagInput={setTagInput}
              onBeginTag={() => {
                setTaggingPath(selected.path);
                setTagInput('');
              }}
              onCommitTag={() => commitTag(selected.path)}
              onCancelTag={() => {
                setTaggingPath(null);
                setTagInput('');
              }}
              onDropTag={(tag) => dropTag(selected.path, tag)}
              onLoad={(deck) => {
                void load(deck, selected.path);
              }}
              onQueue={(deck) => queueTrack(deck, selected.path)}
              onRemove={() => {
                lib.remove(selected.path);
                setSelectedPath(null);
              }}
            />
          ) : selectedBrowse?.kind === 'folder' ? (
            <div className="inspector-empty">
              <Icon name="folder" size={28} />
              <h3>{selectedBrowse.name}</h3>
              <p>{selectedBrowse.trackCount} tracks in this folder.</p>
              <p>Right-click the folder to rename it or move it to the Trash.</p>
            </div>
          ) : (
            <div className="inspector-empty">
              <span aria-hidden="true">♫</span>
              <h3>Your music, in detail</h3>
              <p>
                Select a track to see its cover, file details and waveform.
              </p>
            </div>
          )}
        </aside>
        {trackMenu && (
          <TrackMenu
            x={trackMenu.x}
            y={trackMenu.y}
            groups={trackMenuGroups(
              lib.tracks.find((track) => track.path === trackMenu.track.path) ??
                trackMenu.track,
            )}
            onClose={closeTrackMenu}
          />
        )}
        {folderMenu && (
          <TrackMenu
            x={folderMenu.x}
            y={folderMenu.y}
            groups={folderMenuGroups(folderMenu.path, folderMenu.name)}
            onClose={closeFolderMenu}
          />
        )}
        {folderRename && (
          <FolderNamePrompt
            initialName={folderRename.name}
            busy={folderRenameBusy}
            error={folderRenameError}
            onCancel={() => {
              setFolderRename(null);
              setFolderRenameError(null);
            }}
            onSubmit={(name) => {
              void submitFolderRename(name);
            }}
          />
        )}
      </section>
    );
  }),
);
