import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';

export interface CanvasNode {
  id: string;
  type: string;
  name: string;
  config: string;
  x: number;
  y: number;
  enabled: boolean;
  width: number;
  height: number;
}

export interface WorkflowSettings {
  defaultNodeIntervalSeconds: number;
}

export interface Connection {
  id: string;
  fromId: string;
  toId: string;
  sourceHandle?: string;
}

export interface WorkflowRecord {
  id: string;
  name: string;
  description: string;
  nodes: CanvasNode[];
  connections: Connection[];
  settings: WorkflowSettings;
  status: 'draft' | 'ready';
  createdAt: string;
  updatedAt: string;
  selectedHostIds: string[];
}

interface WorkflowNodesPayload {
  nodes: CanvasNode[];
  settings: WorkflowSettings;
}

const DEFAULT_WORKFLOW_SETTINGS: WorkflowSettings = {
  defaultNodeIntervalSeconds: 0,
};

function normalizeWorkflowSettings(raw: unknown): WorkflowSettings {
  if (!raw || typeof raw !== 'object') return DEFAULT_WORKFLOW_SETTINGS;
  const settings = raw as Partial<WorkflowSettings>;
  const interval = Number(settings.defaultNodeIntervalSeconds);
  return {
    defaultNodeIntervalSeconds: Number.isFinite(interval) && interval > 0 ? interval : 0,
  };
}

export function parseWorkflowNodesPayload(raw: string): WorkflowNodesPayload {
  const parsed = JSON.parse(raw || '[]') as CanvasNode[] | WorkflowNodesPayload;
  if (Array.isArray(parsed)) {
    return { nodes: parsed, settings: DEFAULT_WORKFLOW_SETTINGS };
  }
  return {
    nodes: Array.isArray(parsed.nodes) ? parsed.nodes : [],
    settings: normalizeWorkflowSettings(parsed.settings),
  };
}

export function stringifyWorkflowNodesPayload(nodes: CanvasNode[], settings: WorkflowSettings): string {
  return JSON.stringify({ nodes: nodes || [], settings: normalizeWorkflowSettings(settings) });
}

interface BackendWorkflow {
  id: string;
  name: string;
  description: string;
  nodes: string;
  connections: string;
  status: string;
  created_at: string;
  updated_at: string;
}

function fromBackend(w: BackendWorkflow): WorkflowRecord {
  const payload = parseWorkflowNodesPayload(w.nodes);
  return {
    id: w.id,
    name: w.name,
    description: w.description,
    nodes: payload.nodes,
    connections: JSON.parse(w.connections || '[]'),
    settings: normalizeWorkflowSettings(payload.settings),
    status: w.status as 'draft' | 'ready',
    createdAt: w.created_at,
    updatedAt: w.updated_at,
    selectedHostIds: [],
  };
}

interface WorkflowState {
  workflows: WorkflowRecord[];
  loading: boolean;

  loadWorkflows: () => Promise<void>;
  createWorkflow: (name: string, description: string) => Promise<WorkflowRecord>;
  saveWorkflow: (id: string, name: string, description: string, nodes: CanvasNode[], connections: Connection[], status: string, settings?: WorkflowSettings) => Promise<void>;
  updateWorkflowHosts: (id: string, hostIds: string[]) => Promise<void>;
  deleteWorkflow: (id: string) => Promise<void>;
}

export const useWorkflowStore = create<WorkflowState>((set, get) => ({
  workflows: [],
  loading: false,

  loadWorkflows: async () => {
    set({ loading: true });
    try {
      const raw = await invoke<BackendWorkflow[]>('list_workflows');
      const workflows = raw.map(fromBackend);
      set({ workflows });
    } catch (e) {
      console.error('Failed to load workflows:', e);
      set({ workflows: [] });
    } finally {
      set({ loading: false });
    }
  },

  createWorkflow: async (name, description) => {
    const raw = await invoke<BackendWorkflow>('create_workflow', { name, description });
    const workflow = fromBackend(raw);
    await get().loadWorkflows();
    return workflow;
  },

  saveWorkflow: async (id, name, description, nodes, connections, status, settings = DEFAULT_WORKFLOW_SETTINGS) => {
    const backend = {
      id,
      name,
      description,
      nodes: stringifyWorkflowNodesPayload(nodes, settings),
      connections: JSON.stringify(connections || []),
      status,
      created_at: '',
      updated_at: '',
    };
    await invoke('update_workflow', { workflow: backend });
    await get().loadWorkflows();
  },

  updateWorkflowHosts: async (id, hostIds) => {
    const workflows = get().workflows.map((w) =>
      w.id === id ? { ...w, selectedHostIds: hostIds } : w,
    );
    set({ workflows });
    // Also persist to backend via a no-op save to trigger updatedAt update
    const wf = workflows.find((w) => w.id === id);
    if (wf) {
      await get().saveWorkflow(id, wf.name, wf.description, wf.nodes, wf.connections, wf.status);
    }
  },

  deleteWorkflow: async (id) => {
    await invoke('delete_workflow', { id });
    await get().loadWorkflows();
  },
}));
