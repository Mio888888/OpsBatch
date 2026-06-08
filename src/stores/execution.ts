import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import type { ExecutionTask, HostExecutionResult, ExecutionHistory } from '../types';

interface BackendExecutionHistory {
  id: string;
  command: string;
  host_ids: string;
  host_count: number;
  success_count: number;
  fail_count: number;
  started_at: string;
  completed_at: string;
  duration: number;
}

interface ExecutionState {
  currentTask: ExecutionTask | null;
  history: ExecutionHistory[];
  runningTasks: ExecutionTask[];
  loading: boolean;

  executeCommand: (hostIds: string[], command: string, concurrency: number, timeout: number, quickActionId?: string) => Promise<string>;
  cancelExecution: (taskId: string) => Promise<void>;
  loadHistory: () => Promise<void>;
  getTaskOutput: (taskId: string) => Promise<HostExecutionResult[]>;
}

export const useExecutionStore = create<ExecutionState>((set) => ({
  currentTask: null,
  history: [],
  runningTasks: [],
  loading: false,

  executeCommand: async (hostIds, command, concurrency, timeout, quickActionId) => {
    set({ loading: true });
    try {
      const taskId = await invoke<string>('execute_command', {
        hostIds,
        command,
        concurrency,
        timeout,
        quickActionId: quickActionId || null,
      });

      const task: ExecutionTask = {
        id: taskId,
        hostIds,
        command,
        status: 'running',
        results: hostIds.map((hid) => ({
          hostId: hid,
          hostName: '',
          hostIp: '',
          status: 'pending',
          output: '',
        })),
        concurrency,
        timeout,
        startedAt: new Date().toISOString(),
      };
      set((s) => ({
        currentTask: task,
        runningTasks: [...s.runningTasks, task],
      }));
      return taskId;
    } finally {
      set({ loading: false });
    }
  },

  cancelExecution: async (taskId) => {
    await invoke('cancel_execution', { taskId });
    set((s) => ({
      runningTasks: s.runningTasks.map((t) =>
        t.id === taskId ? { ...t, status: 'cancelled' as const } : t
      ),
      currentTask: s.currentTask?.id === taskId
        ? { ...s.currentTask, status: 'cancelled' as const }
        : s.currentTask,
    }));
  },

  loadHistory: async () => {
    try {
      const raw = await invoke<BackendExecutionHistory[]>('list_execution_history');
      const history = raw.map((item) => ({
        id: item.id,
        command: item.command,
        hostIds: JSON.parse(item.host_ids || '[]') as string[],
        hostCount: item.host_count,
        successCount: item.success_count,
        failCount: item.fail_count,
        startedAt: item.started_at,
        completedAt: item.completed_at,
        duration: item.duration,
      }));
      set({ history });
    } catch {
      set({ history: [] });
    }
  },

  getTaskOutput: async (taskId) => {
    try {
      const results = await invoke<HostExecutionResult[]>('get_task_output', { taskId });
      return results;
    } catch {
      return [];
    }
  },
}));
