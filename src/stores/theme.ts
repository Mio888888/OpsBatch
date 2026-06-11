import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { emit, listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import type { ITheme } from '@xterm/xterm';
import { allThemes, darkThemes, lightThemes, accentColors, getThemeById, getAccentById } from '../data/themes';
import type { ThemePreviewMeta, AccentColorMeta } from '../data/themes';
import { logHandledError } from '../utils/globalLogger';

export interface ThemeState {
  themeMode: 'system' | 'manual';
  terminalThemeId: string;
  terminalThemeTone: 'dark' | 'light';
  accentColorId: string;
  terminalFontSize: number;
  terminalFontFamily: string;
  terminalScrollback: number;
  reduceMotion: boolean;
  loaded: boolean;
}

const LEGACY_TERMINAL_FONT_FAMILY = '"Cascadia Code", Menlo, Monaco, "Courier New", monospace';
export const DEFAULT_TERMINAL_FONT_FAMILY =
  '"Cascadia Code", Consolas, "Lucida Console", Menlo, Monaco, "Courier New", monospace';

const defaults: ThemeState = {
  themeMode: 'manual',
  terminalThemeId: 'tokyo-night-storm',
  terminalThemeTone: 'dark',
  accentColorId: 'blue',
  terminalFontSize: 14,
  terminalFontFamily: DEFAULT_TERMINAL_FONT_FAMILY,
  terminalScrollback: 2000,
  reduceMotion: false,
  loaded: false,
};

export const useThemeStore = create<ThemeState>(() => ({ ...defaults }));

const THEME_SETTINGS_CHANGED_EVENT = 'theme-settings-changed';

const listeners = new Set<() => void>();
let themeEventListenerRegistered = false;
let osAppearanceListenerRegistered = false;

export function onThemeChange(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

function notify() {
  listeners.forEach((fn) => fn());
}

function getThemeEventPayload(): Omit<ThemeState, 'loaded'> & { sourceWindowLabel: string } {
  const s = useThemeStore.getState();
  return {
    themeMode: s.themeMode,
    terminalThemeId: s.terminalThemeId,
    terminalThemeTone: s.terminalThemeTone,
    accentColorId: s.accentColorId,
    terminalFontSize: s.terminalFontSize,
    terminalFontFamily: s.terminalFontFamily,
    terminalScrollback: s.terminalScrollback,
    reduceMotion: s.reduceMotion,
    sourceWindowLabel: getCurrentWindow().label,
  };
}

function applyThemeSettings(settings: Omit<ThemeState, 'loaded'>) {
  useThemeStore.setState({
    themeMode: settings.themeMode || defaults.themeMode,
    terminalThemeId: settings.terminalThemeId || defaults.terminalThemeId,
    terminalThemeTone: settings.terminalThemeTone || defaults.terminalThemeTone,
    accentColorId: settings.accentColorId || defaults.accentColorId,
    terminalFontSize: Number.isFinite(settings.terminalFontSize) ? settings.terminalFontSize : defaults.terminalFontSize,
    terminalFontFamily: normalizeTerminalFontFamily(settings.terminalFontFamily),
    terminalScrollback: Number.isFinite(settings.terminalScrollback) ? settings.terminalScrollback : defaults.terminalScrollback,
    reduceMotion: settings.reduceMotion ?? defaults.reduceMotion,
    loaded: true,
  });
  applyThemeToDOM();
  notify();
}

export function registerThemeSettingsSync() {
  if (themeEventListenerRegistered) return;
  themeEventListenerRegistered = true;

  const currentWindowLabel = getCurrentWindow().label;
  listen<Omit<ThemeState, 'loaded'> & { sourceWindowLabel?: string }>(THEME_SETTINGS_CHANGED_EVENT, (event) => {
    if (event.payload?.sourceWindowLabel === currentWindowLabel) return;
    applyThemeSettings(event.payload);
  }).catch((e) => {
    themeEventListenerRegistered = false;
    console.warn('[theme] listen theme settings changed failed:', e);
  });

  // Register OS appearance and motion listeners (once)
  if (!osAppearanceListenerRegistered) {
    osAppearanceListenerRegistered = true;
    const colorSchemeMql = window.matchMedia('(prefers-color-scheme: dark)');
    colorSchemeMql.addEventListener('change', () => {
      if (useThemeStore.getState().themeMode === 'system') {
        applyThemeToDOM();
        notify();
      }
    });

    const motionMql = window.matchMedia('(prefers-reduced-motion: reduce)');
    motionMql.addEventListener('change', () => {
      applyThemeToDOM();
      notify();
    });
  }
}

export async function loadThemeSettings() {
  try {
    const settings = await invoke<Record<string, string>>('get_general_settings');
    useThemeStore.setState({
      themeMode: (settings.themeMode as ThemeState['themeMode']) || defaults.themeMode,
      terminalThemeId: settings.terminalThemeId || defaults.terminalThemeId,
      terminalThemeTone: (settings.terminalThemeTone as ThemeState['terminalThemeTone']) || defaults.terminalThemeTone,
      accentColorId: settings.accentColorId || defaults.accentColorId,
      terminalFontSize: settings.terminalFontSize ? parseInt(settings.terminalFontSize) : defaults.terminalFontSize,
      terminalFontFamily: normalizeTerminalFontFamily(settings.terminalFontFamily),
      terminalScrollback: settings.terminalScrollback ? parseInt(settings.terminalScrollback) : defaults.terminalScrollback,
      reduceMotion: settings.reduceMotion === 'true',
      loaded: true,
    });
  } catch (error) {
    void logHandledError('theme.loadSettings', error, 'warn');
    useThemeStore.setState({ loaded: true });
  }
  applyThemeToDOM();
  notify();
}

export async function patchTheme(patch: Partial<Omit<ThemeState, 'loaded'>>) {
  useThemeStore.setState(patch);
  const s = useThemeStore.getState();
  let saved = false;
  try {
    await invoke('save_general_settings', {
      settings: {
        themeMode: s.themeMode,
        terminalThemeId: s.terminalThemeId,
        terminalThemeTone: s.terminalThemeTone,
        accentColorId: s.accentColorId,
        terminalFontSize: String(s.terminalFontSize),
        terminalFontFamily: s.terminalFontFamily,
        terminalScrollback: String(s.terminalScrollback),
        reduceMotion: String(s.reduceMotion),
      },
    });
    saved = true;
  } catch (error) {
    void logHandledError('theme.saveSettings', error, 'warn');
  }
  applyThemeToDOM();
  notify();
  if (saved) {
    emit(THEME_SETTINGS_CHANGED_EVENT, getThemeEventPayload()).catch((e) => {
      console.warn('[theme] emit theme settings changed failed:', e);
    });
  }
}

/** Detect the current OS color scheme preference. */
function getOsPrefersDark(): boolean {
  return window.matchMedia('(prefers-color-scheme: dark)').matches;
}

/** Detect whether the OS asks apps to reduce non-essential motion. */
function getOsPrefersReducedMotion(): boolean {
  return window.matchMedia('(prefers-reduced-motion: reduce)').matches;
}

export function shouldReduceMotion(): boolean {
  return useThemeStore.getState().reduceMotion || getOsPrefersReducedMotion();
}

export function normalizeTerminalFontFamily(value?: string | null): string {
  const trimmed = value?.trim();
  if (trimmed === LEGACY_TERMINAL_FONT_FAMILY) return defaults.terminalFontFamily;
  return trimmed || defaults.terminalFontFamily;
}

/** Pick a theme that matches the given tone, preferring the stored ID if its tone matches. */
function resolveThemeForTone(preferredTone: 'dark' | 'light', storedId: string): ThemePreviewMeta {
  const stored = getThemeById(storedId);
  if (stored && stored.tone === preferredTone) return stored;
  const pool = preferredTone === 'dark' ? darkThemes : lightThemes;
  return pool[0];
}

export function applyThemeToDOM() {
  const s = useThemeStore.getState();
  const root = document.documentElement;
  const accent = getAccentById(s.accentColorId) || accentColors[0];

  // Determine which theme and tone to actually apply
  let isDark: boolean;
  let theme: ThemePreviewMeta | undefined;

  if (s.themeMode === 'system') {
    isDark = getOsPrefersDark();
    const osTone = isDark ? 'dark' : 'light';
    theme = resolveThemeForTone(osTone, s.terminalThemeId);
  } else {
    isDark = s.terminalThemeTone === 'dark';
    theme = getThemeById(s.terminalThemeId);
  }

  root.style.setProperty('--color-primary', accent.base);
  root.style.setProperty('--color-primary-strong', accent.strong);

  if (theme) {
    const bg = theme.preview.background;
    const fg = theme.preview.lineStrong;
    const med = theme.preview.lineMedium;
    const weak = theme.preview.lineWeak;
    const acc = theme.preview.accent;

    root.style.setProperty('--terminal-bg', bg);
    root.style.setProperty('--terminal-fg', fg);
    root.style.setProperty('--terminal-muted', med);
    root.style.setProperty('--terminal-weak', weak);
    root.style.setProperty('--terminal-accent', acc);

    const rgb = hexToRgb(bg);
    root.style.setProperty('--terminal-shell-bg', `rgba(${rgb}, 0.85)`);
    root.style.setProperty('--terminal-tab-bg', `rgba(${rgb}, 0.92)`);
    root.style.setProperty('--terminal-border', isDark ? 'rgba(255, 255, 255, 0.06)' : 'rgba(0, 0, 0, 0.08)');
    root.style.setProperty('--terminal-panel-bg', isDark ? darken(bg, 8) : lighten(bg, 4));

    root.style.setProperty('--color-text', fg);
    root.style.setProperty('--color-text-secondary', med);
    root.style.setProperty('--color-text-muted', weak);
    root.style.setProperty('--color-border', theme.ui.border);
    root.style.setProperty('--color-border-strong', theme.ui.borderStrong);
    root.style.setProperty('--color-surface', theme.ui.cardBg);
    root.style.setProperty('--color-surface-soft', theme.ui.mainBg);
    root.style.setProperty('--color-workbench-bg', theme.ui.pageBg);
    root.style.setProperty('--color-settings-page-bg', theme.ui.pageBg);
    root.style.setProperty('--color-settings-sidebar-bg', theme.ui.sidebarBg);
    root.style.setProperty('--color-settings-main-bg', theme.ui.mainBg);
    root.style.setProperty('--color-settings-card-bg', theme.ui.cardBg);
    root.style.setProperty('--color-settings-header-bg', theme.ui.headerBg);
    root.style.setProperty('--color-settings-border', theme.ui.border);
    root.style.setProperty('--color-settings-input-bg', theme.ui.inputBg);
    root.style.setProperty('--color-settings-input-border', theme.ui.inputBorder);
    root.style.setProperty('--color-settings-nav-hover', theme.ui.navHover);
    root.style.setProperty('--color-settings-nav-active-bg', `rgba(${hexToRgb(accent.base)}, ${isDark ? '0.1' : '0.08'})`);
    root.style.setProperty('--color-settings-nav-active-border', accent.base);
  }

  root.dataset.themeMode = s.themeMode;
  root.dataset.themeTone = isDark ? 'dark' : 'light';
  root.dataset.reduceMotion = shouldReduceMotion() ? 'true' : 'false';

  const win = getCurrentWindow();
  if (s.themeMode === 'system') {
    win.setTheme(null).catch((e) => console.warn('[theme] setTheme failed:', e));
  } else {
    win.setTheme(isDark ? 'dark' : 'light').catch((e) => console.warn('[theme] setTheme failed:', e));
  }
  if (theme) {
    win.setBackgroundColor(theme.ui.pageBg).catch((e) => console.warn('[theme] setBackgroundColor failed:', e));
  }
}

function hexToRgb(hex: string): string {
  const h = hex.replace('#', '');
  const r = parseInt(h.substring(0, 2), 16);
  const g = parseInt(h.substring(2, 4), 16);
  const b = parseInt(h.substring(4, 6), 16);
  return `${r}, ${g}, ${b}`;
}

function darken(hex: string, amount: number): string {
  const h = hex.replace('#', '');
  const r = Math.max(0, parseInt(h.substring(0, 2), 16) - amount);
  const g = Math.max(0, parseInt(h.substring(2, 4), 16) - amount);
  const b = Math.max(0, parseInt(h.substring(4, 6), 16) - amount);
  return `#${r.toString(16).padStart(2, '0')}${g.toString(16).padStart(2, '0')}${b.toString(16).padStart(2, '0')}`;
}

function lighten(hex: string, amount: number): string {
  const h = hex.replace('#', '');
  const r = Math.min(255, parseInt(h.substring(0, 2), 16) + amount);
  const g = Math.min(255, parseInt(h.substring(2, 4), 16) + amount);
  const b = Math.min(255, parseInt(h.substring(4, 6), 16) + amount);
  return `#${r.toString(16).padStart(2, '0')}${g.toString(16).padStart(2, '0')}${b.toString(16).padStart(2, '0')}`;
}

export function getCurrentTerminalTheme(): ThemePreviewMeta | undefined {
  const s = useThemeStore.getState();
  if (s.themeMode === 'system') {
    const osTone = getOsPrefersDark() ? 'dark' : 'light';
    return resolveThemeForTone(osTone, s.terminalThemeId);
  }
  return getThemeById(s.terminalThemeId);
}

export function getCurrentTerminalAppearance(): { theme: ITheme; fontSize: number; fontFamily: string; scrollback: number } {
  const s = useThemeStore.getState();
  const theme = getCurrentTerminalTheme();
  return {
    theme: { ...(theme?.xterm ?? {}) },
    fontSize: s.terminalFontSize,
    fontFamily: normalizeTerminalFontFamily(s.terminalFontFamily),
    scrollback: s.terminalScrollback,
  };
}

export function getCurrentAccent(): AccentColorMeta {
  return getAccentById(useThemeStore.getState().accentColorId) || accentColors[0];
}

export { type ThemePreviewMeta, type AccentColorMeta, allThemes, accentColors };
