export interface LibraryDeckTarget {
  index: number;
  letter: string;
}

export interface LibrarySurfaceProps {
  loadedPaths: readonly (string | undefined)[];
  nextDecks: readonly LibraryDeckTarget[];
  keyNotation?: string;
  focusTarget?: 'library' | 'crates' | null;
  focusSeq?: number;
  onSamplerLoad?: (slot: number, path: string) => void;
}

const sameLoadedPaths = (
  previous: readonly (string | undefined)[],
  next: readonly (string | undefined)[],
) => previous.length === next.length && previous.every((path, index) => path === next[index]);

const sameNextDecks = (previous: readonly LibraryDeckTarget[], next: readonly LibraryDeckTarget[]) =>
  previous.length === next.length &&
  previous.every(
    (deck, index) => deck.index === next[index]?.index && deck.letter === next[index]?.letter,
  );

export function libraryPropsEqual(previous: LibrarySurfaceProps, next: LibrarySurfaceProps): boolean {
  return (
    sameLoadedPaths(previous.loadedPaths, next.loadedPaths) &&
    sameNextDecks(previous.nextDecks, next.nextDecks) &&
    previous.keyNotation === next.keyNotation &&
    previous.focusTarget === next.focusTarget &&
    previous.focusSeq === next.focusSeq &&
    previous.onSamplerLoad === next.onSamplerLoad
  );
}
