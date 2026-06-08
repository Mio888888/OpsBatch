import { create } from 'zustand';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { translateText } from '../i18n';
import { useLanguageStore } from './language';

export interface LogEntry {
  timestamp: string;
  level: string;
  source: string;
  message: string;
  origin: 'backend' | 'frontend';
}

const MAX_LOG_ENTRIES = 2000;

interface LogState {
  logs: LogEntry[];
  loading: boolean;
  init: () => void;
  clear: () => void;
}

let _initPromise: Promise<void> | null = null;

export const useLogStore = create<LogState>((set) => ({
  logs: [],
  loading: true,

  init: () => {
    if (_initPromise) return;
    _initPromise = (async () => {
      // 1. Set up listener for real-time logs
      const unlisten: UnlistenFn = await listen<LogEntry>('global-log', (event) => {
        set((s) => ({
          logs: [event.payload, ...s.logs.slice(0, MAX_LOG_ENTRIES - 1)],
        }));
      });

      // 2. Load history from DB (newest first)
      try {
        const history = await invoke<LogEntry[]>('get_log_history', { limit: 500 });
        set({ logs: history.reverse(), loading: false });
      } catch {
        set({ loading: false });
      }

      // 3. Emit a connection log to verify the pipeline
      try {
        await invoke('ping_log', { message: translateText(useLanguageStore.getState().language, 'log.connectedMessage') });
      } catch {
        // ignore
      }

      // Prevent vite hot-reload from creating duplicate listeners
      if (import.meta.hot) {
        import.meta.hot!.dispose(unlisten);
      }
    })();
  },

  clear: () => {
    set({ logs: [] });
  },
}));
