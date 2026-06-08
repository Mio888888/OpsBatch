import { useEffect, useRef, useState, useMemo, useCallback } from 'react';
import { useSearchParams } from 'react-router-dom';
import { WebviewWindow } from '@tauri-apps/api/webviewWindow';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import type { Host } from '../../types';
import {
  Button, Space, Input, InputNumber, message, Empty, Tooltip,
} from '../../components/ui';
import {
  UploadOutlined,
  FolderOpenOutlined, FileOutlined,
  CheckCircleOutlined, CloseCircleOutlined,
  LoadingOutlined, CloudServerOutlined,
} from '../../components/ui/icons';

interface TransferProgress {
  hostId: string;
  hostName: string;
  success: boolean;
  fileSize: number;
  duration: number;
  error: string | null;
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

export default function BatchTransferWindow() {
  const [searchParams] = useSearchParams();
  const hostIds = searchParams.get('hostIds')?.split(',').filter(Boolean) || [];

  const [hosts, setHosts] = useState<Host[]>([]);
  const [hostsLoading, setHostsLoading] = useState(true);
  const [localPath, setLocalPath] = useState('');
  const [remotePath, setRemotePath] = useState('');
  const [concurrency, setConcurrency] = useState(5);
  const [timeout, setTimeout_] = useState(120);
  const [transferring, setTransferring] = useState(false);
  const [results, setResults] = useState<TransferProgress[]>([]);
  const [activeTaskId, setActiveTaskId] = useState<string | null>(null);

  const listenersRef = useRef<UnlistenFn[]>([]);

  const cleanupListeners = useCallback(() => {
    listenersRef.current.forEach((fn) => fn());
    listenersRef.current = [];
  }, []);

  useEffect(() => () => cleanupListeners(), [cleanupListeners]);

  useEffect(() => {
    invoke<Host[]>('list_hosts').then((all) => {
      setHosts(all.filter((h) => hostIds.includes(h.id)));
      setHostsLoading(false);
      WebviewWindow.getByLabel('batch-transfer').then((win) => {
        if (win) win.setTitle(`批量上传 - ${hostIds.length} 台主机`);
      });
    }).catch(() => setHostsLoading(false));
  }, []);

  useEffect(() => {
    let unlisteners: UnlistenFn[] = [];
    if (!activeTaskId) return;

    const setup = async () => {
      const un1 = await listen(`transfer:${activeTaskId}:progress`, (event) => {
        const data = event.payload as TransferProgress;
        setResults((prev) => [...prev, data]);
      });
      const un2 = await listen(`transfer:${activeTaskId}:done`, () => {
        setTransferring(false);
        setActiveTaskId(null);
        message.success('上传完成');
      });
      unlisteners = [un1, un2];
    };
    setup();
    return () => { unlisteners.forEach((fn) => fn()); };
  }, [activeTaskId]);

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
      setResults([]);
      const taskId = await invoke<string>('file_transfer', {
        request: {
          direction: 'upload',
          hostIds,
          localPath,
          remotePath,
          timeout,
        },
      });
      setActiveTaskId(taskId);
      message.info(`已开始上传到 ${hostIds.length} 台主机`);
    } catch (e: unknown) {
      message.error(`上传失败: ${e}`);
      setTransferring(false);
    }
  };

  const successCount = useMemo(() => results.filter((r) => r.success).length, [results]);
  const failCount = useMemo(() => results.filter((r) => !r.success).length, [results]);
  const doneCount = successCount + failCount;
  const progress = results.length > 0 ? (doneCount / hostIds.length) * 100 : 0;

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
      <div className="batch-window">
        <div className="batch-loading">正在加载...</div>
      </div>
    );
  }

  return (
    <div className="batch-window">
      {/* Target hosts */}
      <div className="batch-targets-bar">
        <div className="batch-targets-label">
          <CloudServerOutlined />
          <span>目标主机</span>
          <span className="batch-targets-count">{hostIds.length}</span>
        </div>
        <div className="batch-targets-list">
          {hosts.map((h) => (
            <span key={h.id} className="batch-target-pill">
              {h.name || h.ip}
            </span>
          ))}
          {hosts.length === 0 && <span className="batch-targets-empty">未选择主机</span>}
        </div>
      </div>

      {/* Upload config */}
      <div className="batch-command-card">
        <div className="transfer-direction-row">
          <UploadOutlined />
          <span style={{ fontSize: 13, fontWeight: 600, color: '#263044' }}>批量上传</span>
          <span className="transfer-direction-hint">本地 → 远程</span>
        </div>

        <div className="transfer-paths">
          <div className="transfer-path-group">
            <label className="transfer-path-label">本地路径</label>
            <div className="transfer-path-input-row">
              <Input
                value={localPath}
                onChange={(e) => setLocalPath(e.target.value)}
                placeholder="/path/to/local/file"
                style={{ fontFamily: 'monospace', flex: 1 }}
                size="small"
              />
              <Tooltip title="选择文件">
                <Button size="small" icon={<FileOutlined />} onClick={handleSelectFile} />
              </Tooltip>
              <Tooltip title="选择目录">
                <Button size="small" icon={<FolderOpenOutlined />} onClick={handleSelectDirectory} />
              </Tooltip>
            </div>
          </div>

          <div className="transfer-path-group">
            <label className="transfer-path-label">远程路径</label>
            <Input
              value={remotePath}
              onChange={(e) => setRemotePath(e.target.value)}
              placeholder="/home/{host}/ 或 /home/{firstdir:/home}/web/"
              style={{ fontFamily: 'monospace' }}
              size="small"
            />
            <div className="transfer-path-suggestions">
              {TEMPLATE_QUICK_FILLS.map((item) => (
                <span key={item.label} className="transfer-path-suggestion" onClick={() => setRemotePath(item.value)}>
                  {item.label}
                </span>
              ))}
            </div>
            <div className="transfer-var-help">
              {VARIABLE_HELP.map((v) => (
                <span key={v.var} className="transfer-var-item" title={v.desc}>
                  <code>{v.var}</code> {v.desc}
                </span>
              ))}
            </div>
          </div>
        </div>

        <div className="batch-controls">
          <div className="batch-params">
            <Space>
              <span className="batch-param-label">并发</span>
              <InputNumber min={1} max={20} value={concurrency} onChange={(v) => setConcurrency(v || 5)} style={{ width: 64 }} />
            </Space>
            <Space>
              <span className="batch-param-label">超时</span>
              <InputNumber min={1} max={3600} value={timeout} onChange={(v) => setTimeout_(v || 120)} style={{ width: 80 }} addonAfter="秒" />
            </Space>
          </div>
          <div className="batch-actions">
            {activeTaskId ? (
              <Button type="primary" danger loading>上传中...</Button>
            ) : (
              <Button
                type="primary"
                icon={<UploadOutlined />}
                onClick={handleUpload}
                loading={transferring}
                disabled={hostIds.length === 0}
              >
                开始上传
              </Button>
            )}
          </div>
        </div>
      </div>

      {/* Results */}
      <div className="batch-results-card">
        <div className="batch-results-header">
          <h3>上传结果</h3>
          {results.length > 0 && (
            <div className="batch-stats">
              <span className="batch-stat">
                <span className="batch-stat-value">{doneCount}/{hostIds.length}</span>
              </span>
              {successCount > 0 && (
                <span className="batch-stat batch-stat-success">
                  <CheckCircleOutlined /> {successCount}
                </span>
              )}
              {failCount > 0 && (
                <span className="batch-stat batch-stat-fail">
                  <CloseCircleOutlined /> {failCount}
                </span>
              )}
              {(hostIds.length - doneCount) > 0 && (
                <span className="batch-stat batch-stat-running">
                  <LoadingOutlined /> {hostIds.length - doneCount}
                </span>
              )}
            </div>
          )}
          {results.length > 0 && (
            <div className="batch-results-actions">
              <Button size="small" onClick={() => { setResults([]); setActiveTaskId(null); setTransferring(false); }}>
                清除
              </Button>
            </div>
          )}
        </div>

        {results.length > 0 && (
          <div className="batch-progress-bar">
            <div className="batch-progress-fill" style={{ width: `${progress}%` }} />
          </div>
        )}

        <div className="batch-results-body">
          {results.length > 0 ? (
            <div className="batch-results-list" ref={resultsListRef}>
              {results.map((r, idx) => (
                <div key={`${r.hostId}-${idx}`} className={`batch-result-row ${r.success ? 'batch-result-success' : 'batch-result-failed'}`}>
                  <div className="batch-result-summary">
                    <div className="batch-result-host">
                      {r.success ? (
                        <CheckCircleOutlined style={{ color: '#52c41a', fontSize: 14 }} />
                      ) : (
                        <CloseCircleOutlined style={{ color: '#ff4d4f', fontSize: 14 }} />
                      )}
                      <span className="batch-result-host-name">{r.hostName}</span>
                    </div>
                    <div className="batch-result-meta">
                      <span className={`batch-result-status ${r.success ? 'batch-result-status-success' : 'batch-result-status-failed'}`}>
                        {r.success ? '成功' : '失败'}
                      </span>
                      {r.duration > 0 && <span className="batch-result-duration">{r.duration}ms</span>}
                      {r.fileSize > 0 && <span className="batch-result-duration">{formatSize(r.fileSize)}</span>}
                    </div>
                  </div>
                  {r.error && (
                    <div className="batch-result-output batch-result-output-muted">
                      {r.error}
                    </div>
                  )}
                </div>
              ))}
            </div>
          ) : (
            <div className="batch-empty">
              <Empty description="配置路径后开始上传" />
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
