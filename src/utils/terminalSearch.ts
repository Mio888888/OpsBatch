export interface TerminalSearchLine {
  row: number;
  text: string;
}

export interface TerminalSearchMatch {
  row: number;
  column: number;
  length: number;
}

export function findTerminalSearchMatches(lines: readonly TerminalSearchLine[], query: string): TerminalSearchMatch[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return [];

  const matches: TerminalSearchMatch[] = [];
  lines.forEach((line) => {
    const haystack = line.text.toLowerCase();
    let column = haystack.indexOf(needle);
    while (column !== -1) {
      matches.push({ row: line.row, column, length: needle.length });
      column = haystack.indexOf(needle, column + needle.length);
    }
  });

  return matches;
}
