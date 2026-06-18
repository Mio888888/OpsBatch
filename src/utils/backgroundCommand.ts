export function parseOpsBatchBackgroundCommand(command: string): string | null {
  const trimmed = command.trim();
  const match = trimmed.match(/^opsbatch(?:-bg|\s+bg)\s+([\s\S]+)$/);
  const backgroundCommand = match?.[1]?.trim();
  return backgroundCommand ? backgroundCommand : null;
}
