/** A stable row identity, never a cached index into a refreshed/sorted library. */
export interface LibraryControllerRow {
  key: string;
  kind: 'track' | 'folder' | 'up';
  track?: { path: string };
}
export function selectedControllerRow<T extends LibraryControllerRow>(rows: readonly T[], key: string | null): T | undefined {
  return key == null ? undefined : rows.find(row => row.key === key);
}
export function stepControllerRow<T extends LibraryControllerRow>(rows: readonly T[], key: string | null, steps: number): T | undefined {
  if (!rows.length || !Number.isFinite(steps)) return undefined;
  const current = rows.findIndex(row => row.key === key);
  const next = current < 0 ? (steps < 0 ? rows.length - 1 : 0) : Math.max(0, Math.min(rows.length - 1, current + Math.trunc(steps)));
  return rows[next];
}
export function controllerLoadPath(rows: readonly LibraryControllerRow[], key: string | null): string {
  const row = selectedControllerRow(rows, key);
  if (!row) throw new Error('Select a track with the browse knob before pressing LOAD.');
  if (row.kind !== 'track' || !row.track) throw new Error('A folder is selected. Open it with the browse knob, then select a track to load.');
  return row.track.path;
}
