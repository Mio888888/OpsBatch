export function basenameFromPath(path: string, fallback = 'file'): string {
  const trimmed = path.replace(/[\\/]+$/, '');
  if (!trimmed) return fallback;
  const parts = trimmed.split(/[\\/]/);
  return parts[parts.length - 1] || fallback;
}
