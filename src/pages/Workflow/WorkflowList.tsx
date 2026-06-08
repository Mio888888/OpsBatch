import { Button, Space, Table, Tag, Empty, Card, Popconfirm, message } from '../../components/ui';
import { PlusOutlined, PlayCircleOutlined, DeleteOutlined, EditOutlined, SaveOutlined } from '../../components/ui/icons';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from '../../i18n';
import type { WorkflowRecord } from '../../stores/workflow';
import { collectWorkflowHostIds } from './workflowExecutionLogs';

interface Props {
  workflows: WorkflowRecord[];
  executingWorkflowIds?: string[];
  onEdit: (wf: WorkflowRecord) => void;
  onExecute: (wf: WorkflowRecord) => void;
  onDelete: (id: string) => Promise<void>;
  onOpenCreate: () => void;
  onOpenTemplateCreate: () => void;
}

export default function WorkflowList({ workflows, executingWorkflowIds = [], onEdit, onExecute, onDelete, onOpenCreate, onOpenTemplateCreate }: Props) {
  const { tText } = useTranslation();
  const actionsDisabled = executingWorkflowIds.length > 0;

  const columns = [
    { title: tText('wfList.name'), dataIndex: 'name', width: 200 },
    { title: tText('wfList.description'), dataIndex: 'description', ellipsis: true },
    {
      title: tText('wfList.nodeCount'), width: 80,
      render: (_: unknown, r: WorkflowRecord) => r.nodes.length,
    },
    {
      title: tText('wfList.connectionCount'), width: 80,
      render: (_: unknown, r: WorkflowRecord) => r.connections.length,
    },
    {
      title: tText('wfList.status'), dataIndex: 'status', width: 80,
      render: (s: string) => s === 'ready' ? <Tag color="green">{tText('wfList.ready')}</Tag> : <Tag color="orange">{tText('wfList.draft')}</Tag>,
    },
    {
      title: tText('wfList.action'), width: 280,
      render: (_: unknown, r: WorkflowRecord) => {
        const isExecuting = executingWorkflowIds.includes(r.id);
        const hostIds = collectWorkflowHostIds(r.nodes);
        return (
          <div style={{ whiteSpace: 'nowrap' }}>
            <Space.Compact>
              <Button size="small" type="primary" icon={<PlayCircleOutlined />}
                disabled={r.nodes.length === 0 || hostIds.length === 0 || actionsDisabled}
                loading={isExecuting}
                onClick={() => onExecute(r)}>
                <span style={{ fontSize: 12 }}>{isExecuting ? tText('wfList.executing') : tText('wfList.execute')}</span>
              </Button>
              <Button size="small" icon={<EditOutlined />} disabled={actionsDisabled} onClick={() => onEdit(r)}>
                <span style={{ fontSize: 12 }}>{tText('wfList.edit')}</span>
              </Button>
              <Button size="small" icon={<SaveOutlined />} disabled={actionsDisabled} onClick={async () => {
                await invoke('save_workflow_template', {
                  name: r.name + ' ' + tText('workflow.templateTab'),
                  description: r.description,
                  nodes: JSON.stringify(r.nodes),
                  connections: JSON.stringify(r.connections),
                });
                message.success(tText('wfList.savedAsTemplate'));
              }}>
                <span style={{ fontSize: 12 }}>{tText('wfList.saveAsTemplate')}</span>
              </Button>
              <Popconfirm title={tText('wfList.confirmDelete')} onConfirm={() => onDelete(r.id)}>
                <Button size="small" danger icon={<DeleteOutlined />} disabled={actionsDisabled} />
              </Popconfirm>
            </Space.Compact>
          </div>
        );
      },
    },
  ];

  if (workflows.length === 0) {
    return (
      <Card>
        <Empty description={
          <Space direction="vertical">
            <span>{tText('wfList.noWorkflows')}</span>
            <Space>
              <Button icon={<PlusOutlined />} disabled={actionsDisabled} onClick={onOpenTemplateCreate}>{tText('workflow.createFromTemplate')}</Button>
              <Button type="primary" icon={<PlusOutlined />} disabled={actionsDisabled} onClick={onOpenCreate}>{tText('workflow.createWorkflow')}</Button>
            </Space>
          </Space>
        } image={Empty.PRESENTED_IMAGE_SIMPLE} />
      </Card>
    );
  }

  return <Table rowKey="id" columns={columns} dataSource={workflows} size="small" />;
}
