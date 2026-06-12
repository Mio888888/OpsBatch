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

const MAX_LOG_ENTRIES = 600;

interface LogState {
  logs: LogEntry[];
  loading: boolean;
  init: () => void;
  clear: () => Promise<void>;
}

let _initPromise: Promise<void> | null = null;
let pendingLogs: LogEntry[] = [];
let flushFrame: number | null = null;

function enqueueLog(entry: LogEntry, set: (fn: (state: LogState) => Partial<LogState>) => void) {
  pendingLogs.push(entry);
  if (pendingLogs.length > 200) {
    pendingLogs = pendingLogs.slice(-200);
  }
  if (flushFrame !== null) return;
  flushFrame = requestAnimationFrame(() => {
    flushFrame = null;
    const batch = pendingLogs;
    pendingLogs = [];
    set((s) => ({
      logs: [...batch.reverse(), ...s.logs].slice(0, MAX_LOG_ENTRIES),
    }));
  });
}

export const useLogStore = create<LogState>((set) => ({
  logs: [],
  loading: true,

  init: () => {
    if (_initPromise) return;
    _initPromise = (async () => {
      // 1. Set up listener for real-time logs
      const unlisten: UnlistenFn = await listen<LogEntry>('global-log', (event) => {
        enqueueLog(event.payload, set);
      });

      // 2. Load history from DB (newest first)
      try {
        const history = await invoke<LogEntry[]>('get_log_history', { limit: 200 });
        set({ logs: history.reverse().slice(0, MAX_LOG_ENTRIES), loading: false });
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

  clear: async () => {
    await invoke('clear_log_history');
    set({ logs: [] });
  },
}));
