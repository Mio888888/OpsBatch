import { useState, useEffect, useCallback, useMemo, useRef } from 'react';
import { Button, Modal, Form, Input, Tabs, message, Empty, Table } from '../../components/ui';
import { PlusOutlined, ThunderboltOutlined } from '../../components/ui/icons';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from '../../i18n';
import { useWorkflowStore } from '../../stores/workflow';
import type { WorkflowRecord, CanvasNode, Connection } from '../../stores/workflow';
import WorkflowList from './WorkflowList';
import WorkflowEditor from './WorkflowEditor';
import TemplateManager, { type WorkflowTemplate } from './TemplateManager';
import ScheduledTaskManager, { type ScheduledTask } from './ScheduledTaskManager';
import ExecutionLogPanel from './components/ExecutionLogPanel';
import { executeWorkflow, type ProgressEvent } from './workflowExecutor';
import { collectWorkflowHostIds, createWorkflowLogEntry, type LogEntry } from './workflowExecutionLogs';

interface WorkflowFormValues {
  name: string;
  description?: string;
}

export default function WorkflowPage() {
  const { tText } = useTranslation();
  const { workflows, loadWorkflows, createWorkflow, saveWorkflow, deleteWorkflow } = useWorkflowStore();
  const [activeTab, setActiveTab] = useState('list');
  const [editingWf, setEditingWf] = useState<WorkflowRecord | null>(null);

  // Create modal
  const [createModalOpen, setCreateModalOpen] = useState(false);
  const [createForm] = Form.useForm<WorkflowFormValues>();

  // Templates
  const [templates, setTemplates] = useState<WorkflowTemplate[]>([]);
  const [createFromTemplateModalOpen, setCreateFromTemplateModalOpen] = useState(false);

  // Scheduled tasks
  const [scheduledTasks, setScheduledTasks] = useState<ScheduledTask[]>([]);

  // List execution
  const runningWorkflowIdsRef = useRef<Set<string>>(new Set());
  const [executingWorkflowIds, setExecutingWorkflowIds] = useState<string[]>([]);
  const [executionModalOpen, setExecutionModalOpen] = useState(false);
  const [executionWorkflowName, setExecutionWorkflowName] = useState('');
  const [executionLogs, setExecutionLogs] = useState<LogEntry[]>([]);
  const [executionLogCollapsed, setExecutionLogCollapsed] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [confirmName, setConfirmName] = useState('');
  const [confirmDesc, setConfirmDesc] = useState('');
  const [confirmResolve, setConfirmResolve] = useState<((v: boolean) => void) | null>(null);

  const workflowStats = useMemo(() => {
    return {
      total: workflows.length,
      templates: templates.length,
      scheduled: scheduledTasks.length,
      running: executingWorkflowIds.length,
    };
  }, [workflows, templates.length, scheduledTasks.length, executingWorkflowIds.length]);

  const renderTabLabel = (label: string, count: number) => (
    <span className="workflow-tab-label">
      <span>{label}</span>
      <span className="workflow-tab-count">{count}</span>
    </span>
  );

  useEffect(() => {
    loadWorkflows();
    loadTemplates();
    loadScheduledTasks();
  }, [loadWorkflows]);

  const loadTemplates = async () => {
    try {
      const list = await invoke<WorkflowTemplate[]>('list_workflow_templates');
      setTemplates(list);
    } catch {
      setTemplates([]);
    }
  };

  const loadScheduledTasks = async () => {
    try {
      const list = await invoke<ScheduledTask[]>('list_scheduled_tasks');
      setScheduledTasks(list);
    } catch {
      setScheduledTasks([]);
    }
  };

  const handleCreate = async () => {
    try {
      const values = await createForm.validateFields();
      const wf = await createWorkflow(values.name, values.description || '');
      message.success(tText('workflow.workflowCreated'));
      setCreateModalOpen(false);
      createForm.resetFields();
      setEditingWf(wf);
    } catch {}
  };

  const handleCreateFromTemplate = async (template: WorkflowTemplate) => {
    try {
      const wf = await createWorkflow(template.name, template.description);
      const nodes = JSON.parse(template.nodes || '[]') as CanvasNode[];
      const connections = JSON.parse(template.connections || '[]') as Connection[];
      await saveWorkflow(wf.id, wf.name, wf.description, nodes, connections, 'draft');
      setEditingWf({ ...wf, nodes, connections, status: 'ready', selectedHostIds: [] });
      setCreateFromTemplateModalOpen(false);
      message.success(tText('workflow.createdFromTemplate'));
    } catch {}
  };

  const handleDelete = async (id: string) => {
    await deleteWorkflow(id);
    message.success(tText('library.deleted'));
  };

  const waitForConfirmation = useCallback((nodeName: string, description: string): Promise<boolean> => {
    return new Promise((resolve) => {
      setConfirmName(nodeName);
      setConfirmDesc(description);
      setConfirmOpen(true);
      setConfirmResolve(() => resolve);
    });
  }, []);

  const handleExecuteFromList = useCallback(async (workflow: WorkflowRecord) => {
    if (runningWorkflowIdsRef.current.has(workflow.id)) {
      message.warning(tText('wfList.workflowAlreadyExecuting'));
      return;
    }

    if (runningWorkflowIdsRef.current.size > 0) {
      message.warning(tText('wfList.anotherWorkflowExecuting'));
      return;
    }

    if (workflow.nodes.length === 0) {
      message.warning(tText('wfList.noNodesToExecute'));
      return;
    }

    const hostIds = collectWorkflowHostIds(workflow.nodes);
    if (hostIds.length === 0) {
      message.warning(tText('wfList.noTargetHosts'));
      return;
    }

    runningWorkflowIdsRef.current.add(workflow.id);
    setExecutingWorkflowIds((prev) => [...prev, workflow.id]);
    setExecutionWorkflowName(workflow.name);
    setExecutionLogs([]);
    setExecutionLogCollapsed(false);
    setExecutionModalOpen(true);
    message.success(tText('wfList.executeStarted'));

    try {
      await executeWorkflow(
        workflow.nodes,
        workflow.connections,
        hostIds,
        invoke as <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>,
        waitForConfirmation,
        (event: ProgressEvent) => {
          const logEntry = createWorkflowLogEntry(event, {
            startExecute: tText('wfEditor.startExecute'),
            executeSuccess: tText('wfEditor.executeSuccess'),
            executeFail: tText('wfEditor.executeFail'),
            stageInfo: (values) => tText('wfEditor.stageInfo', values),
          });
          if (logEntry) {
            setExecutionLogs((prev) => [...prev, logEntry]);
          }
        },
      );
      message.success(tText('wfList.executeCompleted'));
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      setExecutionLogs((prev) => [...prev, {
        timestamp: Date.now(),
        message: tText('wfList.executeFailedWithReason', { error: errorMessage }),
        status: 'fail',
      }]);
      message.error(tText('wfList.executeFailed'));
    } finally {
      runningWorkflowIdsRef.current.delete(workflow.id);
      setExecutingWorkflowIds((prev) => prev.filter((id) => id !== workflow.id));
    }
  }, [waitForConfirmation, tText]);

  // Editor mode
  if (editingWf) {
    return (
      <WorkflowEditor
        workflow={editingWf}
        onBack={() => { setEditingWf(null); loadWorkflows(); }}
      />
    );
  }

  return (
    <div className="page-container workflow-page">
      <Tabs
        activeKey={activeTab}
        onChange={setActiveTab}
        items={[
        {
          key: 'list',
          label: renderTabLabel(tText('workflow.workflowTab'), workflowStats.total),
          children: (
            <>
              <div className="workflow-section-header">
                <div>
                  <span className="workflow-eyebrow">{tText('workflow.workflowTab')}</span>
                  <h3>{tText('workflow.workflowListTitle')}</h3>
                  <p>{tText('workflow.workflowListDesc')}</p>
                </div>
                <div className="workflow-section-actions">
                  {workflowStats.running > 0 && (
                    <span className="workflow-running-pill">
                      <ThunderboltOutlined /> {tText('workflow.runningCount', { count: workflowStats.running })}
                    </span>
                  )}
                  <Button disabled={executingWorkflowIds.length > 0} onClick={() => setCreateFromTemplateModalOpen(true)}>{tText('workflow.createFromTemplate')}</Button>
                  <Button type="primary" icon={<PlusOutlined />} disabled={executingWorkflowIds.length > 0} onClick={() => setCreateModalOpen(true)}>
                    {tText('workflow.createWorkflow')}
                  </Button>
                </div>
              </div>

              <WorkflowList
                workflows={workflows}
                executingWorkflowIds={executingWorkflowIds}
                onEdit={(wf) => setEditingWf(wf)}
                onExecute={handleExecuteFromList}
                onDelete={handleDelete}
                onOpenCreate={() => setCreateModalOpen(true)}
                onOpenTemplateCreate={() => setCreateFromTemplateModalOpen(true)}
              />

              <Modal title={tText('workflow.createWorkflowModal')} open={createModalOpen} onOk={handleCreate}
                onCancel={() => setCreateModalOpen(false)} destroyOnHidden>
                <Form form={createForm} layout="vertical" style={{ marginTop: 16 }}>
                  <Form.Item name="name" label={tText('workflow.workflowName')} rules={[{ required: true, message: tText('common.required') }]}>
                    <Input placeholder={tText('workflow.workflowNamePlaceholder')} />
                  </Form.Item>
                  <Form.Item name="description" label={tText('workflow.description')}>
                    <Input.TextArea rows={2} placeholder={tText('workflow.descriptionPlaceholder')} />
                  </Form.Item>
                </Form>
              </Modal>

              <Modal title={tText('workflow.createTemplateModal')} open={createFromTemplateModalOpen}
                onCancel={() => setCreateFromTemplateModalOpen(false)} footer={null} width={700}>
                {templates.length > 0 ? (
                  <Table rowKey="id" size="small" dataSource={templates} columns={[
                    { title: tText('workflow.templateName'), dataIndex: 'name' },
                    { title: tText('workflow.description'), dataIndex: 'description', ellipsis: true },
                    {
                      title: tText('common.action'), width: 120,
                      render: (_: unknown, t: WorkflowTemplate) => (
                        <Button size="small" type="primary" disabled={executingWorkflowIds.length > 0} onClick={() => handleCreateFromTemplate(t)}>
                          {tText('workflow.useTemplate')}
                        </Button>
                      ),
                    },
                  ]} />
                ) : (
                  <Empty description={tText('workflow.noTemplates')} />
                )}
              </Modal>

              <Modal
                title={tText('wfList.executeLogTitle', { name: executionWorkflowName })}
                open={executionModalOpen}
                onCancel={() => {
                  if (executingWorkflowIds.length === 0) setExecutionModalOpen(false);
                }}
                footer={null}
                maskClosable={executingWorkflowIds.length === 0}
                closable={executingWorkflowIds.length === 0}
                width={760}
              >
                <ExecutionLogPanel
                  logs={executionLogs}
                  collapsed={executionLogCollapsed}
                  onToggle={() => setExecutionLogCollapsed((v) => !v)}
                  labels={{ execLog: tText('wfEditor.execLog'), noLogs: tText('wfEditor.noLogs') }}
                />
              </Modal>

              <Modal
                title={tText('wfEditor.manualConfirm')}
                open={confirmOpen}
                onOk={() => { setConfirmOpen(false); confirmResolve?.(true); setConfirmResolve(null); }}
                onCancel={() => { setConfirmOpen(false); confirmResolve?.(false); setConfirmResolve(null); }}
                okText={tText('wfEditor.confirmContinue')}
                cancelText={tText('wfEditor.terminateWorkflow')}
                cancelButtonProps={{ danger: true }}
                maskClosable={false}
                closable={false}
              >
                <p style={{ fontWeight: 600, fontSize: 16, marginBottom: 8 }}>{confirmName}</p>
                {confirmDesc && <p style={{ color: '#666', marginBottom: 12 }}>{confirmDesc}</p>}
                <p>{tText('wfEditor.confirmContinueHint')}</p>
              </Modal>
            </>
          ),
        },
        {
          key: 'templates',
          label: renderTabLabel(tText('workflow.templateTab'), workflowStats.templates),
          children: (
            <>
              <div className="workflow-section-header">
                <div>
                  <span className="workflow-eyebrow">{tText('workflow.templateTab')}</span>
                  <h3>{tText('workflow.workflowTemplates')}</h3>
                  <p>{tText('workflow.templateDesc')}</p>
                </div>
              </div>
              <TemplateManager templates={templates} onLoad={loadTemplates} onUseTemplate={handleCreateFromTemplate} />
            </>
          ),
        },
        {
          key: 'scheduled',
          label: renderTabLabel(tText('workflow.scheduledTab'), workflowStats.scheduled),
          children: (
            <ScheduledTaskManager workflows={workflows} tasks={scheduledTasks} onLoad={loadScheduledTasks} />
          ),
        },
        ]}
        className="workflow-tabs"
      />
    </div>
  );
}
