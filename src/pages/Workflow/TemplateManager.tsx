import { Button, Space, Table, Popconfirm, message } from '../../components/ui';
import { DeleteOutlined } from '../../components/ui/icons';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from '../../i18n';

export interface WorkflowTemplate {
  id: string;
  name: string;
  description: string;
  nodes: string;
  connections: string;
  created_at: string;
}

interface Props {
  templates: WorkflowTemplate[];
  onLoad: () => void;
  onUseTemplate: (template: WorkflowTemplate) => void;
}

export default function TemplateManager({ templates, onLoad, onUseTemplate }: Props) {
  const { tText } = useTranslation();

  const handleDelete = async (id: string) => {
    await invoke('delete_workflow_template', { id });
    message.success(tText('workflow.templateDeleted'));
    onLoad();
  };

  const columns = [
    { title: tText('workflow.templateName'), dataIndex: 'name', width: 200 },
    { title: tText('workflow.description'), dataIndex: 'description', ellipsis: true },
    { title: tText('scheduledTask.lastRunCol'), dataIndex: 'created_at', width: 170 },
    {
      title: tText('common.action'), width: 200,
      render: (_: unknown, r: WorkflowTemplate) => (
        <Space>
          <Button size="small" type="primary" onClick={() => onUseTemplate(r)}>{tText('workflow.createFrom')}</Button>
          <Popconfirm title={tText('workflow.confirmDeleteTemplate')} onConfirm={() => handleDelete(r.id)}>
            <Button size="small" danger icon={<DeleteOutlined />} />
          </Popconfirm>
        </Space>
      ),
    },
  ];

  return <Table rowKey="id" columns={columns} dataSource={templates} size="small" />;
}
