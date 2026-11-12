export const mp3CopyProgressLabel = (
  done: number,
  total: number,
  active: boolean,
): string => {
  if (!active || !Number.isFinite(done) || !Number.isFinite(total) || total <= 0) {
    return '';
  }
  const shownTotal = Math.floor(total);
  const shownDone = Math.min(Math.max(Math.floor(done), 0), shownTotal);
  return `${shownDone}/${shownTotal}`;
};
