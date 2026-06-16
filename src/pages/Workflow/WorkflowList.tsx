import { Badge, Button, Popconfirm, Space, Tag, message } from '../../components/ui';
import { ApartmentOutlined, PlusOutlined, PlayCircleOutlined, DeleteOutlined, EditOutlined, SaveOutlined } from '../../components/ui/icons';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from '../../i18n';
import type { WorkflowRecord } from '../../stores/workflow';
import { stringifyWorkflowNodesPayload } from '../../stores/workflow';
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

  if (workflows.length === 0) {
    return (
      <div className="workflow-empty-card">
        <div className="workflow-empty-visual"><ApartmentOutlined /></div>
        <h3>{tText('wfList.noWorkflows')}</h3>
        <p>{tText('wfList.emptyDesc')}</p>
        <Space wrap>
          <Button icon={<PlusOutlined />} disabled={actionsDisabled} onClick={onOpenTemplateCreate}>{tText('workflow.createFromTemplate')}</Button>
          <Button type="primary" icon={<PlusOutlined />} disabled={actionsDisabled} onClick={onOpenCreate}>{tText('workflow.createWorkflow')}</Button>
        </Space>
      </div>
    );
  }

  return (
    <div className="workflow-card-grid">
      {workflows.map((workflow) => {
        const isExecuting = executingWorkflowIds.includes(workflow.id);
        const hostIds = collectWorkflowHostIds(workflow.nodes);
        const canExecute = workflow.nodes.length > 0 && hostIds.length > 0 && !actionsDisabled;

        return (
          <article key={workflow.id} className={`workflow-card workflow-card-${workflow.status}`}>
            <div className="workflow-card-header">
              <div className="workflow-card-title-row">
                <span className="workflow-card-icon"><ApartmentOutlined /></span>
                <div className="workflow-card-title-text">
                  <span className="workflow-card-name" title={workflow.name}>{workflow.name}</span>
                  <span className="workflow-card-updated">{tText('wfList.updatedAt', { time: workflow.updatedAt || '-' })}</span>
                </div>
              </div>
              <Badge status={workflow.status === 'ready' ? 'success' : 'warning'} text={tText(workflow.status === 'ready' ? 'wfList.ready' : 'wfList.draft')} />
            </div>

            <p className="workflow-card-description">{workflow.description || tText('wfList.noDescription')}</p>

            <div className="workflow-card-meta-row">
              <span className="workflow-meta-label">
                <strong>{workflow.nodes.length}</strong>
                {tText('wfList.nodeCount')}
              </span>
              <span className="workflow-meta-label">
                <strong>{workflow.connections.length}</strong>
                {tText('wfList.connectionCount')}
              </span>
              <span className="workflow-meta-label workflow-meta-hosts">
                <strong>{hostIds.length}</strong>
                {tText('wfList.targetHosts')}
              </span>
            </div>

            <div className="workflow-card-footer">
              <Tag color={canExecute || isExecuting ? 'green' : 'orange'}>
                {canExecute || isExecuting ? tText('wfList.executable') : tText('wfList.needsConfig')}
              </Tag>
              <Space size={4} wrap>
                <Button
                  size="small"
                  type="primary"
                  icon={<PlayCircleOutlined />}
                  disabled={!canExecute}
                  loading={isExecuting}
                  onClick={() => onExecute(workflow)}
                >
                  {isExecuting ? tText('wfList.executing') : tText('wfList.execute')}
                </Button>
                <Button size="small" icon={<EditOutlined />} disabled={actionsDisabled} onClick={() => onEdit(workflow)}>
                  {tText('wfList.edit')}
                </Button>
                <Button size="small" icon={<SaveOutlined />} disabled={actionsDisabled} onClick={async () => {
                  await invoke('save_workflow_template', {
                    name: workflow.name + ' ' + tText('workflow.templateTab'),
                    description: workflow.description,
                    nodes: stringifyWorkflowNodesPayload(workflow.nodes, workflow.settings),
                    connections: JSON.stringify(workflow.connections),
                  });
                  message.success(tText('wfList.savedAsTemplate'));
                }}>
                  {tText('wfList.saveAsTemplate')}
                </Button>
                <Popconfirm title={tText('wfList.confirmDelete')} onConfirm={() => onDelete(workflow.id)}>
                  <Button size="small" danger icon={<DeleteOutlined />} disabled={actionsDisabled} />
                </Popconfirm>
              </Space>
            </div>
          </article>
        );
      })}
    </div>
  );
}
