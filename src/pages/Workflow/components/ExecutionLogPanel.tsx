import { useEffect, useRef } from 'react';
import type { LogEntry } from '../workflowExecutionLogs';

export default function ExecutionLogPanel({ logs, collapsed, onToggle, labels }: {
  logs: LogEntry[];
  collapsed: boolean;
  onToggle: () => void;
  labels: { execLog: string; noLogs: string };
}) {
  const logBodyRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!collapsed && logBodyRef.current) {
      logBodyRef.current.scrollTop = logBodyRef.current.scrollHeight;
    }
  }, [logs, collapsed]);

  return (
    <div className={`wf-exec-log${collapsed ? ' wf-exec-log--collapsed' : ''}`}>
      <div className="wf-exec-log-header" onClick={onToggle}>
        <span className="wf-exec-log-title">
          {labels.execLog}
          {logs.length > 0 && <span className="wf-exec-log-count">{logs.length}</span>}
        </span>
        <span className="wf-exec-log-toggle">{collapsed ? '▲' : '▼'}</span>
      </div>
      {!collapsed && (
        <div className="wf-exec-log-body" ref={logBodyRef}>
          {logs.length === 0 ? (
            <div className="wf-exec-log-empty">{labels.noLogs}</div>
          ) : (
            logs.map((log, i) => (
              <div key={i} className={`wf-exec-log-entry${log.status ? ` wf-exec-log-entry--${log.status}` : ''}`}>
                <span className="wf-exec-log-time">
                  {new Date(log.timestamp).toLocaleTimeString(undefined, { hour12: false })}
                </span>
                {log.nodeName && <span className="wf-exec-log-node">[{log.nodeName}]</span>}
                <span className="wf-exec-log-msg">{log.message}</span>
              </div>
            ))
          )}
        </div>
      )}
    </div>
  );
}
