export type TerminalTabCloseMode = 'all' | 'others' | 'left' | 'right';

export function getTerminalTabCloseTargets(
  tabKeys: readonly string[],
  targetKey: string,
  mode: TerminalTabCloseMode,
) {
  if (mode === 'all') return [...tabKeys];

  const targetIndex = tabKeys.indexOf(targetKey);
  if (targetIndex === -1) return [];

  if (mode === 'others') {
    return tabKeys.filter((key) => key !== targetKey);
  }

  if (mode === 'left') {
    return tabKeys.slice(0, targetIndex);
  }

  return tabKeys.slice(targetIndex + 1);
}
