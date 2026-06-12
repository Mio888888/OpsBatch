import { useEffect, useRef, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useLogStore, type LogEntry } from '../../stores/log';
import { useTranslation } from '../../i18n';
import WindowControls from '../../components/WindowControls';

const levelColors: Record<string, string> = {
  error: 'var(--color-error, #ef4444)',
  warn: 'var(--color-warning, #f59e0b)',
  success: 'var(--color-success, #22c55e)',
  info: 'var(--color-primary, #3b82f6)',
};

const ORIGIN_KEYS: Record<string, 'log.backend' | 'log.frontend'> = {
  backend: 'log.backend',
  frontend: 'log.frontend',
};

const SOURCE_KEYS: Record<string, 'log.source.system' | 'log.source.ssh' | 'log.source.execution'> = {
  system: 'log.source.system',
  ssh: 'log.source.ssh',
  execution: 'log.source.execution',
};

const MAX_RENDERED_LOGS = 300;

function formatLogMessage(message: string, tText: (key: string, values?: Record<string, string | number>) => string): string {
  if (message === 'OpsBatch 后端服务已启动' || message === 'OpsBatch backend started') {
    return tText('log.message.backendStarted');
  }
  if (message === '全局日志窗口已连接' || message === 'Global log connected') {
    return tText('log.connectedMessage');
  }

  let match = message.match(/^主机 (.+) 正在连接\.\.\.$/);
  if (match) return tText('log.message.sshConnecting', { hostId: match[1] });

  match = message.match(/^主机 (.+) 已连接$/);
  if (match) return tText('log.message.sshConnected', { hostId: match[1] });

  match = message.match(/^主机 (.+) 连接空闲$/);
  if (match) return tText('log.message.sshIdle', { hostId: match[1] });

  match = message.match(/^主机 (.+) 连接断开$/);
  if (match) return tText('log.message.sshDisconnected', { hostId: match[1] });

  match = message.match(/^主机 (.+) 正在重连\.\.\.$/);
  if (match) return tText('log.message.sshReconnecting', { hostId: match[1] });

  match = message.match(/^主机 (.+) 正在连接 ([^:]+):(\d+)\.\.\.$/);
  if (match) return tText('log.message.sshConnectingHost', { hostId: match[1], host: match[2], port: match[3] });

  match = message.match(/^主机 (.+) \((.+)\) 已连接$/);
  if (match) return tText('log.message.sshConnectedHost', { hostId: match[1], host: match[2] });

  match = message.match(/^主机 (.+) 连接失败: (.+)$/);
  if (match) return tText('log.message.sshConnectFailed', { hostId: match[1], error: match[2] });

  match = message.match(/^开始执行命令 \[(.*)\]，目标 (\d+) 台主机$/);
  if (match) return tText('log.message.executionStart', { command: match[1], count: match[2] });

  match = message.match(/^命令执行完成 \[(.*)\]：全部成功（(\d+)台，耗时(\d+)ms）$/);
  if (match) return tText('log.message.executionAllSucceeded', { command: match[1], count: match[2], duration: match[3] });

  match = message.match(/^命令执行失败 \[(.*)\]：全部失败（(\d+)台，耗时(\d+)ms）$/);
  if (match) return tText('log.message.executionAllFailed', { command: match[1], count: match[2], duration: match[3] });

  match = message.match(/^命令执行完成 \[(.*)\]：成功(\d+)台，失败(\d+)台（耗时(\d+)ms）$/);
  if (match) return tText('log.message.executionPartial', { command: match[1], success: match[2], fail: match[3], duration: match[4] });

  return message;
}

export default function GlobalLogPage() {
  const { t, tText } = useTranslation();
  const logs = useLogStore((s) => s.logs);
  const loading = useLogStore((s) => s.loading);
  const clear = useLogStore((s) => s.clear);
  const init = useLogStore((s) => s.init);
  const bodyRef = useRef<HTMLDivElement>(null);
  const [autoScroll, setAutoScroll] = useState(true);
  const [filterLevel, setFilterLevel] = useState<string>('all');
  const [filterOrigin, setFilterOrigin] = useState<string>('all');

  useEffect(() => {
    init();
  }, [init]);

  useEffect(() => {
    if (autoScroll && bodyRef.current) {
      bodyRef.current.scrollTop = 0;
    }
  }, [logs, autoScroll]);

  const handleScroll = () => {
    if (!bodyRef.current) return;
    setAutoScroll(bodyRef.current.scrollTop < 40);
  };

  const filteredAll = logs.filter((log) => {
    if (filterLevel !== 'all' && log.level !== filterLevel) return false;
    if (filterOrigin !== 'all' && log.origin !== filterOrigin) return false;
    return true;
  });
  const filtered = filteredAll.slice(0, MAX_RENDERED_LOGS);

  const getOriginLabel = (origin: string) => {
    const key = ORIGIN_KEYS[origin];
    return key ? tText(key) : origin;
  };

  const getSourceLabel = (source: string) => {
    const key = SOURCE_KEYS[source];
    return key ? tText(key) : source;
  };

  return (
    <div className="global-log-root">
      <div
        className="global-log-toolbar"
        onMouseDown={(e) => {
          const target = e.target as HTMLElement;
          if (target.closest('button, select')) return;
          void getCurrentWindow().startDragging();
        }}
      >
        <div className="global-log-filters">
          <WindowControls className="global-log-window-controls" />
          <select
            className="global-log-filter-select"
            value={filterLevel}
            onChange={(e) => setFilterLevel(e.target.value)}
          >
            <option value="all">{tText('log.allLevels')}</option>
            <option value="error">{tText('log.error')}</option>
            <option value="warn">{tText('log.warn')}</option>
            <option value="info">{tText('log.info')}</option>
            <option value="success">{tText('log.success')}</option>
          </select>
          <select
            className="global-log-filter-select"
            value={filterOrigin}
            onChange={(e) => setFilterOrigin(e.target.value)}
          >
            <option value="all">{tText('log.allOrigins')}</option>
            <option value="backend">{tText('log.backend')}</option>
            <option value="frontend">{tText('log.frontend')}</option>
          </select>
          <span className="global-log-count">{t('log.count', { count: filteredAll.length })}</span>
        </div>
        <div className="global-log-actions">
          {!autoScroll && (
            <button
              type="button"
              className="global-log-action-btn"
              onClick={() => { setAutoScroll(true); }}
            >
              {t('log.follow')}
            </button>
          )}
          <button
            type="button"
            className="global-log-action-btn"
            onClick={() => {
              void clear();
            }}
          >
            {t('log.clear')}
          </button>
        </div>
      </div>
      <div className="workbench-log-body" ref={bodyRef} onScroll={handleScroll}>
        {loading && (
          <div className="global-log-empty">{t('log.loading')}</div>
        )}
        {!loading && filtered.length === 0 && (
          <div className="global-log-empty">{t('log.empty')}</div>
        )}
        {filtered.map((log, i) => (
          <LogLine
            key={`${log.timestamp}-${log.message}-${i}`}
            log={log}
            originLabel={getOriginLabel(log.origin)}
            sourceLabel={getSourceLabel(log.source)}
            message={formatLogMessage(log.message, tText as (key: string, values?: Record<string, string | number>) => string)}
          />
        ))}
      </div>
    </div>
  );
}

function LogLine({ log, originLabel, sourceLabel, message }: { log: LogEntry; originLabel: string; sourceLabel: string; message: string }) {
  const color = levelColors[log.level] ?? levelColors.info;

  return (
    <div className={`workbench-log-line workbench-log-${log.level}`}>
      <span className="workbench-log-time">{log.timestamp}</span>
      <span
        className="workbench-log-origin"
        style={{ color }}
      >
        [{originLabel}]
      </span>
      <span
        className="workbench-log-source"
        style={{ color }}
      >
        [{sourceLabel}]
      </span>
      <span>{message}</span>
    </div>
  );
}
