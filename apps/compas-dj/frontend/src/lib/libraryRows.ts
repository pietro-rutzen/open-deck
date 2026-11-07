type NumberedRow<T extends { kind: string }> = T extends unknown
  ? T & { trackNumber: T['kind'] extends 'track' ? number : null }
  : never;

export function withTrackNumbers<T extends { kind: string }>(rows: readonly T[]): NumberedRow<T>[] {
  let nextTrackNumber = 0;
  return rows.map((row) => {
    if (row.kind !== 'track') return { ...row, trackNumber: null } as NumberedRow<T>;
    nextTrackNumber += 1;
    return { ...row, trackNumber: nextTrackNumber } as NumberedRow<T>;
  });
}
