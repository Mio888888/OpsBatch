import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import type { CommandEntry, CommandParameter, ScriptEntry, QuickAction } from '../types';

interface BackendCommand {
  id: string;
  name: string;
  command: string;
  url: string;
  category: string;
  tags: string;
  risk: string;
  description: string;
  platform: string;
  parameters: string;
  starred: boolean;
  is_builtin: boolean;
}

interface BackendScript {
  id: string;
  name: string;
  language: string;
  category: string;
  tags: string;
  risk: string;
  description: string;
  content: string;
  parameters: string;
  url: string;
  platform: string;
  starred: boolean;
  is_builtin: boolean;
}

interface BackendQuickAction {
  id: string;
  name: string;
  command: string;
  category: string;
  parameters: string;
  sort_order: number;
  starred: number;
  description: string;
  tags: string;
  language: string;
  last_run_at: string;
  last_status: string;
}

function commandFromBackend(c: BackendCommand): CommandEntry {
  return {
    id: c.id,
    name: c.name,
    command: c.command,
    url: c.url || '',
    category: c.category,
    tags: JSON.parse(c.tags || '[]'),
    risk: c.risk as CommandEntry['risk'],
    description: c.description,
    platform: c.platform as CommandEntry['platform'],
    parameters: JSON.parse(c.parameters || '[]') as CommandParameter[],
    starred: c.starred,
    isBuiltin: c.is_builtin,
  };
}

function scriptFromBackend(s: BackendScript): ScriptEntry {
  return {
    id: s.id,
    name: s.name,
    language: s.language as ScriptEntry['language'],
    category: s.category,
    tags: JSON.parse(s.tags || '[]'),
    risk: s.risk as ScriptEntry['risk'],
    description: s.description,
    content: s.content,
    parameters: JSON.parse(s.parameters || '[]'),
    url: s.url || '',
    platform: s.platform as ScriptEntry['platform'],
    starred: s.starred,
    isBuiltin: s.is_builtin,
  };
}

function actionFromBackend(a: BackendQuickAction): QuickAction {
  return {
    id: a.id,
    name: a.name,
    command: a.command,
    category: a.category,
    parameters: JSON.parse(a.parameters || '[]'),
    order: a.sort_order,
    starred: a.starred === 1,
    description: a.description || '',
    tags: JSON.parse(a.tags || '[]'),
    language: (a.language || 'shell') as QuickAction['language'],
    lastRunAt: a.last_run_at || '',
    lastStatus: a.last_status || '',
  };
}

function commandToBackend(c: Omit<CommandEntry, 'id' | 'isBuiltin'>) {
  return {
    name: c.name,
    command: c.command,
    url: c.url || '',
    category: c.category,
    tags: JSON.stringify(c.tags),
    risk: c.risk,
    description: c.description,
    platform: c.platform,
    parameters: JSON.stringify(c.parameters),
  };
}

function scriptToBackend(s: Omit<ScriptEntry, 'id' | 'isBuiltin'>) {
  return {
    name: s.name,
    language: s.language,
    category: s.category,
    tags: JSON.stringify(s.tags),
    risk: s.risk,
    description: s.description,
    content: s.content,
    parameters: JSON.stringify(s.parameters),
    url: s.url || '',
    platform: s.platform,
  };
}

function actionToBackend(a: Omit<QuickAction, 'id' | 'order' | 'starred' | 'lastRunAt' | 'lastStatus'>) {
  return {
    name: a.name,
    command: a.command,
    category: a.category,
    parameters: JSON.stringify(a.parameters || []),
    description: a.description || '',
    tags: JSON.stringify(a.tags || []),
    language: a.language || 'shell',
  };
}

interface LibraryState {
  commands: CommandEntry[];
  scripts: ScriptEntry[];
  quickActions: QuickAction[];
  searchQuery: string;
  selectedCategory: string | null;

  loadCommands: () => Promise<void>;
  addCommand: (cmd: Omit<CommandEntry, 'id' | 'isBuiltin'>) => Promise<void>;
  updateCommand: (cmd: CommandEntry) => Promise<void>;
  deleteCommand: (id: string) => Promise<void>;
  toggleStarCommand: (id: string) => Promise<void>;

  loadScripts: () => Promise<void>;
  addScript: (script: Omit<ScriptEntry, 'id' | 'isBuiltin'>) => Promise<void>;
  updateScript: (script: ScriptEntry) => Promise<void>;
  deleteScript: (id: string) => Promise<void>;
  toggleStarScript: (id: string) => Promise<void>;

  loadQuickActions: () => Promise<void>;
  addQuickAction: (action: Omit<QuickAction, 'id' | 'order' | 'starred' | 'lastRunAt' | 'lastStatus'>) => Promise<void>;
  updateQuickAction: (action: QuickAction) => Promise<void>;
  deleteQuickAction: (id: string) => Promise<void>;
  toggleStarQuickAction: (id: string) => Promise<void>;

  setSearchQuery: (q: string) => void;
  setSelectedCategory: (c: string | null) => void;
}

export const useLibraryStore = create<LibraryState>((set, get) => ({
  commands: [],
  scripts: [],
  quickActions: [],
  searchQuery: '',
  selectedCategory: null,

  loadCommands: async () => {
    try {
      const raw = await invoke<BackendCommand[]>('list_commands');
      const commands = raw.map(commandFromBackend);
      set({ commands });
    } catch {
      set({ commands: [] });
    }
  },

  addCommand: async (cmd) => {
    const backend = commandToBackend(cmd);
    await invoke('add_command', { command: backend });
    await get().loadCommands();
  },

  updateCommand: async (cmd) => {
    const backend = {
      id: cmd.id,
      name: cmd.name,
      command: cmd.command,
      url: cmd.url || '',
      category: cmd.category,
      tags: JSON.stringify(cmd.tags),
      risk: cmd.risk,
      description: cmd.description,
      platform: cmd.platform,
      parameters: JSON.stringify(cmd.parameters),
      starred: cmd.starred,
      is_builtin: cmd.isBuiltin,
    };
    await invoke('update_command', { command: backend });
    await get().loadCommands();
  },

  deleteCommand: async (id) => {
    await invoke('delete_command', { id });
    await get().loadCommands();
  },

  toggleStarCommand: async (id) => {
    await invoke('toggle_star_command', { id });
    await get().loadCommands();
  },

  loadScripts: async () => {
    try {
      const raw = await invoke<BackendScript[]>('list_scripts');
      const scripts = raw.map(scriptFromBackend);
      set({ scripts });
    } catch {
      set({ scripts: [] });
    }
  },

  addScript: async (script) => {
    const backend = scriptToBackend(script);
    await invoke('add_script', { script: backend });
    await get().loadScripts();
  },

  updateScript: async (script) => {
    const backend = {
      id: script.id,
      name: script.name,
      language: script.language,
      category: script.category,
      tags: JSON.stringify(script.tags),
      risk: script.risk,
      description: script.description,
      content: script.content,
      parameters: JSON.stringify(script.parameters),
      url: script.url || '',
      platform: script.platform,
      starred: script.starred,
      is_builtin: script.isBuiltin,
    };
    await invoke('update_script', { script: backend });
    await get().loadScripts();
  },

  deleteScript: async (id) => {
    await invoke('delete_script', { id });
    await get().loadScripts();
  },

  toggleStarScript: async (id) => {
    await invoke('toggle_star_script', { id });
    await get().loadScripts();
  },

  loadQuickActions: async () => {
    try {
      const raw = await invoke<BackendQuickAction[]>('list_quick_actions');
      const quickActions = raw.map(actionFromBackend);
      set({ quickActions });
    } catch {
      set({ quickActions: [] });
    }
  },

  addQuickAction: async (action) => {
    const backend = actionToBackend(action);
    await invoke('add_quick_action', { action: backend });
    await get().loadQuickActions();
  },

  updateQuickAction: async (action) => {
    const backend = {
      id: action.id,
      name: action.name,
      command: action.command,
      category: action.category,
      parameters: JSON.stringify(action.parameters || []),
      sort_order: action.order,
      description: action.description || '',
      tags: JSON.stringify(action.tags || []),
      language: action.language || 'shell',
    };
    await invoke('update_quick_action', { action: backend });
    await get().loadQuickActions();
  },

  deleteQuickAction: async (id) => {
    await invoke('delete_quick_action', { id });
    await get().loadQuickActions();
  },

  toggleStarQuickAction: async (id) => {
    await invoke('toggle_star_quick_action', { id });
    await get().loadQuickActions();
  },

  setSearchQuery: (q) => set({ searchQuery: q }),
  setSelectedCategory: (c) => set({ selectedCategory: c }),
}));
