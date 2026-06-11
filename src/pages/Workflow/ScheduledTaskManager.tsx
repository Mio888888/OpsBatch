import { useState } from 'react';
import { Button, Form, Input, Modal, Popconfirm, Select, Switch, Table, message } from '../../components/ui';
import { ClockCircleOutlined, DeleteOutlined, PlusOutlined } from '../../components/ui/icons';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from '../../i18n';
import type { WorkflowRecord } from '../../stores/workflow';
import { logHandledError } from '../../utils/globalLogger';

export interface ScheduledTask {
  id: string;
  name: string;
  cron: string;
  workflow_id: string;
  enabled: boolean;
  last_run_at: string | null;
  next_run_at: string | null;
  created_at: string;
}

interface ScheduledTaskFormValues {
  name: string;
  workflow_id: string;
  cron: string;
}

interface Props {
  workflows: WorkflowRecord[];
  tasks: ScheduledTask[];
  onLoad: () => void;
}

export default function ScheduledTaskManager({ workflows, tasks, onLoad }: Props) {
  const { tText } = useTranslation();
  const [modalOpen, setModalOpen] = useState(false);
  const [form] = Form.useForm<ScheduledTaskFormValues>();

  const handleAdd = async () => {
    try {
      const values = await form.validateFields();
      await invoke('add_scheduled_task', {
        name: values.name,
        cron: values.cron,
        workflowId: values.workflow_id,
      });
      message.success(tText('scheduledTask.created'));
      setModalOpen(false);
      form.resetFields();
      onLoad();
    } catch (error) {
      void logHandledError('scheduledTask.add', error, 'warn');
    }
  };

  const handleToggle = async (id: string, enabled: boolean) => {
    const task = tasks.find((t) => t.id === id);
    if (!task) return;
    await invoke('update_scheduled_task', {
      id,
      name: task.name,
      cron: task.cron,
      workflowId: task.workflow_id,
      enabled,
    });
    onLoad();
  };

  const handleDelete = async (id: string) => {
    await invoke('delete_scheduled_task', { id });
    message.success(tText('scheduledTask.deleted'));
    onLoad();
  };

  const columns = [
    {
      title: tText('scheduledTask.nameCol'),
      dataIndex: 'name',
      width: 220,
      render: (name: string) => (
        <span className="workflow-table-name">
          <ClockCircleOutlined />
          <span title={name}>{name}</span>
        </span>
      ),
    },
    {
      title: tText('scheduledTask.ruleCol'), dataIndex: 'cron', width: 200,
      render: (c: string) => <code className="workflow-inline-code">{c}</code>,
    },
    {
      title: tText('scheduledTask.workflowCol'), dataIndex: 'workflow_id', width: 200,
      render: (wfId: string) => {
        const wf = workflows.find((w) => w.id === wfId);
        return wf ? wf.name : <span style={{ color: '#999' }}>{tText('scheduledTask.deletedWorkflow')}</span>;
      },
    },
    {
      title: tText('scheduledTask.enableCol'), dataIndex: 'enabled', width: 80,
      render: (e: boolean, r: ScheduledTask) =>
        <Switch size="small" checked={e} onChange={(v) => handleToggle(r.id, v)} />,
    },
    {
      title: tText('scheduledTask.lastRunCol'), dataIndex: 'last_run_at', width: 170,
      render: (t: string | null) => t || '-',
    },
    {
      title: tText('common.action'), width: 80,
      render: (_: unknown, r: ScheduledTask) => (
        <Popconfirm title={tText('scheduledTask.confirmDeleteTask')} onConfirm={() => handleDelete(r.id)}>
          <Button size="small" danger icon={<DeleteOutlined />} />
        </Popconfirm>
      ),
    },
  ];

  return (
    <>
      <div className="workflow-section-header">
        <div>
          <span className="workflow-eyebrow">{tText('workflow.scheduledTab')}</span>
          <h3>{tText('scheduledTask.title')}</h3>
          <p>{tText('scheduledTask.description')}</p>
        </div>
        <Button type="primary" icon={<PlusOutlined />} onClick={() => { form.resetFields(); setModalOpen(true); }}>
          {tText('scheduledTask.createTask')}
        </Button>
      </div>

      {tasks.length > 0 ? (
        <div className="workflow-table-panel">
          <Table rowKey="id" columns={columns} dataSource={tasks} size="small" />
        </div>
      ) : (
        <div className="workflow-empty-card workflow-empty-card-compact">
          <div className="workflow-empty-visual"><ClockCircleOutlined /></div>
          <h3>{tText('scheduledTask.noTasks')}</h3>
          <p>{tText('scheduledTask.emptyDesc')}</p>
          <Button type="primary" icon={<PlusOutlined />} onClick={() => setModalOpen(true)}>
            {tText('scheduledTask.createTaskBtn')}
          </Button>
        </div>
      )}

      <Modal title={tText('scheduledTask.createModal')} open={modalOpen} onOk={handleAdd} onCancel={() => setModalOpen(false)} destroyOnHidden>
        <Form form={form} layout="vertical" style={{ marginTop: 16 }}>
          <Form.Item name="name" label={tText('scheduledTask.taskName')} rules={[{ required: true, message: tText('common.required') }]}>
            <Input placeholder={tText('scheduledTask.taskNamePlaceholder')} />
          </Form.Item>
          <Form.Item name="workflow_id" label={tText('scheduledTask.linkedWorkflow')} rules={[{ required: true, message: tText('common.required') }]}>
            <Select placeholder={tText('scheduledTask.selectWorkflow')} options={workflows.map((w) => ({ value: w.id, label: w.name }))} />
          </Form.Item>
          <Form.Item name="cron" label={tText('scheduledTask.scheduleRule')} rules={[{ required: true, message: tText('common.required') }]}
            extra={tText('scheduledTask.scheduleRuleExtra')}>
            <Input placeholder={tText('scheduledTask.schedulePlaceholder')} style={{ fontFamily: 'monospace' }} />
          </Form.Item>
        </Form>
      </Modal>
    </>
  );
}
