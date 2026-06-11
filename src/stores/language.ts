import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { emit, listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import type { AppLanguage, LanguageMode } from '../i18n/language';
import { resolveLanguage } from '../i18n/language';
import { logHandledError } from '../utils/globalLogger';

export interface LanguageState {
  languageMode: LanguageMode;
  language: AppLanguage;
  loaded: boolean;
}

const defaults: LanguageState = {
  languageMode: 'system',
  language: resolveLanguage('system'),
  loaded: false,
};

export const useLanguageStore = create<LanguageState>(() => ({ ...defaults }));

const LANGUAGE_SETTINGS_CHANGED_EVENT = 'language-settings-changed';
const GENERAL_SETTINGS_LANGUAGE_KEY = 'languageMode';

const listeners = new Set<() => void>();
let languageEventListenerRegistered = false;
let systemLanguageListenerRegistered = false;

interface LanguageSettingsPayload {
  languageMode: LanguageMode;
  sourceWindowLabel?: string;
}

export function onLanguageChange(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

function notify() {
  listeners.forEach((fn) => fn());
}

function isLanguageMode(value: string | undefined): value is LanguageMode {
  return value === 'system' || value === 'zh-CN' || value === 'en-US';
}

function getLanguageEventPayload(): LanguageSettingsPayload {
  return {
    languageMode: useLanguageStore.getState().languageMode,
    sourceWindowLabel: getCurrentWindow().label,
  };
}

function applyLanguageMode(languageMode: LanguageMode, loaded = true) {
  const language = resolveLanguage(languageMode);
  useLanguageStore.setState({ languageMode, language, loaded });
  applyLanguageToDOM();
  notify();
}

function refreshSystemLanguage() {
  const state = useLanguageStore.getState();
  if (state.languageMode !== 'system') return;
  const language = resolveLanguage('system');
  if (language === state.language) return;
  useLanguageStore.setState({ language });
  applyLanguageToDOM();
  notify();
}

export function registerLanguageSettingsSync() {
  if (languageEventListenerRegistered) return;
  languageEventListenerRegistered = true;

  const currentWindowLabel = getCurrentWindow().label;
  listen<LanguageSettingsPayload>(LANGUAGE_SETTINGS_CHANGED_EVENT, (event) => {
    if (event.payload?.sourceWindowLabel === currentWindowLabel) return;
    if (!isLanguageMode(event.payload?.languageMode)) return;
    applyLanguageMode(event.payload.languageMode);
  }).catch((e) => {
    languageEventListenerRegistered = false;
    console.warn('[language] listen language settings changed failed:', e);
  });

  if (!systemLanguageListenerRegistered) {
    systemLanguageListenerRegistered = true;
    window.addEventListener('languagechange', refreshSystemLanguage);
  }
}

export async function loadLanguageSettings() {
  try {
    const settings = await invoke<Record<string, string>>('get_general_settings');
    const languageMode = isLanguageMode(settings[GENERAL_SETTINGS_LANGUAGE_KEY])
      ? settings[GENERAL_SETTINGS_LANGUAGE_KEY]
      : defaults.languageMode;
    applyLanguageMode(languageMode);
  } catch (error) {
    void logHandledError('language.loadSettings', error, 'warn');
    applyLanguageMode(defaults.languageMode);
  }
}

export async function patchLanguage(languageMode: LanguageMode) {
  applyLanguageMode(languageMode);
  let saved = false;
  try {
    await invoke('save_general_settings', {
      settings: {
        [GENERAL_SETTINGS_LANGUAGE_KEY]: languageMode,
      },
    });
    saved = true;
  } catch (error) {
    void logHandledError('language.saveSettings', error, 'warn');
  }

  if (saved) {
    emit(LANGUAGE_SETTINGS_CHANGED_EVENT, getLanguageEventPayload()).catch((e) => {
      console.warn('[language] emit language settings changed failed:', e);
    });
  }
}

export function applyLanguageToDOM() {
  const state = useLanguageStore.getState();
  const root = document.documentElement;
  root.lang = state.language;
  root.dataset.languageMode = state.languageMode;
}
