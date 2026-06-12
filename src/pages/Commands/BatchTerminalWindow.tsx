import { useEffect, useState, useCallback, useRef, useMemo } from 'react';
import { useSearchParams } from 'react-router-dom';
import { WebviewWindow } from '@tauri-apps/api/webviewWindow';
import { invoke } from '@tauri-apps/api/core';
import type { Host } from '../../types';
import WindowControls from '../../components/WindowControls';
import TerminalPane from './TerminalPane';
import { logHandledError } from '../../utils/globalLogger';
import './batch-terminal.css';

const MAX_CONCURRENT_CONNECTS = 4;

function createSemaphore(limit: number) {
  let current = 0;
  const queue: (() => void)[] = [];
  return {
    acquire(): Promise<void> {
      if (current < limit) {
        current++;
        return Promise.resolve();
      }
      return new Promise<void>((resolve) => queue.push(resolve));
    },
    release(): void {
      if (queue.length > 0) {
        queue.shift()!();
      } else {
        current--;
      }
    },
  };
}

interface PaneSession {
  hostId: string;
  hostName: string;
  hostIp: string;
  sessionId: string | null;
  state: 'connecting' | 'connected' | 'error';
  errorMessage?: string;
}

export default function BatchTerminalWindow() {
  const [searchParams] = useSearchParams();
  const hostIds = useMemo(
    () => searchParams.get('hostIds')?.split(',').filter(Boolean).slice(0, 16) || [],
    [searchParams],
  );
  const [hosts, setHosts] = useState<Host[]>([]);
  const [loading, setLoading] = useState(true);
  const [sessions, setSessions] = useState<PaneSession[]>([]);
  const [gridCols, setGridCols] = useState(2);
  const disposedRef = useRef(false);
  const sessionsRef = useRef<PaneSession[]>([]);
  const createdSessionIdsRef = useRef<Set<string>>(new Set());
  const semaphoreRef = useRef(createSemaphore(MAX_CONCURRENT_CONNECTS));

  useEffect(() => {
    sessionsRef.current = sessions;
  }, [sessions]);

  // load hosts
  useEffect(() => {
    invoke<Host[]>('list_hosts').then((all) => {
      const filtered = all.filter((h) => hostIds.includes(h.id));
      setHosts(filtered);
      setLoading(false);
      WebviewWindow.getByLabel('batch-terminal').then((win) => {
        if (win) win.setTitle(`广播终端 - ${filtered.length} 台主机`);
      });
    }).catch((error) => {
      void logHandledError('batchTerminal.loadHosts', error, 'warn');
      setLoading(false);
    });
  }, [hostIds]);

  // auto-connect all hosts
  useEffect(() => {
    if (hosts.length === 0) return;

    const pending: PaneSession[] = hosts.map((h) => ({
      hostId: h.id,
      hostName: h.name || h.ip,
      hostIp: h.ip,
      sessionId: null,
      state: 'connecting' as const,
    }));
    setSessions((prev) => {
      prev.forEach((s) => {
        if (s.sessionId) {
          void invoke('terminal_disconnect', { sessionId: s.sessionId }).catch((error) => {
            void logHandledError('batchTerminal.disconnectPrevious', error, 'warn');
          });
        }
      });
      return pending;
    });

    const controller = new AbortController();
    const sem = semaphoreRef.current;

    hosts.forEach((host) => {
      const sessionId = crypto.randomUUID();
      createdSessionIdsRef.current.add(sessionId);

      setSessions((prev) => prev.map((s) =>
        s.hostId === host.id ? { ...s, sessionId } : s,
      ));

      void sem.acquire().then(() => {
        if (disposedRef.current || controller.signal.aborted) {
          sem.release();
          invoke('terminal_disconnect', { sessionId }).catch((error) => {
            void logHandledError('batchTerminal.disconnectStale', error, 'warn');
          });
          return;
        }

        invoke<string>('terminal_connect', {
          hostId: host.id,
          cols: 80,
          rows: 24,
          sessionId,
        }).then(() => {
          sem.release();
          if (disposedRef.current || controller.signal.aborted) {
            invoke('terminal_disconnect', { sessionId }).catch((error) => {
              void logHandledError('batchTerminal.disconnectStale', error, 'warn');
            });
            return;
          }
          setSessions((prev) => prev.map((s) =>
            s.hostId === host.id ? { ...s, state: 'connected' as const } : s,
          ));
        }).catch((e) => {
          sem.release();
          if (disposedRef.current || controller.signal.aborted) {
            invoke('terminal_disconnect', { sessionId }).catch((error) => {
              void logHandledError('batchTerminal.disconnectFailedSession', error, 'warn');
            });
            return;
          }
          setSessions((prev) => prev.map((s) =>
            s.hostId === host.id
              ? { ...s, state: 'error' as const, errorMessage: String(e) }
              : s,
          ));
        });
      });
    });

    return () => controller.abort();
  }, [hosts]);

  // disconnect all on unmount
  useEffect(() => {
    disposedRef.current = false;
    return () => {
      disposedRef.current = true;
      const sessionIds = new Set(createdSessionIdsRef.current);
      sessionsRef.current.forEach((s) => {
        if (s.sessionId) {
          sessionIds.add(s.sessionId);
        }
      });
      createdSessionIdsRef.current.clear();
      sessionIds.forEach((sessionId) => {
        invoke('terminal_disconnect', { sessionId }).catch((error) => {
          void logHandledError('batchTerminal.disconnectUnmount', error, 'warn');
        });
      });
    };
  }, []);

  const retryHost = useCallback((hostId: string) => {
    const host = hosts.find((h) => h.id === hostId);
    if (!host) return;

    const sessionId = crypto.randomUUID();
    createdSessionIdsRef.current.add(sessionId);
    setSessions((prev) => prev.map((s) => {
      if (s.hostId !== hostId) return s;
      if (s.sessionId) {
        void invoke('terminal_disconnect', { sessionId: s.sessionId }).catch((error) => {
          void logHandledError('batchTerminal.disconnectRetryPrevious', error, 'warn');
        });
      }
      return { ...s, sessionId, state: 'connecting', errorMessage: undefined };
    }));

    const sem = semaphoreRef.current;
    void sem.acquire().then(() => {
      if (disposedRef.current) {
        sem.release();
        invoke('terminal_disconnect', { sessionId }).catch((error) => {
          void logHandledError('batchTerminal.disconnectDisposedRetry', error, 'warn');
        });
        return;
      }
      invoke<string>('terminal_connect', {
        hostId: host.id,
        cols: 80,
        rows: 24,
        sessionId,
      }).then(() => {
        sem.release();
        if (disposedRef.current) {
          invoke('terminal_disconnect', { sessionId }).catch((error) => {
            void logHandledError('batchTerminal.disconnectDisposedRetry', error, 'warn');
          });
          return;
        }
        setSessions((prev) => prev.map((s) =>
          s.hostId === hostId ? { ...s, state: 'connected' } : s,
        ));
      }).catch((e) => {
        sem.release();
        if (disposedRef.current) {
          invoke('terminal_disconnect', { sessionId }).catch((error) => {
            void logHandledError('batchTerminal.disconnectFailedRetry', error, 'warn');
          });
          return;
        }
        setSessions((prev) => prev.map((s) =>
          s.hostId === hostId ? { ...s, state: 'error', errorMessage: String(e) } : s,
        ));
      });
    });
  }, [hosts]);

  const handleBroadcast = useCallback((data: string) => {
    const writes: [string, string][] = [];
    for (const s of sessions) {
      if (s.sessionId && s.state === 'connected') {
        writes.push([s.sessionId, data]);
      }
    }
    if (writes.length > 0) {
      invoke('terminal_batch_write', { writes }).catch((error) => {
        void logHandledError('batchTerminal.write', error, 'warn');
      });
    }
  }, [sessions]);

  const handleClose = useCallback(async () => {
    const win = await WebviewWindow.getByLabel('batch-terminal');
    win?.close();
  }, []);

  const connectedCount = useMemo(() => sessions.filter((s) => s.state === 'connected').length, [sessions]);
  const errorCount = useMemo(() => sessions.filter((s) => s.state === 'error').length, [sessions]);

  // grid cols: auto-calculate based on pane count
  const autoCols = useMemo(() => Math.ceil(Math.sqrt(sessions.length)), [sessions.length]);
  const effectiveCols = gridCols === 0 ? autoCols : gridCols;

  if (loading) {
    return <div className="bt-loading">加载主机信息...</div>;
  }

  return (
    <div className="bt-root">
      <header
        className="bt-header"
        onMouseDown={(e) => {
          const target = e.target as HTMLElement;
          if (target.closest('button, select')) return;
          void WebviewWindow.getCurrent().startDragging();
        }}
      >
        <div className="bt-header-left">
          <WindowControls className="bt-window-controls" />
          <span className="bt-header-title">广播终端</span>
          <span className="bt-header-count">{sessions.length} 台</span>
          {connectedCount > 0 && (
            <span className="bt-header-stat bt-header-stat-ok">{connectedCount} 已连接</span>
          )}
          {errorCount > 0 && (
            <span className="bt-header-stat bt-header-stat-err">{errorCount} 失败</span>
          )}
          {sessions.length - connectedCount - errorCount > 0 && (
            <span className="bt-header-stat bt-header-stat-pending">
              {sessions.length - connectedCount - errorCount} 连接中
            </span>
          )}
        </div>
        <div className="bt-header-right">
          <select
            className="bt-layout-select"
            value={gridCols}
            onChange={(e) => setGridCols(Number(e.target.value))}
          >
            <option value={0}>自动</option>
            <option value={1}>1列</option>
            <option value={2}>2列</option>
            <option value={3}>3列</option>
            <option value={4}>4列</option>
          </select>
          <button className="bt-close-btn" onClick={handleClose}>关闭</button>
        </div>
      </header>

      <div
        className="bt-grid"
        style={{ gridTemplateColumns: `repeat(${effectiveCols}, 1fr)` }}
      >
        {sessions.map((s) => (
          <TerminalPane
            key={s.hostId}
            sessionId={s.sessionId}
            hostName={s.hostName}
            hostIp={s.hostIp}
            state={s.state}
            errorMessage={s.errorMessage}
            onRetry={() => retryHost(s.hostId)}
            onBroadcast={handleBroadcast}
          />
        ))}
      </div>
    </div>
  );
}
