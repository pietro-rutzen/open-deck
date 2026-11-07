/** Matches `.tl-row` min-height in the shipped library styles. */
export const LIBRARY_ROW_HEIGHT = 40;
/** Matches the sticky `.tl-head` height that covers the top of the scrollport. */
export const LIBRARY_HEADER_HEIGHT = 29;
export const LIBRARY_OVERSCAN = 8;
const FALLBACK_VISIBLE_ROWS = 12;

export interface LibraryRowWindow {
  start: number;
  end: number;
  topHeight: number;
  bottomHeight: number;
  rowHeight: number;
}

export interface LibraryMountedRange {
  start: number;
  end: number;
}

export function visibleLibraryRowWindow(
  rowCount: number,
  scrollTop: number,
  viewportHeight: number,
  rowHeight = LIBRARY_ROW_HEIGHT,
  headerHeight = LIBRARY_HEADER_HEIGHT,
  overscan = LIBRARY_OVERSCAN,
): LibraryRowWindow {
  const height = rowHeight > 0 ? rowHeight : LIBRARY_ROW_HEIGHT;
  const count = Math.max(0, Math.trunc(rowCount));
  if (count === 0) {
    return { start: 0, end: 0, topHeight: 0, bottomHeight: 0, rowHeight: height };
  }
  const requested = Number.isFinite(scrollTop) ? Math.max(0, scrollTop) : 0;
  const viewport = Number.isFinite(viewportHeight) && viewportHeight > 0
    ? viewportHeight
    : height * FALLBACK_VISIBLE_ROWS;
  const header = Number.isFinite(headerHeight) && headerHeight > 0 ? headerHeight : 0;
  const maxScroll = Math.max(0, header + count * height - viewport);
  const scroll = Math.min(requested, maxScroll);
  const first = Math.floor(scroll / height);
  const last = Math.ceil((scroll + viewport - header) / height);
  const start = Math.max(0, Math.min(count, first - overscan));
  const end = Math.max(start, Math.min(count, Math.max(first, last) + overscan));
  return {
    start,
    end,
    topHeight: start * height,
    bottomHeight: (count - end) * height,
    rowHeight: height,
  };
}

/** Keep one off-window row mounted so selection and focus survive a reorder. */
export function libraryMountedRanges(
  start: number,
  end: number,
  pinnedIndex: number | null,
  rowCount: number,
): LibraryMountedRange[] {
  const count = Math.max(0, Math.trunc(rowCount));
  const ranges: LibraryMountedRange[] = [];
  const add = (from: number, to: number) => {
    const clampedStart = Math.max(0, Math.min(count, from));
    const clampedEnd = Math.max(clampedStart, Math.min(count, to));
    if (clampedEnd > clampedStart) ranges.push({ start: clampedStart, end: clampedEnd });
  };
  add(start, end);
  if (pinnedIndex != null && pinnedIndex >= 0 && pinnedIndex < count) add(pinnedIndex, pinnedIndex + 1);
  ranges.sort((left, right) => left.start - right.start);
  return ranges.reduce<LibraryMountedRange[]>((merged, range) => {
    const previous = merged[merged.length - 1];
    if (previous && previous.end >= range.start) {
      return [
        ...merged.slice(0, -1),
        { start: previous.start, end: Math.max(previous.end, range.end) },
      ];
    }
    return [...merged, range];
  }, []);
}

/** Scroll the sheet only when the row is outside the visible band (`nearest`). */
export function scrollTopForLibraryRow(
  index: number,
  scrollTop: number,
  viewportHeight: number,
  rowHeight = LIBRARY_ROW_HEIGHT,
  headerHeight = LIBRARY_HEADER_HEIGHT,
): number {
  const height = rowHeight > 0 ? rowHeight : LIBRARY_ROW_HEIGHT;
  const header = Number.isFinite(headerHeight) && headerHeight > 0 ? headerHeight : 0;
  const scroll = Number.isFinite(scrollTop) ? Math.max(0, scrollTop) : 0;
  const viewport = Number.isFinite(viewportHeight) && viewportHeight > 0
    ? viewportHeight
    : height * FALLBACK_VISIBLE_ROWS;
  const top = header + index * height;
  const bottom = top + height;
  const viewStart = scroll + header;
  const viewEnd = scroll + viewport;
  if (top >= viewStart && bottom <= viewEnd) return scroll;
  if (top < viewStart) return Math.max(0, top - header);
  return Math.max(0, bottom - viewport);
}
