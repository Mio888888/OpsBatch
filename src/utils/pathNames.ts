export function basenameFromPath(path: string, fallback = 'file'): string {
  const trimmed = path.replace(/[\\/]+$/, '');
  if (!trimmed) return fallback;
  const parts = trimmed.split(/[\\/]/);
  return parts[parts.length - 1] || fallback;
}

export function joinPath(base: string, name: string): string {
  if (!base) return name;
  const separator = base.includes('\\') && !base.includes('/') ? '\\' : '/';
  const trimmedBase = base.replace(/[\\/]+$/, '');
  if (!trimmedBase) return `${separator}${name}`;
  return `${trimmedBase}${separator}${name}`;
}

export function dirnameFromPath(path: string): string {
  const driveRoot = path.match(/^([A-Za-z]:)[\\/]$/);
  if (driveRoot) return `${driveRoot[1]}\\`;
  const trimmed = path.replace(/[\\/]+$/, '');
  if (!trimmed) return '/';
  const separator = trimmed.includes('\\') && !trimmed.includes('/') ? '\\' : '/';
  const index = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'));
  if (index < 0) return '/';
  if (index === 0) return '/';
  if (/^[A-Za-z]:[\\/]?$/.test(trimmed.slice(0, index + 1))) {
    return `${trimmed.slice(0, index)}${separator}`;
  }
  return trimmed.slice(0, index) || '/';
}
