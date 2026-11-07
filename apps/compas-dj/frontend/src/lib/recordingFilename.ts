export function recordingDefaultFilename(when: Date): string {
  const stamp = when.toISOString().slice(0, 19).replace(/[:T]/g, "-");
  return `open-deck-mix-${stamp}.wav`;
}
