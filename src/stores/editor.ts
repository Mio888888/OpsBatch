import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';

export interface TreeNode {
  name: string;
  path: string;
  is_dir: boolean;
  children: TreeNode[];
}

interface EditorState {
  hostId: string;
  filePath: string;
  dirPath: string;
  mode: 'file' | 'dir';
  fileName: string;
  content: string;
  originalContent: string;
  loading: boolean;
  saving: boolean;
  dirty: boolean;
  tree: TreeNode | null;
  treeLoading: boolean;
  currentFilePath: string | null;
  currentFileName: string | null;
  error: string | null;

  initFile: (hostId: string, filePath: string) => Promise<void>;
  initDir: (hostId: string, dirPath: string) => Promise<void>;
  openFileInDir: (filePath: string) => Promise<void>;
  setContent: (content: string) => void;
  save: () => Promise<void>;
  loadTree: (dirPath: string) => Promise<void>;
}

export const useEditorStore = create<EditorState>((set, get) => ({
  hostId: '',
  filePath: '',
  dirPath: '',
  mode: 'file',
  fileName: '',
  content: '',
  originalContent: '',
  loading: false,
  saving: false,
  dirty: false,
  tree: null,
  treeLoading: false,
  currentFilePath: null,
  currentFileName: null,
  error: null,

  initFile: async (hostId, filePath) => {
    const name = filePath.split('/').pop() || filePath;
    set({
      loading: true,
      treeLoading: false,
      error: null,
      hostId,
      filePath,
      mode: 'file',
      fileName: name,
      content: '',
      originalContent: '',
      dirty: false,
      tree: null,
      currentFilePath: null,
      currentFileName: null,
    });
    try {
      const data = await invoke<Array<number>>('sftp_read_file', {
        hostId,
        path: filePath,
        maxSize: 10 * 1024 * 1024,
      });
      const bytes = new Uint8Array(data);
      const text = new TextDecoder('utf-8', { fatal: false }).decode(bytes);
      set({
        content: text,
        originalContent: text,
        fileName: name,
        loading: false,
        dirty: false,
        currentFilePath: filePath,
        currentFileName: name,
      });
    } catch (e: unknown) {
      set({ error: String(e), loading: false });
    }
  },

  initDir: async (hostId, dirPath) => {
    const name = dirPath.replace(/\/$/, '').split('/').pop() || dirPath;
    set({
      loading: false,
      treeLoading: true,
      error: null,
      hostId,
      dirPath,
      mode: 'dir',
      filePath: '',
      fileName: name,
      content: '',
      originalContent: '',
      dirty: false,
      tree: null,
      currentFilePath: null,
      currentFileName: null,
    });
    try {
      const tree = await invoke<TreeNode>('sftp_read_file_tree', {
        hostId,
        path: dirPath,
        maxDepth: 3,
      });
      set({ tree, fileName: name, treeLoading: false });
    } catch (e: unknown) {
      set({ error: String(e), treeLoading: false });
    }
  },

  openFileInDir: async (filePath) => {
    const { hostId } = get();
    if (!hostId) return;
    set({ loading: true, error: null });
    const name = filePath.split('/').pop() || filePath;
    try {
      const data = await invoke<Array<number>>('sftp_read_file', {
        hostId,
        path: filePath,
        maxSize: 10 * 1024 * 1024,
      });
      const bytes = new Uint8Array(data);
      const text = new TextDecoder('utf-8', { fatal: false }).decode(bytes);
      set({
        content: text,
        originalContent: text,
        loading: false,
        dirty: false,
        currentFilePath: filePath,
        currentFileName: name,
      });
    } catch (e: unknown) {
      set({ error: String(e), loading: false });
    }
  },

  setContent: (content) => {
    const { originalContent } = get();
    set({ content, dirty: content !== originalContent });
  },

  save: async () => {
    const { hostId, mode, filePath, currentFilePath, content } = get();
    const savePath = mode === 'dir' ? currentFilePath : filePath;
    if (!hostId || !savePath) {
      return;
    }
    set({ saving: true, error: null });
    try {
      await invoke('sftp_write_file', { hostId, path: savePath, content });
      set({ saving: false, dirty: false, originalContent: content });
    } catch (e: unknown) {
      console.error('[Editor:save] failed', e);
      set({ saving: false, error: String(e) });
    }
  },

  loadTree: async (dirPath) => {
    const { hostId } = get();
    if (!hostId) return;
    set({ treeLoading: true });
    try {
      const tree = await invoke<TreeNode>('sftp_read_file_tree', {
        hostId,
        path: dirPath,
        maxDepth: 3,
      });
      set({ tree, treeLoading: false });
    } catch (e: unknown) {
      set({ treeLoading: false });
    }
  },
}));
