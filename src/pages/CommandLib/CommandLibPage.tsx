import { useEffect, useState, useMemo } from 'react';
import {
  Input, Button, Select, Tooltip, message,
  Modal, Form, Tag,
} from '../../components/ui';
import {
  SearchOutlined, PlusOutlined,
  CopyOutlined, DeleteOutlined,
  ThunderboltOutlined, CodeOutlined,
} from '../../components/ui/icons';
import { useLibraryStore } from '../../stores/library';
import { useTranslation } from '../../i18n';
import type { TranslationKey } from '../../i18n';
import type { CommandEntry } from '../../types';
import { logHandledError } from '../../utils/globalLogger';
import '../../styles/pages/command-lib.css';

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

const KIND_OPTIONS = [
  { value: 'all', label: '全部' },
  { value: 'command', label: '命令' },
  { value: 'docker', label: 'Docker' },
] as const;

const PLATFORM_BADGE: Record<CommandEntry['platform'], string> = {
  linux: 'Linux',
  windows: 'Win',
  both: '通用',
};

interface CommandFormValues extends Omit<CommandEntry, 'id' | 'starred' | 'isBuiltin'> {}

interface CommandLibPageProps {
  /** 嵌入设置面板时使用，隐藏外层标题并去掉卡片边框 */
  embedded?: boolean;
}

export default function CommandLibPage({ embedded = false }: CommandLibPageProps) {
  const { commands, loadCommands, addCommand, deleteCommand, addQuickAction } = useLibraryStore();
  const { t, tText } = useTranslation();
  const [searchQuery, setSearchQuery] = useState('');
  const [selectedCategory, setSelectedCategory] = useState<string | null>(null);
  const [selectedKind, setSelectedKind] = useState<'all' | 'command' | 'docker'>('all');
  const [addModalOpen, setAddModalOpen] = useState(false);
  const [form] = Form.useForm<CommandFormValues>();

  useEffect(() => {
    loadCommands();
  }, [loadCommands]);

  // 按类目聚合计数，供左侧导航显示
  const categoryCounts = useMemo(() => {
    const map = new Map<string, number>();
    for (const command of commands) {
      const category = command.category.trim();
      if (category) map.set(category, (map.get(category) ?? 0) + 1);
    }
    return map;
  }, [commands]);

  const categories = useMemo(() => Array.from(categoryCounts.keys()).sort(), [categoryCounts]);

  const filtered = useMemo(() => {
    const query = searchQuery.trim().toLowerCase();
    return commands.filter((c) => {
      const commandKind = c.kind ?? 'command';
      if (selectedKind !== 'all' && commandKind !== selectedKind) return false;
      if (selectedCategory && c.category !== selectedCategory) return false;
      if (query) {
        return (
          c.name.toLowerCase().includes(query) ||
          c.command.toLowerCase().includes(query) ||
          c.description.toLowerCase().includes(query) ||
          c.tags.some((tag) => tag.toLowerCase().includes(query))
        );
      }
      return true;
    });
  }, [commands, selectedKind, selectedCategory, searchQuery]);

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

  const handleCopy = (cmd: CommandEntry) => {
    navigator.clipboard.writeText(cmd.url ? `curl -sSL '${cmd.url}' | bash` : cmd.command);
    message.success(tText('commandLib.copied'));
  };

  const handleAdd = async () => {
    try {
      const values = await form.validateFields();
      await addCommand({
        ...values,
        kind: values.kind || 'command',
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

  const categoryOptions = useMemo(
    () => categories.map((category) => ({ value: category, label: category })),
    [categories],
  );

  const riskOptions = useMemo(
    () => RISK_OPTION_KEYS.map((r) => ({ value: r.value, label: tText(r.labelKey) })),
    [tText],
  );

  const platformOptions = useMemo(
    () => PLATFORM_KEYS.map((p) => ({
      value: p.value,
      label: p.value === 'linux' ? 'Linux' : p.value === 'windows' ? 'Windows' : tText(p.labelKey),
    })),
    [tText],
  );

  const renderCommandText = (cmd: CommandEntry) =>
    cmd.url ? `curl -sSL '${cmd.url}' | bash` : cmd.command;

  return (
    <div className={`clib${embedded ? ' clib-embedded' : ''}`}>
      {/* 顶部：标题 / 搜索 / 操作 */}
      <div className="clib-topbar">
        {!embedded && <h2 className="clib-title">{t('commandLib.title')}</h2>}
        <div className="clib-search">
          <Input
            placeholder={tText('commandLib.searchPlaceholder')}
            prefix={<SearchOutlined />}
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            allowClear
          />
        </div>
        <Select
          value={selectedKind}
          onChange={(value) => setSelectedKind(value as 'all' | 'command' | 'docker')}
          options={KIND_OPTIONS.map((option) => ({ value: option.value, label: option.label }))}
          className="clib-kind-select"
        />
        <span className="clib-count">
          {t('commandLib.count', { filtered: filtered.length, total: commands.length })}
        </span>
        <Button type="primary" icon={<PlusOutlined />} onClick={() => setAddModalOpen(true)}>
          {t('commandLib.addCommand')}
        </Button>
      </div>

      {/* 主体：左侧类目栏 + 右侧命令列表 */}
      <div className="clib-body">
        <aside className="clib-categories" aria-label={tText('quickActions.category')}>
          <button
            type="button"
            className={`clib-cat${!selectedCategory ? ' clib-cat-active' : ''}`}
            onClick={() => setSelectedCategory(null)}
          >
            <span className="clib-cat-name">{t('library.category.all')}</span>
            <span className="clib-cat-count">{commands.length}</span>
          </button>
          {categories.map((cat) => (
            <button
              key={cat}
              type="button"
              className={`clib-cat${selectedCategory === cat ? ' clib-cat-active' : ''}`}
              onClick={() => setSelectedCategory(cat)}
              title={cat}
            >
              <span className="clib-cat-name">{cat}</span>
              <span className="clib-cat-count">{categoryCounts.get(cat) ?? 0}</span>
            </button>
          ))}
        </aside>

        <div className="clib-list">
          {filtered.length === 0 ? (
            <div className="clib-empty">{t('commandLib.noCommands')}</div>
          ) : (
            filtered.map((cmd) => {
              const isHighRisk = cmd.risk === 'high' || cmd.risk === 'critical';
              return (
                <div key={cmd.id} className="clib-row" title={cmd.description || cmd.name}>
                  <div className="clib-row-main">
                    <div className="clib-row-head">
                      <CodeOutlined style={{ color: 'var(--color-text-muted)', opacity: 0.7 }} />
                      <span className="clib-row-name">{cmd.name}</span>
                      {cmd.kind === 'docker' && (
                        <Tag className="clib-row-tag clib-row-tag-docker">Docker</Tag>
                      )}
                      {cmd.url && (
                        <Tag className="clib-row-tag clib-row-tag-remote">{t('library.remote')}</Tag>
                      )}
                      {cmd.parameters.length > 0 && (
                        <Tag className="clib-row-tag">{t('library.parameters')}</Tag>
                      )}
                    </div>
                    <code className="clib-row-code" title={renderCommandText(cmd)}>
                      {renderCommandText(cmd)}
                    </code>
                  </div>

                  <div className="clib-row-meta">
                    <span
                      className={`clib-risk ${isHighRisk ? 'clib-risk-high' : `clib-risk-${cmd.risk}`}`}
                    >
                      <span className="clib-risk-dot" />
                      {t(RISK_LABEL_KEYS[cmd.risk])}
                    </span>
                    {cmd.platform !== 'linux' && (
                      <Tag className="clib-row-tag clib-row-tag-platform">
                        {PLATFORM_BADGE[cmd.platform]}
                      </Tag>
                    )}
                    <span className="clib-row-category">{cmd.category}</span>
                  </div>

                  <div className="clib-row-actions">
                    <Tooltip title={tText('commandLib.copyCommand')}>
                      <button
                        type="button"
                        className="clib-act"
                        onClick={() => handleCopy(cmd)}
                        aria-label={tText('commandLib.copyCommand')}
                      >
                        <CopyOutlined />
                      </button>
                    </Tooltip>
                    <Tooltip title={tText('library.addToQuickActions')}>
                      <button
                        type="button"
                        className="clib-act"
                        onClick={() => handleAddToQuickActions(cmd)}
                        aria-label={tText('library.addToQuickActions')}
                      >
                        <ThunderboltOutlined />
                      </button>
                    </Tooltip>
                    {!cmd.isBuiltin && (
                      <Tooltip title={tText('common.delete')}>
                        <button
                          type="button"
                          className="clib-act clib-act-danger"
                          onClick={() => handleDelete(cmd.id)}
                          aria-label={tText('common.delete')}
                        >
                          <DeleteOutlined />
                        </button>
                      </Tooltip>
                    )}
                  </div>
                </div>
              );
            })
          )}
        </div>
      </div>

      {/* 添加自定义命令 */}
      <Modal
        title={tText('commandLib.addCustomCommand')}
        open={addModalOpen}
        onOk={handleAdd}
        onCancel={() => { setAddModalOpen(false); form.resetFields(); }}
        width={580}
        destroyOnHidden
        okText={tText('common.add')}
        cancelText={tText('common.cancel')}
      >
        <Form form={form} layout="vertical" style={{ marginTop: 16 }}>
          <div className="clib-form-row">
            <Form.Item name="name" label={tText('commandLib.commandName')} rules={[{ required: true, message: tText('commandLib.enterCommandName') }]}>
              <Input placeholder={tText('commandLib.commandNamePlaceholder')} />
            </Form.Item>
            <Form.Item name="kind" label="类型" initialValue="command">
              <Select options={KIND_OPTIONS.filter((option) => option.value !== 'all').map((option) => ({ value: option.value, label: option.label }))} />
            </Form.Item>
          </div>

          <div className="clib-form-row">
            <Form.Item name="category" label={tText('quickActions.category')} rules={[{ required: true, message: tText('scriptLib.selectCategoryRequired') }]}>
              <Select options={categoryOptions} placeholder={tText('scriptLib.selectCategory')} />
            </Form.Item>
            <div />
          </div>

          <Form.Item name="command" label={tText('commandLib.commandContent')} rules={[{ required: true, message: tText('commandLib.enterCommandContent') }]}>
            <Input.TextArea rows={3} placeholder={tText('commandLib.commandContentPlaceholder')} style={{ fontFamily: 'monospace', fontSize: 13 }} />
          </Form.Item>

          <Form.Item name="url" label={tText('commandLib.remoteUrl')} extra={tText('commandLib.remoteUrlExtra')}>
            <Input placeholder={tText('commandLib.remoteUrlPlaceholder')} style={{ fontFamily: 'monospace', fontSize: 13 }} />
          </Form.Item>

          <Form.Item name="description" label={tText('scriptLib.description')}>
            <Input placeholder={tText('quickActions.descriptionPlaceholder')} />
          </Form.Item>

          <div className="clib-form-row">
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
