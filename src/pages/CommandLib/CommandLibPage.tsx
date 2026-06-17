import { useEffect, useState, useMemo } from 'react';
import {
  Tag, Input, Space, Button, Select, Tooltip, message,
  Modal, Form,
} from '../../components/ui';
import {
  SearchOutlined, PlusOutlined,
  CopyOutlined, DeleteOutlined,
  ThunderboltOutlined,
} from '../../components/ui/icons';
import { useLibraryStore } from '../../stores/library';
import { useTranslation } from '../../i18n';
import type { TranslationKey } from '../../i18n';
import type { CommandEntry } from '../../types';
import { logHandledError } from '../../utils/globalLogger';
import '../../styles/pages/library.css';
import '../../styles/pages/quick-actions.css';

const RISK_LABEL_KEYS: Record<string, TranslationKey> = {
  low: 'library.risk.low',
  medium: 'library.risk.medium',
  high: 'library.risk.high',
  critical: 'library.risk.critical',
};

const RISK_OPTION_KEYS: { value: string; labelKey: TranslationKey }[] = [
  { value: 'low', labelKey: 'library.risk.lowOption' },
  { value: 'medium', labelKey: 'library.risk.mediumOption' },
  { value: 'high', labelKey: 'library.risk.highOption' },
  { value: 'critical', labelKey: 'library.risk.criticalOption' },
];

const PLATFORM_KEYS: { value: string; labelKey: TranslationKey }[] = [
  { value: 'linux', labelKey: 'commandLib.platform' },
  { value: 'windows', labelKey: 'commandLib.platform' },
  { value: 'both', labelKey: 'commandLib.platformGeneral' },
];

interface CommandFormValues extends Omit<CommandEntry, 'id' | 'starred' | 'isBuiltin'> {}

export default function CommandLibPage() {
  const { commands, loadCommands, addCommand, deleteCommand, addQuickAction } = useLibraryStore();
  const { t, tText } = useTranslation();
  const [searchQuery, setSearchQuery] = useState('');
  const [selectedCategory, setSelectedCategory] = useState<string | null>(null);
  const [addModalOpen, setAddModalOpen] = useState(false);
  const [form] = Form.useForm<CommandFormValues>();

  useEffect(() => {
    loadCommands();
  }, [loadCommands]);

  const filtered = commands.filter((c) => {
    if (selectedCategory && c.category !== selectedCategory) return false;
    if (searchQuery) {
      const q = searchQuery.toLowerCase();
      return c.name.toLowerCase().includes(q) || c.command.toLowerCase().includes(q) ||
        c.description.toLowerCase().includes(q) || c.tags.some((t) => t.toLowerCase().includes(q));
    }
    return true;
  });

  const handleDelete = async (id: string) => {
    await deleteCommand(id);
    message.success(tText('library.deleted'));
  };

  const handleAddToQuickActions = async (cmd: CommandEntry) => {
    await addQuickAction({
      name: cmd.name,
      command: cmd.command,
      category: cmd.category,
      parameters: [],
      description: cmd.description || '',
      tags: [],
      language: 'shell',
    });
    message.success(tText('library.addedToQuickActions'));
  };

  const handleAdd = async () => {
    try {
      const values = await form.validateFields();
      await addCommand({
        ...values,
        url: values.url || '',
        tags: values.tags || [],
        parameters: values.parameters || [],
        starred: false,
      });
      message.success(tText('commandLib.commandAdded'));
      setAddModalOpen(false);
      form.resetFields();
    } catch (error) {
      void logHandledError('commandLib.add', error, 'warn');
    }
  };

  const categories = useMemo(() => {
    const categorySet = new Set<string>();
    for (const command of commands) {
      if (command.category.trim()) categorySet.add(command.category);
    }
    return Array.from(categorySet).sort();
  }, [commands]);

  const categoryOptions = useMemo(() => {
    return categories.map((category) => ({ value: category, label: category }));
  }, [categories]);

  const riskOptions = useMemo(() => {
    return RISK_OPTION_KEYS.map((r) => ({ value: r.value, label: tText(r.labelKey) }));
  }, [tText]);

  const platformOptions = useMemo(() => {
    return PLATFORM_KEYS.map((p) => ({ value: p.value, label: p.value === 'linux' ? 'Linux' : p.value === 'windows' ? 'Windows' : tText(p.labelKey) }));
  }, [tText]);

  return (
    <div className="page-container">
      <div className="page-header">
        <h2>{t('commandLib.title')}</h2>
        <Space>
          <Button type="primary" icon={<PlusOutlined />} onClick={() => setAddModalOpen(true)}>
            {t('commandLib.addCommand')}
          </Button>
        </Space>
      </div>

      {/* Category Tabs */}
      <div className="qa-tabs">
        <button
          className={`qa-tab${!selectedCategory ? ' qa-tab-active' : ''}`}
          onClick={() => setSelectedCategory(null)}
        >{t('library.category.all')}</button>
        {categories.map((cat) => (
          <button
            key={cat}
            className={`qa-tab${selectedCategory === cat ? ' qa-tab-active' : ''}`}
            onClick={() => setSelectedCategory(cat)}
          >{cat}</button>
        ))}
      </div>

      {/* Toolbar */}
      <div className="qa-toolbar">
        <Input
          placeholder={tText('commandLib.searchPlaceholder')}
          prefix={<SearchOutlined />}
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          allowClear
          className="qa-search"
        />
        <span className="qa-count">{t('commandLib.count', { filtered: filtered.length, total: commands.length })}</span>
      </div>

      {/* Card Grid */}
      <div className="qa-card-grid">
        {filtered.map((cmd) => {
          const isHighRisk = cmd.risk === 'high' || cmd.risk === 'critical';
          return (
            <div key={cmd.id} className="qa-card">
              <div className="qa-card-header">
                <div className="qa-card-title-row">
                  <span className="qa-card-lang-icon">⌘</span>
                  <span className="qa-card-name">{cmd.name}</span>
                </div>
                <div className="qa-card-header-badges">
                  {cmd.url && <Tag color="cyan" className="qa-card-micro-tag">{t('library.remote')}</Tag>}
                  {cmd.parameters.length > 0 && <Tag color="blue" className="qa-card-micro-tag">{t('library.parameters')}</Tag>}
                  <Tooltip title={tText('library.addToQuickActions')}>
                    <button className="qa-star-btn" onClick={() => handleAddToQuickActions(cmd)}>
                      <ThunderboltOutlined />
                    </button>
                  </Tooltip>
                </div>
              </div>
              <code className={`qa-card-code ${cmd.url ? 'qa-card-code-remote' : ''}`}>{cmd.url ? `curl -sSL '${cmd.url}' | bash` : cmd.command}</code>
              {cmd.description && (
                <div className="qa-card-desc">{cmd.description}</div>
              )}
              <div className="qa-card-footer">
                <div className="qa-card-status">
                  <span className={`qa-card-risk-dot ${isHighRisk ? 'qa-card-risk-dot-high' : ''}`} />
                  <Tag className="qa-tag">{t(RISK_LABEL_KEYS[cmd.risk])}</Tag>
                  <Tag className="qa-tag">{cmd.category}</Tag>
                  {cmd.platform !== 'linux' && (
                    <Tag className="qa-tag">{cmd.platform === 'windows' ? 'Win' : tText('commandLib.platformGeneral')}</Tag>
                  )}
                </div>
                <Space size={2}>
                  <Tooltip title={tText('commandLib.copyCommand')}>
                    <Button type="text" size="small" icon={<CopyOutlined />}
                      onClick={() => { navigator.clipboard.writeText(cmd.url ? `curl -sSL '${cmd.url}' | bash` : cmd.command); message.success(tText('commandLib.copied')); }}
                    />
                  </Tooltip>
                  {!cmd.isBuiltin && (
                    <Tooltip title={tText('common.delete')}>
                      <Button type="text" size="small" danger icon={<DeleteOutlined />} onClick={() => handleDelete(cmd.id)} />
                    </Tooltip>
                  )}
                </Space>
              </div>
            </div>
          );
        })}
        {filtered.length === 0 && (
          <div className="qa-empty">{t('commandLib.noCommands')}</div>
        )}
      </div>

      {/* 添加自定义命令 */}
      <Modal title={tText('commandLib.addCustomCommand')} open={addModalOpen} onOk={handleAdd}
        onCancel={() => { setAddModalOpen(false); form.resetFields(); }}
        width={580} destroyOnHidden okText={tText('common.add')} cancelText={tText('common.cancel')}>
        <Form form={form} layout="vertical" style={{ marginTop: 16 }}>
          {/* Row 1: Name + Category */}
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
            <Form.Item name="name" label={tText('commandLib.commandName')} rules={[{ required: true, message: tText('commandLib.enterCommandName') }]}>
              <Input placeholder={tText('commandLib.commandNamePlaceholder')} />
            </Form.Item>
            <Form.Item name="category" label={tText('quickActions.category')} rules={[{ required: true, message: tText('scriptLib.selectCategoryRequired') }]}>
              <Select options={categoryOptions} placeholder={tText('scriptLib.selectCategory')} />
            </Form.Item>
          </div>

          {/* Command content */}
          <Form.Item name="command" label={tText('commandLib.commandContent')} rules={[{ required: true, message: tText('commandLib.enterCommandContent') }]}>
            <Input.TextArea rows={3} placeholder={tText('commandLib.commandContentPlaceholder')} style={{ fontFamily: 'monospace', fontSize: 13 }} />
          </Form.Item>

          {/* Remote script URL */}
          <Form.Item name="url" label={tText('commandLib.remoteUrl')} extra={tText('commandLib.remoteUrlExtra')}>
            <Input placeholder={tText('commandLib.remoteUrlPlaceholder')} style={{ fontFamily: 'monospace', fontSize: 13 }} />
          </Form.Item>

          {/* Description */}
          <Form.Item name="description" label={tText('scriptLib.description')}>
            <Input placeholder={tText('quickActions.descriptionPlaceholder')} />
          </Form.Item>

          {/* Row 2: Risk + Platform */}
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
            <Form.Item name="risk" label={tText('scriptLib.riskLevel')} initialValue="low">
              <Select options={riskOptions} />
            </Form.Item>
            <Form.Item name="platform" label={tText('commandLib.platform')} initialValue="linux">
              <Select options={platformOptions} />
            </Form.Item>
          </div>
        </Form>
      </Modal>
    </div>
  );
}
