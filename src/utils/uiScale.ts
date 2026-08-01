export const UI_SCALE_MIN = 90;
export const UI_SCALE_MAX = 150;
export const WINDOWS_DEFAULT_UI_SCALE = 115;
export const OTHER_PLATFORM_DEFAULT_UI_SCALE = 100;

export const UI_SCALE_OPTIONS = [100, 110, 115, 125, 140] as const;

export function normalizeUiScale(value: unknown, fallback = OTHER_PLATFORM_DEFAULT_UI_SCALE): number {
  const parsed = typeof value === 'number' ? value : Number.parseInt(String(value ?? ''), 10);
  if (!Number.isFinite(parsed)) return fallback;
  return Math.min(UI_SCALE_MAX, Math.max(UI_SCALE_MIN, Math.round(parsed / 5) * 5));
}

export function defaultUiScaleForPlatform(platformText?: string): number {
  const detectedPlatform = platformText
    ?? (typeof navigator === 'undefined' ? '' : `${navigator.userAgent} ${navigator.platform}`);
  return detectedPlatform.toLowerCase().includes('win')
    ? WINDOWS_DEFAULT_UI_SCALE
    : OTHER_PLATFORM_DEFAULT_UI_SCALE;
}
