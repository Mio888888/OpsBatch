import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export interface FileEntry {
  name: string;
  path: string;
  is_dir: boolean;
  is_symlink: boolean;
  size: number;
  modified: string | null;
  permissions: number | null;
}

export interface TransferItem {
  id: string;
  direction: 'upload' | 'download';
  fileName: string;
  localPath: string;
  remotePath: string;
  fileSize: number;
  status: 'pending' | 'transferring' | 'done' | 'error';
  progress: number;
  speed: number;
  durationMs: number | null;
  error: string | null;
}

export interface SftpBookmark {
  id: string;
  name: string;
  path: string;
  side: 'local' | 'remote';
}

interface SftpState {
  remotePath: string;
  localPath: string;
  remoteEntries: FileEntry[];
  localEntries: FileEntry[];
  remoteLoading: boolean;
  localLoading: boolean;
  remoteError: string | null;
  localError: string | null;
  selectedRemote: FileEntry | null;
  selectedLocal: FileEntry | null;
  transfers: TransferItem[];
  bookmarks: SftpBookmark[];
  previewFile: { side: 'local' | 'remote'; entry: FileEntry; data: string | null } | null;
  previewLoading: boolean;
  panelOpen: boolean;

  initSession: (hostId: string) => Promise<string>;
  closeSession: (hostId: string) => Promise<void>;
  navigateRemote: (hostId: string, path: string) => Promise<void>;
  navigateLocal: (path: string) => Promise<void>;
  authorizeLocalDirectory: () => Promise<string | null>;
  goRemoteUp: (hostId: string) => Promise<void>;
  goLocalUp: () => Promise<void>;
  setSelectedRemote: (entry: FileEntry | null) => void;
  setSelectedLocal: (entry: FileEntry | null) => void;
  refreshRemote: (hostId: string) => Promise<void>;
  refreshLocal: () => Promise<void>;
  upload: (hostId: string, localPath: string, remoteDir: string) => Promise<void>;
  download: (hostId: string, remotePath: string, localDir: string) => Promise<void>;
  remoteMkdir: (hostId: string, path: string) => Promise<void>;
  remoteRename: (hostId: string, oldPath: string, newPath: string) => Promise<void>;
  remoteRemove: (hostId: string, path: string, isDir: boolean) => Promise<void>;
  localMkdir: (path: string) => Promise<void>;
  localRename: (oldPath: string, newPath: string) => Promise<void>;
  localRemove: (path: string) => Promise<void>;
  previewRemote: (hostId: string, entry: FileEntry) => Promise<void>;
  previewLocal: (entry: FileEntry) => Promise<void>;
  closePreview: () => void;
  extractArchive: (hostId: string, archivePath: string, targetDir: string) => Promise<void>;
  setPanelOpen: (open: boolean) => void;
  addBookmark: (bookmark: SftpBookmark) => void;
  removeBookmark: (id: string) => void;
}

const STORAGE_KEY = 'sftp-bookmarks';

function loadBookmarks(): SftpBookmark[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

function saveBookmarks(bookmarks: SftpBookmark[]) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(bookmarks));
}

function applyTransferDone(
  transfers: TransferItem[],
  transferId: string,
  payload: { success: boolean; fileSize?: number; durationMs?: number; error?: string },
) {
  return transfers.map((t) =>
    t.id === transferId
      ? {
          ...t,
          status: payload.success ? 'done' as const : 'error' as const,
          progress: payload.success ? 100 : t.progress,
          fileSize: payload.fileSize || t.fileSize,
          durationMs: payload.durationMs || null,
          error: payload.error || null,
          speed: payload.durationMs ? (payload.fileSize || 0) / payload.durationMs : 0,
        }
      : t
  );
}

function applyTransferStart(
  transfers: TransferItem[],
  transferId: string,
  payload: { fileSize?: number },
) {
  return transfers.map((t) =>
    t.id === transferId ? { ...t, fileSize: payload.fileSize || t.fileSize } : t
  );
}

export const useSftpStore = create<SftpState>((set, get) => ({
  remotePath: '/',
  localPath: '',
  remoteEntries: [],
  localEntries: [],
  remoteLoading: false,
  localLoading: false,
  remoteError: null,
  localError: null,
  selectedRemote: null,
  selectedLocal: null,
  transfers: [],
  bookmarks: loadBookmarks(),
  previewFile: null,
  previewLoading: false,
  panelOpen: false,

  initSession: async (hostId) => {
    const caps = await invoke<{ can_write: boolean; home_dir: string }>('sftp_open', { hostId });
    set({ remotePath: caps.home_dir });
    return caps.home_dir;
  },

  closeSession: async (hostId) => {
    await invoke('sftp_close', { hostId });
  },

  navigateRemote: async (hostId, path) => {
    set({ remoteLoading: true, remoteError: null });
    try {
      const entries = await invoke<FileEntry[]>('sftp_list_dir', { hostId, path });
      set({
        remoteEntries: entries,
        remotePath: path,
        remoteLoading: false,
        selectedRemote: null,
      });
    } catch (e: unknown) {
      set({ remoteError: String(e), remoteLoading: false });
    }
  },

  navigateLocal: async (path) => {
    set({ localLoading: true, localError: null });
    try {
      const entries = await invoke<FileEntry[]>('local_list_dir', { path });
      set({
        localEntries: entries,
        localPath: path,
        localLoading: false,
        selectedLocal: null,
      });
    } catch (e: unknown) {
      set({ localError: String(e), localLoading: false });
    }
  },

  authorizeLocalDirectory: async () => {
    const path = await invoke<string | null>('local_authorize_directory');
    if (!path) return null;
    await get().navigateLocal(path);
    return path;
  },

  goRemoteUp: async (hostId) => {
    const { remotePath } = get();
    const parts = remotePath.replace(/\/$/, '').split('/');
    parts.pop();
    const parent = parts.join('/') || '/';
    await get().navigateRemote(hostId, parent);
  },

  goLocalUp: async () => {
    const { localPath } = get();
    const parts = localPath.replace(/\/$/, '').split('/');
    parts.pop();
    const parent = parts.join('/') || '/';
    await get().navigateLocal(parent);
  },

  setSelectedRemote: (entry) => set({ selectedRemote: entry }),
  setSelectedLocal: (entry) => set({ selectedLocal: entry }),

  refreshRemote: async (hostId) => {
    const { remotePath } = get();
    await get().navigateRemote(hostId, remotePath);
  },

  refreshLocal: async () => {
    const { localPath } = get();
    await get().navigateLocal(localPath);
  },

  upload: async (hostId, localFilePath, remoteDir) => {
    const fileName = localFilePath.split('/').pop() || 'file';
    const remotePath = remoteDir.endsWith('/')
      ? `${remoteDir}${fileName}`
      : `${remoteDir}/${fileName}`;
    const transferId = `upload-${Date.now()}-${Math.random().toString(36).slice(2)}`;

    const transfer: TransferItem = {
      id: transferId,
      direction: 'upload',
      fileName,
      localPath: localFilePath,
      remotePath,
      fileSize: 0,
      status: 'transferring',
      progress: 0,
      speed: 0,
      durationMs: null,
      error: null,
    };

    set((s) => ({ transfers: [...s.transfers, transfer] }));

    let settled = false;
    let unlistenDone: (() => void) | undefined;
    let unlistenStart: (() => void) | undefined;
    const cleanupListeners = () => {
      unlistenDone?.();
      unlistenStart?.();
      unlistenDone = undefined;
      unlistenStart = undefined;
    };
    const finishTransfer = (payload: { success: boolean; fileSize?: number; durationMs?: number; error?: string }) => {
      if (settled) return;
      settled = true;
      cleanupListeners();
      set((s) => ({ transfers: applyTransferDone(s.transfers, transferId, payload) }));
      if (payload.success) {
        get().refreshRemote(hostId);
      }
    };

    try {
      unlistenStart = await listen(`sftp-transfer:${transferId}:start`, (event) => {
        set((s) => ({
          transfers: applyTransferStart(s.transfers, transferId, event.payload as { fileSize?: number }),
        }));
      });
      unlistenDone = await listen(`sftp-transfer:${transferId}:done`, (event) => {
        finishTransfer(event.payload as { success: boolean; fileSize?: number; durationMs?: number; error?: string });
      });
    } catch (error: unknown) {
      finishTransfer({ success: false, error: String(error) });
      return;
    }

    invoke('sftp_upload', {
      hostId,
      localPath: localFilePath,
      remotePath,
      transferId,
    }).catch((error: unknown) => {
      finishTransfer({ success: false, error: String(error) });
    });
  },

  download: async (hostId, remoteFilePath, localDir) => {
    const fileName = remoteFilePath.split('/').pop() || 'file';
    const localPath = localDir.endsWith('/')
      ? `${localDir}${fileName}`
      : `${localDir}/${fileName}`;
    const transferId = `download-${Date.now()}-${Math.random().toString(36).slice(2)}`;

    const transfer: TransferItem = {
      id: transferId,
      direction: 'download',
      fileName,
      localPath,
      remotePath: remoteFilePath,
      fileSize: 0,
      status: 'transferring',
      progress: 0,
      speed: 0,
      durationMs: null,
      error: null,
    };

    set((s) => ({ transfers: [...s.transfers, transfer] }));

    let settled = false;
    let unlistenDone: (() => void) | undefined;
    let unlistenStart: (() => void) | undefined;
    const cleanupListeners = () => {
      unlistenDone?.();
      unlistenStart?.();
      unlistenDone = undefined;
      unlistenStart = undefined;
    };
    const finishTransfer = (payload: { success: boolean; fileSize?: number; durationMs?: number; error?: string }) => {
      if (settled) return;
      settled = true;
      cleanupListeners();
      set((s) => ({ transfers: applyTransferDone(s.transfers, transferId, payload) }));
      if (payload.success) {
        get().refreshLocal();
      }
    };

    try {
      unlistenStart = await listen(`sftp-transfer:${transferId}:start`, (event) => {
        set((s) => ({
          transfers: applyTransferStart(s.transfers, transferId, event.payload as { fileSize?: number }),
        }));
      });
      unlistenDone = await listen(`sftp-transfer:${transferId}:done`, (event) => {
        finishTransfer(event.payload as { success: boolean; fileSize?: number; durationMs?: number; error?: string });
      });
    } catch (error: unknown) {
      finishTransfer({ success: false, error: String(error) });
      return;
    }

    invoke('sftp_download', {
      hostId,
      remotePath: remoteFilePath,
      localPath,
      transferId,
    }).catch((error: unknown) => {
      finishTransfer({ success: false, error: String(error) });
    });
  },

  remoteMkdir: async (hostId, path) => {
    await invoke('sftp_mkdir', { hostId, path });
    await get().refreshRemote(hostId);
  },

  remoteRename: async (hostId, oldPath, newPath) => {
    await invoke('sftp_rename', { hostId, oldPath, newPath });
    await get().refreshRemote(hostId);
  },

  remoteRemove: async (hostId, path, isDir) => {
    if (isDir) {
      await invoke('sftp_rmdir', { hostId, path });
    } else {
      await invoke('sftp_remove', { hostId, path });
    }
    await get().refreshRemote(hostId);
  },

  localMkdir: async (path) => {
    await invoke('local_mkdir', { path });
    await get().refreshLocal();
  },

  localRename: async (oldPath, newPath) => {
    await invoke('local_rename', { oldPath, newPath });
    await get().refreshLocal();
  },

  localRemove: async (path) => {
    await invoke('local_remove', { path });
    await get().refreshLocal();
  },

  previewRemote: async (hostId, entry) => {
    if (entry.is_dir) return;
    set({ previewLoading: true, previewFile: { side: 'remote', entry, data: null } });
    try {
      const data = await invoke<Array<number>>('sftp_read_file', {
        hostId,
        path: entry.path,
        maxSize: 2 * 1024 * 1024,
      });
      const bytes = new Uint8Array(data);
      const blob = new Blob([bytes]);
      const reader = new FileReader();
      reader.onload = () => {
        const result = reader.result as string;
        const base64 = result.split(',')[1];
        set({ previewFile: { side: 'remote', entry, data: base64 }, previewLoading: false });
      };
      reader.onerror = () => {
        set({ previewLoading: false, previewFile: null });
      };
      reader.readAsDataURL(blob);
    } catch (e: unknown) {
      set({ previewLoading: false, previewFile: null });
    }
  },

  previewLocal: async (entry) => {
    if (entry.is_dir) return;
    set({ previewLoading: true, previewFile: { side: 'local', entry, data: null } });
    try {
      const data = await invoke<Array<number>>('local_read_file', {
        path: entry.path,
        maxSize: 2 * 1024 * 1024,
      });
      const bytes = new Uint8Array(data);
      const blob = new Blob([bytes]);
      const reader = new FileReader();
      reader.onload = () => {
        const result = reader.result as string;
        const base64 = result.split(',')[1];
        set({ previewFile: { side: 'local', entry, data: base64 }, previewLoading: false });
      };
      reader.onerror = () => {
        set({ previewLoading: false, previewFile: null });
      };
      reader.readAsDataURL(blob);
    } catch (e: unknown) {
      set({ previewLoading: false, previewFile: null });
    }
  },

  closePreview: () => set({ previewFile: null }),

  extractArchive: async (hostId, archivePath, targetDir) => {
    await invoke('sftp_extract_archive', { hostId, archivePath, targetDir });
    await get().refreshRemote(hostId);
  },

  setPanelOpen: (open) => set({ panelOpen: open }),

  addBookmark: (bookmark) => {
    const newBookmark = { ...bookmark, id: `bm-${Date.now()}-${Math.random().toString(36).slice(2)}` };
    set((s) => {
      const bookmarks = [...s.bookmarks, newBookmark];
      saveBookmarks(bookmarks);
      return { bookmarks };
    });
  },

  removeBookmark: (id) => {
    set((s) => {
      const bookmarks = s.bookmarks.filter((b) => b.id !== id);
      saveBookmarks(bookmarks);
      return { bookmarks };
    });
  },
}));
