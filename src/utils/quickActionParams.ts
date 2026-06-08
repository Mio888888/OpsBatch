export interface QuickActionParam {
  name: string;
  defaultValue: string;
}

export function parseQuickActionParams(command: string): QuickActionParam[] {
  const regex = /\{\{(\w+)(?::([^}]+))?\}\}/g;
  const params: QuickActionParam[] = [];
  const seen = new Set<string>();
  let match: RegExpExecArray | null;
  while ((match = regex.exec(command)) !== null) {
    const name = match[1];
    if (!seen.has(name)) {
      seen.add(name);
      params.push({ name, defaultValue: match[2] || '' });
    }
  }
  return params;
}

export function replaceQuickActionParams(command: string, values: Record<string, string>): string {
  return command.replace(/\{\{(\w+)(?::([^}]+))?\}\}/g, (_match, name: string, fallback: string | undefined) => {
    return values[name] !== undefined ? values[name] : (fallback || '');
  });
}
