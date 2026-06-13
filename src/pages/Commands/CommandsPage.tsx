import { useEffect, useRef, useState, useMemo, useCallback } from 'react';
import {
  Button, Space, Input, Card, Tabs, Table, Tag, Badge,
  InputNumber, message, Modal, Empty, Row, Col, Statistic,
  Tooltip,
} from '../../components/ui';
import type { BadgeProps } from '../../components/ui';
import {
  PlayCircleOutlined, StopOutlined, ClearOutlined,
  WarningOutlined, CheckCircleOutlined, CloseCircleOutlined,
  LoadingOutlined, HistoryOutlined, SafetyOutlined,
  UploadOutlined,
} from '../../components/ui/icons';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { compileDangerRules, checkDangerousCommand as checkDanger, DEFAULT_COMPILED_DANGER_RULES, type CompiledDangerRule } from '../../utils/dangerCommandCheck';
import type { ColumnsType } from '../../components/ui';
import { useLocation } from 'react-router-dom';
import { useAssetsStore } from '../../stores/assets';
import AsciinemaPlayer from '../../components/AsciinemaPlayer';
import { useExecutionStore } from '../../stores/execution';
import { useLibraryStore } from '../../stores/library';
import { renderAnsiOutput, RISK_COLORS } from '../../utils/ansi.tsx';
import { useTranslation } from '../../i18n';
import type { HostExecutionResult } from '../../types';

interface ExecutionHistoryRow {
  id: string;
  command: string;
  hostCount: number;
  successCount: number;
  failCount: number;
  startedAt: string;
  duration: number;
}

interface ExecutionDetailRow {
  host_id: string;
  host_name: string;
  status: string;
  output: string;
  exit_code: number;
  duration: number;
}

export default function CommandsPage() {
  const { t, tText } = useTranslation();
  const hosts = useAssetsStore((s) => s.hosts);
  const selectedHostIds = useAssetsStore((s) => s.selectedHostIds);
  const loadHosts = useAssetsStore((s) => s.loadHosts);
  const cancelExecution = useExecutionStore((s) => s.cancelExecution);
  const loadHistory = useExecutionStore((s) => s.loadHistory);
  const history = useExecutionStore((s) => s.history);
  const commands = useLibraryStore((s) => s.commands);
  const loadCommands = useLibraryStore((s) => s.loadCommands);
  const location = useLocation();
  const [replayModalOpen, setReplayModalOpen] = useState(false);
  const [replayDetails, setReplayDetails] = useState<ExecutionDetailRow[]>([]);
  const [asciinemaOpen, setAsciinemaOpen] = useState(false);
  const [asciinemaHistoryId, setAsciinemaHistoryId] = useState('');

  const [command, setCommand] = useState('');
  const [concurrency, setConcurrency] = useState(10);
  const [timeout, setTimeout_] = useState(30);
  const [results, setResults] = useState<HostExecutionResult[]>([]);
  const [activeTab, setActiveTab] = useState('execute');
  const [dangerModalVisible, setDangerModalVisible] = useState(false);
  const [dangerInfo, setDangerInfo] = useState('');
  const [dangerMatchedRules, setDangerMatchedRules] = useState<string[]>([]);
  const [activeTaskId, setActiveTaskId] = useState<string | null>(null);
  const [executing, setExecuting] = useState(false);
  const [cmdLibModalOpen, setCmdLibModalOpen] = useState(false);

  // AI risk assessment state
  const [riskAssessment, setRiskAssessment] = useState<{ level: string; reason: string } | null>(null);
  const [riskLoading, setRiskLoading] = useState(false);

  const selectedHosts = hosts.filter((h) => selectedHostIds.includes(h.id));

  const openBatchTransfer = useCallback(async () => {
    if (selectedHostIds.length === 0) return;
    await invoke('open_managed_window', { kind: 'batch-transfer', hostIds: selectedHostIds });
  }, [selectedHostIds]);

  const listenersRef = useRef<UnlistenFn[]>([]);

  const cleanupListeners = useCallback(() => {
    listenersRef.current.forEach((fn) => fn());
    listenersRef.current = [];
  }, []);

  useEffect(() => () => cleanupListeners(), [cleanupListeners]);

  const setupListeners = useCallback(async (taskId: string) => {
    cleanupListeners();

    const un1 = await listen(`exec:${taskId}:start`, (event) => {
      const hostId = event.payload as string;
      setResults((prev) => {
        const exists = prev.find((r) => r.hostId === hostId);
        if (exists) {
          return prev.map((r) => r.hostId === hostId ? { ...r, status: 'running' as const, startedAt: new Date().toISOString() } : r);
        }
        return [...prev, {
          hostId,
          hostName: '',
          hostIp: '',
          status: 'running' as const,
          output: '',
          startedAt: new Date().toISOString(),
        }];
      });
    });

    const un2 = await listen(`exec:${taskId}:output`, (event) => {
      const data = event.payload as {
        hostId: string;
        hostName: string;
        status: string;
        output: string;
        exitCode: number;
        duration: number;
      };
      setResults((prev) => {
        const exists = prev.find((r) => r.hostId === data.hostId);
        if (exists) {
          return prev.map((r) => r.hostId === data.hostId ? {
            ...r,
            hostName: data.hostName || r.hostName,
            status: data.status as HostExecutionResult['status'],
            output: data.output,
            exitCode: data.exitCode,
            duration: data.duration,
            completedAt: new Date().toISOString(),
          } : r);
        }
        return [...prev, {
          hostId: data.hostId,
          hostName: data.hostName,
          hostIp: '',
          status: data.status as HostExecutionResult['status'],
          output: data.output,
          exitCode: data.exitCode,
          duration: data.duration,
          completedAt: new Date().toISOString(),
        }];
      });
    });

    const un3 = await listen(`exec:${taskId}:done`, (event) => {
      const data = event.payload as { successCount: number; failCount: number; duration: number };
      message.success(tText('commands.completedToast', { success: data.successCount, failed: data.failCount, duration: data.duration }));
      setActiveTaskId(null);
      setExecuting(false);
    });

    listenersRef.current = [un1, un2, un3];
  }, [cleanupListeners, tText]);

  useEffect(() => {
    loadHosts();
    loadCommands();
    loadHistory();
    // Read command passed from other pages (CommandLib, ScriptLib)
    const state = location.state as { command?: string } | null;
    if (state?.command) {
      setCommand(state.command);
      setActiveTab('execute');
    }
  }, []);

  const [compiledDangerRules, setCompiledDangerRules] = useState<CompiledDangerRule[]>(DEFAULT_COMPILED_DANGER_RULES);

  const loadDangerRules = async () => {
    try {
      const rules = await invoke<{ id: string; name: string; pattern: string; enabled: boolean; is_builtin: boolean }[]>('list_danger_rules');
      setCompiledDangerRules(compileDangerRules(rules));
    } catch {
      // 保留默认内置规则，不清空
    }
  };

  useEffect(() => {
    loadDangerRules();
  }, []);

  const checkDangerousCommand = (cmd: string): string[] => {
    return checkDanger(compiledDangerRules, cmd);
  };

  const doExecute = async () => {
    try {
      const taskId = crypto.randomUUID();
      // Initialize results for all selected hosts
      const initialResults: HostExecutionResult[] = selectedHostIds.map((hid) => {
        const host = hosts.find((h) => h.id === hid);
        return {
          hostId: hid,
          hostName: host?.name || '',
          hostIp: host?.ip || '',
          status: 'pending' as const,
          output: '',
        };
      });
      setResults(initialResults);
      setExecuting(true);

      await setupListeners(taskId);

      await invoke('execute_command', { taskId, hostIds: selectedHostIds, command, concurrency, timeout });
      setActiveTaskId(taskId);
      message.info(tText('commands.dispatchedToast', { count: selectedHostIds.length }));
    } catch (e) {
      cleanupListeners();
      message.error(tText('commands.executeFailed', { error: String(e) }));
      setExecuting(false);
    }
  };

  const handleExecute = async () => {
    if (!command.trim()) {
      message.warning(tText('commands.enterCommand'));
      return;
    }
    if (selectedHostIds.length === 0) {
      message.warning(tText('commands.selectHostsFirst'));
      return;
    }

    const matched = checkDangerousCommand(command);
    if (matched.length > 0) {
      setDangerInfo(command);
      setDangerMatchedRules(matched);
      setDangerModalVisible(true);
      return;
    }

    doExecute();
  };

  const handleStop = async () => {
    if (activeTaskId) {
      await cancelExecution(activeTaskId);
      cleanupListeners();
      setActiveTaskId(null);
      setExecuting(false);
      setResults((prev) => prev.map((r) =>
        r.status === 'pending' || r.status === 'running' ? { ...r, status: 'failed' as const, output: `${r.output}\n${tText('commands.cancelled')}` } : r
      ));
    }
  };

  const handleRiskAssessment = async () => {
    if (!command.trim()) {
      message.warning(tText('commands.enterCommandFirst'));
      return;
    }
    setRiskLoading(true);
    setRiskAssessment(null);
    try {
      const result = await invoke<{ level: string; reason: string }>('ai_risk_assessment', { command });
      setRiskAssessment(result);
    } catch (e) {
      message.error(tText('commands.riskAssessmentFailed', { error: String(e) }));
    } finally {
      setRiskLoading(false);
    }
  };

  // Filter results
  const successResults = useMemo(() => results.filter((r) => r.status === 'success'), [results]);
  const failResults = useMemo(() => results.filter((r) => r.status === 'failed' || r.status === 'timeout'), [results]);
  const runningResults = useMemo(() => results.filter((r) => r.status === 'running' || r.status === 'pending'), [results]);

  const reExecuteFailed = async () => {
    const failedIds = failResults.map((r) => r.hostId);
    if (failedIds.length === 0) return;
    try {
      const taskId = crypto.randomUUID();
      await setupListeners(taskId);
      await invoke('execute_command', { taskId, hostIds: failedIds, command, concurrency, timeout });
      setActiveTaskId(taskId);
      setExecuting(true);
      setResults((prev) => prev.map((r) =>
        failedIds.includes(r.hostId) ? { ...r, status: 'pending' as const, output: '' } : r
      ));
      message.info(tText('commands.reExecuteFailedHosts', { count: failedIds.length }));
    } catch (e) {
      cleanupListeners();
      message.error(tText('commands.reExecuteFailed', { error: String(e) }));
      setExecuting(false);
    }
  };

  const resultColumns: ColumnsType<HostExecutionResult> = [
    {
      title: t('common.host'),
      width: 180,
      render: (_, r) => (
        <span>{r.hostName || r.hostIp || r.hostId}</span>
      ),
      sorter: (a, b) => (a.hostName || a.hostIp).localeCompare(b.hostName || b.hostIp),
    },
    {
      title: t('common.status'),
      dataIndex: 'status',
      width: 100,
      render: (s: string) => {
        const map: Record<string, { color: BadgeProps['status']; text: string }> = {
          pending: { color: 'default', text: tText('commands.pending') },
          running: { color: 'processing', text: tText('commands.running') },
          success: { color: 'success', text: tText('commands.success') },
          failed: { color: 'error', text: tText('commands.failed') },
          timeout: { color: 'warning', text: tText('commands.timeoutStatus') },
        };
        const info = map[s] || map.pending;
        return <Badge status={info.color} text={info.text} />;
      },
      filters: [
        { text: tText('commands.success'), value: 'success' },
        { text: tText('commands.failed'), value: 'failed' },
        { text: tText('commands.running'), value: 'running' },
        { text: tText('commands.pending'), value: 'pending' },
      ],
      onFilter: (value, record) => record.status === value,
    },
    {
      title: t('common.outputPreview'),
      dataIndex: 'output',
      ellipsis: true,
      render: (text: string) => (
        <span style={{ fontFamily: 'monospace', fontSize: 12, color: text ? undefined : '#999' }}>
          {text ? (renderAnsiOutput(text.length > 100 ? text.substring(0, 100) + '...' : text)) : t('commands.waitingOutput')}
        </span>
      ),
    },
    {
      title: t('common.duration'),
      dataIndex: 'duration',
      width: 80,
      render: (d?: number) => d != null ? `${d}ms` : '-',
    },
  ];

  const historyColumns: ColumnsType<ExecutionHistoryRow> = [
    { title: t('commands.historyCommand'), dataIndex: 'command', width: 250, ellipsis: true },
    { title: t('commands.hostCount'), dataIndex: 'hostCount', width: 80 },
    {
      title: t('commands.result'), width: 120,
      render: (_, r) => (
        <Space>
          <Tag color="green">{r.successCount} {t('commands.success')}</Tag>
          {r.failCount > 0 && <Tag color="red">{r.failCount} {t('commands.failed')}</Tag>}
        </Space>
      ),
    },
    { title: t('commands.startTime'), dataIndex: 'startedAt', width: 170 },
    { title: t('common.duration'), dataIndex: 'duration', width: 80, render: (d: number) => `${d}ms` },
    {
      title: t('common.action'), width: 200,
      render: (_, r) => (
        <Space>
          <Button size="small" onClick={() => { setCommand(r.command as string); message.info(tText('commands.commandFilled')); }}>
            {t('commands.reuse')}
          </Button>
          <Button size="small" icon={<HistoryOutlined />} onClick={async () => {
            try {
              const details = await invoke<ExecutionDetailRow[]>('get_execution_detail', { historyId: r.id });
              setReplayDetails(details);
              setReplayModalOpen(true);
            } catch (e) {
              message.error(`${e}`);
            }
          }}>
            {t('commands.tableReplay')}
          </Button>
          <Button size="small" type="link" onClick={() => {
            setAsciinemaHistoryId(r.id as string);
            setAsciinemaOpen(true);
          }}>
            {t('commands.recordingReplay')}
          </Button>
        </Space>
      ),
    },
  ];

  return (
    <div className="page-container">
      <div className="page-header">
        <h2>{t('commands.title')}</h2>
        <Space>
          <Tag color="blue">{t('commands.selectedHosts', { count: selectedHostIds.length })}</Tag>
          <Button size="small" icon={<UploadOutlined />} disabled={selectedHostIds.length === 0} onClick={openBatchTransfer}>
            {t('commands.batchUpload')}
          </Button>
          <Button size="small" onClick={() => setCmdLibModalOpen(true)}>
            {t('commands.selectFromLibrary')}
          </Button>
        </Space>
      </div>

      <Tabs activeKey={activeTab} onChange={setActiveTab} items={[
        {
          key: 'execute',
          label: t('commands.executeTab'),
          children: (
            <div>
              <Card size="small" style={{ marginBottom: 12 }}>
                <Space direction="vertical" style={{ width: '100%' }} size={12}>
                  <div style={{ display: 'flex', gap: 8 }}>
                    <div style={{ flex: 1 }}>
                      <Input.TextArea
                        value={command}
                        onChange={(e) => { setCommand(e.target.value); setRiskAssessment(null); }}
                        placeholder={tText('commands.inputPlaceholder')}
                        rows={3}
                        style={{ fontFamily: 'monospace' }}
                      />
                      {riskAssessment && (
                        <div style={{ marginTop: 6, display: 'flex', alignItems: 'center', gap: 8 }}>
                          <Tag color={RISK_COLORS[riskAssessment.level] || 'default'}>
                            {t('commands.riskLevel', { level: riskAssessment.level })}
                          </Tag>
                          {riskAssessment.reason && (
                            <span style={{ fontSize: 12, color: '#666' }}>{riskAssessment.reason}</span>
                          )}
                        </div>
                      )}
                    </div>
                    <div style={{ display: 'flex', flexDirection: 'column', gap: 8, justifyContent: 'center' }}>
                      <Space>
                        <span style={{ fontSize: 12, color: '#666' }}>{t('commands.concurrency')}</span>
                        <InputNumber min={1} max={100} value={concurrency} onChange={(v) => setConcurrency(v || 10)} style={{ width: 70 }} />
                      </Space>
                      <Space>
                        <span style={{ fontSize: 12, color: '#666' }}>{t('commands.timeout')}</span>
                        <InputNumber min={1} max={3600} value={timeout} onChange={(v) => setTimeout_(v || 30)} style={{ width: 70 }} addonAfter={t('commands.seconds')} />
                      </Space>
                      <Space>
                        <Tooltip title={t('commands.aiRiskAssessment')}>
                          <Button icon={<SafetyOutlined />} onClick={handleRiskAssessment} loading={riskLoading}
                            disabled={!command.trim()}>
                            {t('commands.assess')}
                          </Button>
                        </Tooltip>
                        <Button type="primary" icon={<PlayCircleOutlined />} onClick={handleExecute} loading={executing} disabled={!!activeTaskId}>
                          {t('commands.execute')}
                        </Button>
                        {activeTaskId && (
                          <Button danger icon={<StopOutlined />} onClick={handleStop}>
                            {t('commands.stop')}
                          </Button>
                        )}
                      </Space>
                    </div>
                  </div>
                  {selectedHostIds.length > 0 && (
                    <div style={{ fontSize: 12, color: '#666' }}>
                      {t('commands.targetHosts', { hosts: selectedHosts.map((h: { name: string; ip: string }) => h.name || h.ip).join(', ') })}
                    </div>
                  )}
                </Space>
              </Card>

              {/* 执行统计 */}
              {results.length > 0 && (
                <Row gutter={16} style={{ marginBottom: 12 }}>
                  <Col span={6}><Card size="small"><Statistic title={t('commands.total')} value={results.length} /></Card></Col>
                  <Col span={6}><Card size="small"><Statistic title={t('commands.success')} value={successResults.length} valueStyle={{ color: '#52c41a' }} prefix={<CheckCircleOutlined />} /></Card></Col>
                  <Col span={6}><Card size="small"><Statistic title={t('commands.failed')} value={failResults.length} valueStyle={{ color: '#ff4d4f' }} prefix={<CloseCircleOutlined />} /></Card></Col>
                  <Col span={6}><Card size="small"><Statistic title={t('commands.running')} value={runningResults.length} valueStyle={{ color: '#1677ff' }} prefix={<LoadingOutlined />} /></Card></Col>
                </Row>
              )}

              {/* 执行结果 */}
              <Card size="small" title={tText('commands.results')} extra={
                <Space>
                  {failResults.length > 0 && (
                    <Button size="small" type="primary" danger icon={<PlayCircleOutlined />} onClick={reExecuteFailed}>
                      {t('commands.retryFailed', { count: failResults.length })}
                    </Button>
                  )}
                  <Button size="small" icon={<ClearOutlined />} onClick={() => { cleanupListeners(); setResults([]); setActiveTaskId(null); setExecuting(false); }}>{t('common.clear')}</Button>
                </Space>
              }>
                {results.length > 0 ? (
                  <Table
                    rowKey="hostId"
                    columns={resultColumns}
                    dataSource={results}
                    size="small"
                    pagination={false}
                    scroll={{ y: 300 }}
                    expandable={{
                      defaultExpandAllRows: true,
                      expandedRowRender: (record) => (
                        <div className="execution-output" style={{ maxHeight: 200 }}>
                          {record.output ? renderAnsiOutput(record.output) : (record.status === 'pending' ? t('commands.waitingExecute') : record.status === 'running' ? t('commands.runningOutput') : t('common.noOutput'))}
                        </div>
                      ),
                    }}
                  />
                ) : (
                  <Empty description={t('commands.viewResultsAfterExecute')} />
                )}
              </Card>
            </div>
          ),
        },
        {
          key: 'history',
          label: t('commands.historyTab'),
          children: (
            <Card size="small">
              <Table
                rowKey="id"
                columns={historyColumns}
            dataSource={history.map((item) => ({
              id: item.id,
              command: item.command,
              hostCount: item.hostCount,
              successCount: item.successCount,
              failCount: item.failCount,
              startedAt: item.startedAt,
              duration: item.duration,
            }))}
                size="small"
                pagination={{ pageSize: 20 }}
              />
            </Card>
          ),
        },
      ]} />

      {/* 危险命令确认 */}
      <Modal
        title={<><WarningOutlined style={{ color: '#faad14' }} /> {t('commands.dangerWarning')}</>}
        open={dangerModalVisible}
        onOk={() => { setDangerModalVisible(false); doExecute(); }}
        onCancel={() => setDangerModalVisible(false)}
        okText={t('commands.confirmExecute')}
        cancelText={t('common.cancel')}
        okButtonProps={{ danger: true }}
      >
        <p>{t('commands.dangerMatched')}</p>
        <div style={{ marginBottom: 8 }}>
          {dangerMatchedRules.map((name) => (
            <Tag key={name} color="warning">{name}</Tag>
          ))}
        </div>
        <div className="execution-output">{renderAnsiOutput(dangerInfo)}</div>
        <p style={{ marginTop: 12, color: '#ff4d4f' }}>{t('commands.confirmContinue')}</p>
      </Modal>

      {/* 命令库选择 */}
      <Modal
        title="从命令库选择"
        open={cmdLibModalOpen}
        onCancel={() => setCmdLibModalOpen(false)}
        footer={null}
        width={600}
      >
        <Input.Search
          placeholder="搜索命令..."
          style={{ marginBottom: 12 }}
          allowClear
        />
        <div style={{ maxHeight: 400, overflow: 'auto' }}>
          {commands.map((cmd) => (
            <div
              key={cmd.id}
              style={{
                padding: '8px 12px',
                border: '1px solid #f0f0f0',
                borderRadius: 6,
                marginBottom: 4,
                cursor: 'pointer',
                display: 'flex',
                justifyContent: 'space-between',
                alignItems: 'center',
              }}
              onClick={() => {
                setCommand(cmd.command);
                setRiskAssessment(null);
                setCmdLibModalOpen(false);
                message.success(`已选择: ${cmd.name}`);
              }}
            >
              <div>
                <div style={{ fontWeight: 500 }}>{cmd.name}</div>
                <code style={{ fontSize: 12, color: '#666' }}>{cmd.command}</code>
              </div>
              <Tag color={cmd.risk === 'low' ? 'green' : cmd.risk === 'medium' ? 'orange' : 'red'}>
                {cmd.risk}
              </Tag>
            </div>
          ))}
        </div>
      </Modal>

      {/* 执行历史回放 */}
      <Modal
        title="执行历史回放"
        open={replayModalOpen}
        onCancel={() => { setReplayModalOpen(false); setReplayDetails([]); }}
        width={800}
        footer={<Button onClick={() => { setReplayModalOpen(false); setReplayDetails([]); }}>关闭</Button>}
      >
        {replayDetails.length > 0 ? (
          <Table
            rowKey="host_id"
            size="small"
            pagination={false}
            scroll={{ y: 400 }}
            dataSource={replayDetails}
            expandable={{
              defaultExpandAllRows: true,
              expandedRowRender: (record) => (
                <div className="execution-output" style={{ maxHeight: 250 }}>{record.output ? renderAnsiOutput(record.output) : '无输出'}</div>
              ),
            }}
            columns={[
              { title: '主机', dataIndex: 'host_name', width: 200 },
              { title: '状态', dataIndex: 'status', width: 80,
                render: (s: string) => <Badge status={s === 'success' ? 'success' : 'error'} text={s === 'success' ? '成功' : '失败'} />,
              },
              { title: '退出码', dataIndex: 'exit_code', width: 80 },
              { title: '耗时', dataIndex: 'duration', width: 100, render: (d: number) => d > 0 ? `${d}ms` : '-' },
            ]}
          />
        ) : (
          <Empty description="暂无执行详情" />
        )}
      </Modal>

      {/* Asciinema recording replay */}
      <AsciinemaPlayer
        open={asciinemaOpen}
        historyId={asciinemaHistoryId}
        onClose={() => setAsciinemaOpen(false)}
      />
    </div>
  );
}
