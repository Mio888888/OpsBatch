import type { CanvasNode } from '../../stores/workflow';
import type { ProgressEvent } from './workflowExecutor';

export interface LogEntry {
  timestamp: number;
  message: string;
  status?: 'running' | 'success' | 'fail' | 'info';
  nodeName?: string;
}

export interface WorkflowExecutionLogLabels {
  startExecute: string;
  executeSuccess: string;
  executeFail: string;
  stageInfo: (values: { level: number; total: number; count: number }) => string;
}

export function collectWorkflowHostIds(nodes: CanvasNode[]): string[] {
  const ids: string[] = [];
  for (const node of nodes) {
    if (node.type !== 'selectHost') continue;
    try {
      const cfg = JSON.parse(node.config || '{}') as { hostIds?: unknown };
      if (Array.isArray(cfg.hostIds)) {
        for (const hostId of cfg.hostIds) {
          if (typeof hostId === 'string' && hostId.trim()) {
            ids.push(hostId);
          }
        }
      }
    } catch {
      // Ignore invalid node config; execution validation will report no target hosts if none can be read.
    }
  }
  return [...new Set(ids)];
}

export function createWorkflowLogEntry(event: ProgressEvent, labels: WorkflowExecutionLogLabels): LogEntry | null {
  switch (event.type) {
    case 'log':
      return { timestamp: Date.now(), message: event.message, status: 'info' };

    case 'node_start':
      return {
        timestamp: Date.now(),
        message: labels.startExecute,
        nodeName: event.nodeName,
        status: 'running',
      };

    case 'node_complete': {
      const shortOutput = event.output.length > 200 ? `${event.output.slice(0, 200)}...` : event.output;
      return {
        timestamp: Date.now(),
        message: shortOutput || (event.success ? labels.executeSuccess : labels.executeFail),
        nodeName: event.nodeName,
        status: event.success ? 'success' : 'fail',
      };
    }

    case 'level_start':
      return {
        timestamp: Date.now(),
        message: labels.stageInfo({ level: event.level, total: event.total, count: event.count }),
        status: 'info',
      };

    case 'done':
      return null;
  }
}
