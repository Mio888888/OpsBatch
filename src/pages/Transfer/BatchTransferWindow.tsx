import { useEffect, useRef, useState, useMemo, useCallback } from 'react';
import { useSearchParams } from 'react-router-dom';
import { WebviewWindow } from '@tauri-apps/api/webviewWindow';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import type { Host } from '../../types';
import {
  Button, Input, InputNumber, message, Tooltip,
} from '../../components/ui';
import WindowControls from '../../components/WindowControls';
import { logHandledError } from '../../utils/globalLogger';
import {
  UploadOutlined,
  FolderOpenOutlined, FileOutlined,
  CheckCircleOutlined, CloseCircleOutlined,
  LoadingOutlined,
} from '../../components/ui/icons';

interface TransferProgress {
  hostId: string;
  hostName: string;
  success: boolean;
  fileSize: number;
  duration: number;
  error: string | null;
}

interface TransferDonePayload {
  direction: string;
  results?: TransferProgress[];
}

const TEMPLATE_QUICK_FILLS = [
  { label: '主机名', value: '/home/{host}/' },
  { label: '第一个目录', value: '/home/{firstdir:/home}/' },
  { label: '/tmp/', value: '/tmp/' },
  { label: '/opt/', value: '/opt/' },
  { label: '/home/', value: '/home/' },
  { label: '/var/log/', value: '/var/log/' },
  { label: '/usr/local/bin/', value: '/usr/local/bin/' },
  { label: '/etc/', value: '/etc/' },
];

const VARIABLE_HELP = [
  { var: '{host}', desc: '主机名称' },
  { var: '{firstdir:路径}', desc: '远程路径下第一个子目录' },
];

const MAX_COLLAPSED_HOSTS = 3;

function getFileName(path: string) {
  if (!path) return '';
  return path.split(/[\\/]/).filter(Boolean).pop() || path;
}

export default function BatchTransferWindow() {
  const [searchParams] = useSearchParams();
  const hostIdsParam = searchParams.get('hostIds') || '';
  const hostIds = useMemo(() => hostIdsParam.split(',').filter(Boolean), [hostIdsParam]);
  const hostIdSet = useMemo(() => new Set(hostIds), [hostIds]);

  const [hosts, setHosts] = useState<Host[]>([]);
  const [hostsLoading, setHostsLoading] = useState(true);
  const [localPath, setLocalPath] = useState('');
  const [remotePath, setRemotePath] = useState('');
  const [concurrency, setConcurrency] = useState(5);
  const [timeout, setTimeout_] = useState(120);
  const [transferring, setTransferring] = useState(false);
  const [results, setResults] = useState<TransferProgress[]>([]);
  const [activeTaskId, setActiveTaskId] = useState<string | null>(null);
  const [targetsExpanded, setTargetsExpanded] = useState(false);

  const listenersRef = useRef<UnlistenFn[]>([]);
  const resultIndexByHostIdRef = useRef<Map<string, number>>(new Map());

  const mergeResults = useCallback((incoming: TransferProgress[]) => {
    setResults((prev) => {
      const next = prev.slice();
      for (const data of incoming) {
        const index = resultIndexByHostIdRef.current.get(data.hostId);
        if (index === undefined) {
          resultIndexByHostIdRef.current.set(data.hostId, next.length);
          next.push(data);
        } else {
          next[index] = data;
        }
      }
      return next;
    });
  }, []);

  const cleanupListeners = useCallback(() => {
    listenersRef.current.forEach((fn) => fn());
    listenersRef.current = [];
  }, []);

  const setupTaskListeners = useCallback(async (taskId: string) => {
    cleanupListeners();

    const un1 = await listen(`transfer:${taskId}:progress`, (event) => {
      mergeResults([event.payload as TransferProgress]);
    });
    const un2 = await listen(`transfer:${taskId}:done`, (event) => {
      const payload = event.payload as TransferDonePayload;
      if (payload.results?.length) {
        mergeResults(payload.results);
        const failed = payload.results.filter((item) => !item.success).length;
        if (failed > 0) {
          message.error(`上传完成，失败 ${failed} 台`);
        } else {
          message.success('上传完成');
        }
      } else {
        message.success('上传完成');
      }
      setTransferring(false);
      setActiveTaskId(null);
      cleanupListeners();
    });

    listenersRef.current = [un1, un2];
  }, [cleanupListeners, mergeResults]);

  useEffect(() => () => cleanupListeners(), [cleanupListeners]);

  useEffect(() => {
    invoke<Host[]>('list_hosts').then((all) => {
      setHosts(all.filter((h) => hostIdSet.has(h.id)));
      setHostsLoading(false);
      WebviewWindow.getByLabel('batch-transfer').then((win) => {
        if (win) win.setTitle(`批量上传 - ${hostIds.length} 台主机`);
      });
    }).catch((error) => {
      void logHandledError('transfer.loadHosts', error, 'warn');
      setHostsLoading(false);
    });
  }, [hostIdSet, hostIds.length]);

  useEffect(() => {
    if (hosts.length <= MAX_COLLAPSED_HOSTS && targetsExpanded) {
      setTargetsExpanded(false);
    }
  }, [hosts.length, targetsExpanded]);

  const handleSelectFile = async () => {
    try {
      const selected = await open({ multiple: false, title: '选择要上传的文件' });
      if (selected) setLocalPath(selected);
    } catch { /* cancelled */ }
  };

  const handleSelectDirectory = async () => {
    try {
      const selected = await open({ multiple: false, directory: true, title: '选择要上传的目录' });
      if (selected) setLocalPath(selected);
    } catch { /* cancelled */ }
  };

  const handleUpload = async () => {
    if (hostIds.length === 0) {
      message.warning('未选择目标主机');
      return;
    }
    if (!localPath || !remotePath) {
      message.warning('请填写本地路径和远程路径');
      return;
    }

    try {
      setTransferring(true);
      resultIndexByHostIdRef.current.clear();
      setResults([]);
      const taskId = crypto.randomUUID();
      await setupTaskListeners(taskId);
      setActiveTaskId(taskId);
      await invoke<string>('file_transfer', {
        request: {
          task_id: taskId,
          direction: 'upload',
          host_ids: hostIds,
          local_path: localPath,
          remote_path: remotePath,
          timeout,
        },
      });
      message.info(`已开始上传到 ${hostIds.length} 台主机`);
    } catch (e: unknown) {
      cleanupListeners();
      setActiveTaskId(null);
      message.error(`上传失败: ${e}`);
      setTransferring(false);
    }
  };

  const { successCount, failCount } = useMemo(() => {
    let success = 0;
    let failed = 0;
    for (const result of results) {
      if (result.success) {
        success += 1;
      } else {
        failed += 1;
      }
    }
    return { successCount: success, failCount: failed };
  }, [results]);
  const doneCount = successCount + failCount;
  const pendingCount = Math.max(hostIds.length - doneCount, 0);
  const progress = hostIds.length > 0 ? (doneCount / hostIds.length) * 100 : 0;
  const isReady = hostIds.length > 0 && Boolean(localPath) && Boolean(remotePath);
  const selectedSourceName = getFileName(localPath);
  const visibleHosts = targetsExpanded ? hosts : hosts.slice(0, MAX_COLLAPSED_HOSTS);
  const hiddenHostCount = Math.max(hosts.length - MAX_COLLAPSED_HOSTS, 0);

  const resultsListRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (transferring && resultsListRef.current) {
      resultsListRef.current.scrollTop = resultsListRef.current.scrollHeight;
    }
  }, [results, transferring]);

  const formatSize = (bytes: number) => {
    if (bytes <= 0) return '-';
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  };

  if (hostsLoading) {
    return (
      <div className="batch-window batch-window-loading-state">
        <div className="batch-loading-card">
          <LoadingOutlined spin />
          <div>
            <strong>正在加载目标主机</strong>
            <span>读取批量上传上下文</span>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="batch-window">
      <header
        className="batch-titlebar"
        onMouseDown={(e) => {
          const target = e.target as HTMLElement;
          if (target.closest('button')) return;
          void import('@tauri-apps/api/window').then(({ getCurrentWindow }) => getCurrentWindow().startDragging());
        }}
        onDoubleClick={(e) => {
          const target = e.target as HTMLElement;
          if (target.closest('button')) return;
          void import('@tauri-apps/api/window')
            .then(({ getCurrentWindow }) => getCurrentWindow().toggleMaximize())
            .catch((error) => console.error('Window command failed:', error));
        }}
      >
        <WindowControls className="batch-window-controls" />
        <div className="batch-targets-strip-label">
          <span>目标主机</span>
          <strong>{hosts.length}</strong>
        </div>
        <div className="batch-targets-strip-list">
          {visibleHosts.map((h) => (
            <span key={h.id} className="batch-target-pill" title={`${h.name || h.ip} · ${h.ip}:${h.port}`}>
              {h.name || h.ip}
            </span>
          ))}
          {!targetsExpanded && hiddenHostCount > 0 && (
            <button
              className="batch-target-count-tag"
              type="button"
              onClick={() => setTargetsExpanded(true)}
              aria-label={`显示剩余 ${hiddenHostCount} 台主机`}
            >
              +{hiddenHostCount}
            </button>
          )}
          {targetsExpanded && hiddenHostCount > 0 && (
            <button
              className="batch-target-count-tag batch-target-count-tag-muted"
              type="button"
              onClick={() => setTargetsExpanded(false)}
            >
              收起
            </button>
          )}
          {hosts.length === 0 && <span className="batch-targets-empty">未选择主机</span>}
        </div>
      </header>

      <div className="batch-layout">
        <section className="batch-panel batch-config-panel" aria-label="上传设置">
          <header className="batch-panel-header">
            <div>
              <span className="batch-panel-eyebrow">Source & path</span>
              <h2>上传设置</h2>
            </div>
            {selectedSourceName && <span className="batch-source-name">{selectedSourceName}</span>}
          </header>

          <div className="transfer-paths">
            <div className="transfer-path-group">
              <label className="transfer-path-label" htmlFor="batch-local-path">本地路径</label>
              <div className="transfer-path-input-row">
                <Input
                  id="batch-local-path"
                  value={localPath}
                  onChange={(e) => setLocalPath(e.target.value)}
                  placeholder="/path/to/local/file"
                  className="transfer-path-input"
                  size="small"
                />
                <Tooltip title="选择文件">
                  <Button className="transfer-icon-button" size="small" icon={<FileOutlined />} onClick={handleSelectFile} />
                </Tooltip>
                <Tooltip title="选择目录">
                  <Button className="transfer-icon-button" size="small" icon={<FolderOpenOutlined />} onClick={handleSelectDirectory} />
                </Tooltip>
              </div>
            </div>

            <div className="transfer-path-group">
              <label className="transfer-path-label" htmlFor="batch-remote-path">远程路径</label>
              <Input
                id="batch-remote-path"
                value={remotePath}
                onChange={(e) => setRemotePath(e.target.value)}
                placeholder="/home/{host}/ 或 /home/{firstdir:/home}/web/"
                className="transfer-path-input"
                size="small"
              />
              <div className="transfer-path-suggestions" aria-label="远程路径快捷填充">
                {TEMPLATE_QUICK_FILLS.map((item) => (
                  <button
                    key={item.label}
                    className="transfer-path-suggestion"
                    type="button"
                    onClick={() => setRemotePath(item.value)}
                  >
                    {item.label}
                  </button>
                ))}
              </div>
              <div className="transfer-var-help">
                {VARIABLE_HELP.map((v) => (
                  <span key={v.var} className="transfer-var-item" title={v.desc}>
                    <code>{v.var}</code>
                    <span>{v.desc}</span>
                  </span>
                ))}
              </div>
            </div>
          </div>

          <div className="batch-controls">
            <label className="batch-param-card">
              <span className="batch-param-label">并发</span>
              <InputNumber min={1} max={20} value={concurrency} onChange={(v) => setConcurrency(v || 5)} />
            </label>
            <label className="batch-param-card batch-param-card-wide">
              <span className="batch-param-label">超时</span>
              <InputNumber min={1} max={3600} value={timeout} onChange={(v) => setTimeout_(v || 120)} addonAfter="秒" />
            </label>
            <div className="batch-actions">
              {activeTaskId ? (
                <Button className="batch-upload-button" type="primary" danger loading>
                  上传中
                </Button>
              ) : (
                <Button
                  className="batch-upload-button"
                  type="primary"
                  icon={<UploadOutlined />}
                  onClick={handleUpload}
                  loading={transferring}
                  disabled={!isReady}
                >
                  开始上传
                </Button>
              )}
            </div>
          </div>
        </section>

        <section className="batch-panel batch-results-panel" aria-label="上传结果">
          <div className="batch-results-header">
            <div>
              <span className="batch-panel-eyebrow">Results</span>
              <h2>上传结果</h2>
            </div>
            <div className="batch-results-tools">
              <span className="batch-result-counter">{doneCount}/{hostIds.length}</span>
              {results.length > 0 && (
                <Button size="small" onClick={() => { resultIndexByHostIdRef.current.clear(); setResults([]); setActiveTaskId(null); setTransferring(false); }}>
                  清除
                </Button>
              )}
            </div>
          </div>

          <div className="batch-progress-block" aria-label="上传进度">
            <div className="batch-progress-meta">
              <span>{Math.round(progress)}%</span>
              <span>{pendingCount > 0 ? `剩余 ${pendingCount}` : '无待处理主机'}</span>
            </div>
            <div className="batch-progress-bar">
              <div className="batch-progress-fill" style={{ width: `${progress}%` }} />
            </div>
          </div>

          <div className="batch-results-body">
            {results.length > 0 ? (
              <div className="batch-results-list" ref={resultsListRef}>
                {results.map((r, idx) => (
                  <div key={`${r.hostId}-${idx}`} className={`batch-result-row ${r.success ? 'batch-result-success' : 'batch-result-failed'}`}>
                    <div className="batch-result-summary">
                      <div className="batch-result-host">
                        <span className="batch-result-icon" aria-hidden="true">
                          {r.success ? <CheckCircleOutlined /> : <CloseCircleOutlined />}
                        </span>
                        <span className="batch-result-host-name">{r.hostName}</span>
                      </div>
                      <div className="batch-result-meta">
                        <span className={`batch-result-status ${r.success ? 'batch-result-status-success' : 'batch-result-status-failed'}`}>
                          {r.success ? '成功' : '失败'}
                        </span>
                        {r.duration > 0 && <span>{r.duration}ms</span>}
                        {r.fileSize > 0 && <span>{formatSize(r.fileSize)}</span>}
                      </div>
                    </div>
                    {r.error && (
                      <div className="batch-result-output">
                        {r.error}
                      </div>
                    )}
                  </div>
                ))}
              </div>
            ) : (
              <div className="batch-empty">
                <div className="batch-empty-mark">
                  <UploadOutlined />
                </div>
                <strong>暂无上传结果</strong>
                <span>{hostIds.length > 0 ? '等待上传任务' : '未选择目标主机'}</span>
              </div>
            )}
          </div>
        </section>
      </div>
    </div>
  );
}
