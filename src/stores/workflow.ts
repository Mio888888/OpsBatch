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
  status: 'draft' | 'ready';
  createdAt: string;
  updatedAt: string;
  selectedHostIds: string[];
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
  return {
    id: w.id,
    name: w.name,
    description: w.description,
    nodes: JSON.parse(w.nodes || '[]'),
    connections: JSON.parse(w.connections || '[]'),
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
  saveWorkflow: (id: string, name: string, description: string, nodes: CanvasNode[], connections: Connection[], status: string) => Promise<void>;
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

  saveWorkflow: async (id, name, description, nodes, connections, status) => {
    const backend = {
      id,
      name,
      description,
      nodes: JSON.stringify(nodes || []),
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
