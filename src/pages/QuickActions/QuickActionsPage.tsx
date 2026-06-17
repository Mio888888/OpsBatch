import { useEffect, useState, useMemo } from 'react';
import {
  Tag, Input, Space, Button, Modal, Form, Select, message,
  Popconfirm, Tooltip,
} from '../../components/ui';
import {
  PlusOutlined, DeleteOutlined, EditOutlined, PlayCircleOutlined,
  ImportOutlined, ExportOutlined, StarOutlined, StarFilled,
} from '../../components/ui/icons';
import { useAssetsStore } from '../../stores/assets';
import { useLibraryStore } from '../../stores/library';
import { useExecutionStore } from '../../stores/execution';
import { useTranslation } from '../../i18n';
import type { TranslationKey } from '../../i18n';
import type { QuickAction } from '../../types';
import { parseQuickActionParams, replaceQuickActionParams } from '../../utils/quickActionParams';
import { logHandledError } from '../../utils/globalLogger';
import '../../styles/pages/quick-actions.css';

interface QuickActionFormValues {
  name: string;
  command: string;
  category: string;
  description: string;
  tags: string;
  language: 'shell' | 'python' | 'powershell';
}

interface QuickActionsPageProps {
  embedded?: boolean;
}

const CATEGORY_COLORS: Record<string, string> = {
  '系统': 'blue',
  '磁盘': 'cyan',
  '内存': 'purple',
  '网络': 'green',
  '进程': 'orange',
  '安全': 'red',
  '其他': 'default',
};

const CATEGORY_OPTIONS = [
  { value: '系统', label: '系统' }, { value: '磁盘', label: '磁盘' },
  { value: '内存', label: '内存' }, { value: '网络', label: '网络' },
  { value: '进程', label: '进程' }, { value: '安全', label: '安全' },
  { value: '其他', label: '其他' },
];

const LANGUAGE_OPTIONS = [
  { value: 'shell', label: '🐚 Shell' },
  { value: 'python', label: '🐍 Python' },
  { value: 'powershell', label: '⚡ PowerShell' },
];

const LANGUAGE_ICONS: Record<string, string> = {
  shell: '🐚',
  python: '🐍',
  powershell: '⚡',
};

const LANGUAGE_LABELS: Record<string, string> = {
  shell: 'Shell',
  python: 'Python',
  powershell: 'PowerShell',
};

const STATUS_KEY_MAP: Record<string, { color: string; textKey: TranslationKey }> = {
  success: { color: '#52c41a', textKey: 'quickActions.status.success' },
  failed: { color: '#ff4d4f', textKey: 'quickActions.status.failed' },
  partial: { color: '#faad14', textKey: 'quickActions.status.partial' },
};

function formatLastRun(isoStr: string, tText: (key: TranslationKey, values?: Record<string, string | number>) => string): string {
  if (!isoStr) return '';
  try {
    const d = new Date(isoStr.replace(' ', 'T'));
    const now = new Date();
    const diffMs = now.getTime() - d.getTime();
    const diffMin = Math.floor(diffMs / 60000);
    if (diffMin < 1) return tText('quickActions.justNow');
    if (diffMin < 60) return tText('quickActions.minutesAgo', { count: diffMin });
    const diffHr = Math.floor(diffMin / 60);
    if (diffHr < 24) return tText('quickActions.hoursAgo', { count: diffHr });
    const diffDay = Math.floor(diffHr / 24);
    if (diffDay < 30) return tText('quickActions.daysAgo', { count: diffDay });
    return d.toLocaleDateString();
  } catch {
    return '';
  }
}

export default function QuickActionsPage({ embedded = false }: QuickActionsPageProps) {
  const { selectedHostIds } = useAssetsStore();
  const { quickActions, loadQuickActions, addQuickAction, updateQuickAction, deleteQuickAction, toggleStarQuickAction } = useLibraryStore();
  const executeCommand = useExecutionStore((s) => s.executeCommand);
  const { t, tText } = useTranslation();
  const [modalOpen, setModalOpen] = useState(false);
  const [editingAction, setEditingAction] = useState<QuickAction | null>(null);
  const [form] = Form.useForm<QuickActionFormValues>();

  // Filters
  const [searchQuery, setSearchQuery] = useState('');
  const [selectedCategory, setSelectedCategory] = useState<string>('全部');
  const [starredOnly, setStarredOnly] = useState(false);
  const [selectedLanguage, setSelectedLanguage] = useState<string | null>(null);

  const categories = useMemo(() => {
    const cats = new Set(quickActions.map((a) => a.category).filter(Boolean));
    return ['全部', ...cats];
  }, [quickActions]);

  const filtered = useMemo(() => {
    return quickActions.filter((a) => {
      if (selectedCategory !== '全部' && a.category !== selectedCategory) return false;
      if (starredOnly && !a.starred) return false;
      if (selectedLanguage && a.language !== selectedLanguage) return false;
      if (searchQuery) {
        const q = searchQuery.toLowerCase();
        return a.name.toLowerCase().includes(q)
          || a.command.toLowerCase().includes(q)
          || (a.description || '').toLowerCase().includes(q);
      }
      return true;
    });
  }, [quickActions, selectedCategory, starredOnly, selectedLanguage, searchQuery]);

  useEffect(() => {
    loadQuickActions();
  }, [loadQuickActions]);

  // Parameter modal state
  const [paramModalOpen, setParamModalOpen] = useState(false);
  const [paramAction, setParamAction] = useState<QuickAction | null>(null);
  const [paramForm] = Form.useForm<Record<string, string>>();

  const parsedParams = useMemo(() => {
    if (!paramAction) return [];
    return parseQuickActionParams(paramAction.command);
  }, [paramAction]);

  const handleExecuteAction = async (action: QuickAction) => {
    if (selectedHostIds.length === 0) {
      message.warning(tText('quickActions.selectHostsFirst'));
      return;
    }

    const params = parseQuickActionParams(action.command);
    if (params.length === 0) {
      try {
        await executeCommand(selectedHostIds, action.command, 10, 30, action.id);
        message.success(tText('quickActions.dispatched', { command: action.command, count: selectedHostIds.length }));
      } catch (e) {
        message.error(tText('quickActions.executeFailed', { error: String(e) }));
      }
      return;
    }

    setParamAction(action);
    const initialValues: Record<string, string> = {};
    for (const p of params) {
      initialValues[p.name] = p.defaultValue;
    }
    paramForm.setFieldsValue(initialValues);
    setParamModalOpen(true);
  };

  const handleParamSubmit = async () => {
    if (!paramAction) return;
    try {
      const values = await paramForm.validateFields();
      const finalCommand = replaceQuickActionParams(paramAction.command, values);
      await executeCommand(selectedHostIds, finalCommand, 10, 30, paramAction.id);
      message.success(tText('quickActions.dispatched', { command: finalCommand, count: selectedHostIds.length }));
      setParamModalOpen(false);
      setParamAction(null);
      paramForm.resetFields();
    } catch (e) {
      if (e && typeof e === 'object' && 'errorFields' in (e as object)) {
        return;
      }
      message.error(tText('quickActions.executeFailed', { error: String(e) }));
      setParamAction(null);
      paramForm.resetFields();
    }
  };

  const handleSave = async () => {
    try {
      const values = await form.validateFields();
      if (editingAction) {
        await updateQuickAction({
          ...editingAction,
          name: values.name,
          command: values.command,
          category: values.category,
          description: values.description || '',
          tags: values.tags ? values.tags.split(',').map((t: string) => t.trim()).filter(Boolean) : [],
          language: values.language || 'shell',
        });
        message.success(tText('quickActions.updated'));
      } else {
        await addQuickAction({
          name: values.name,
          command: values.command,
          category: values.category,
          parameters: [],
          description: values.description || '',
          tags: values.tags ? values.tags.split(',').map((t: string) => t.trim()).filter(Boolean) : [],
          language: values.language || 'shell',
        });
        message.success(tText('quickActions.added'));
      }
      setModalOpen(false);
      setEditingAction(null);
      form.resetFields();
    } catch (error) {
      void logHandledError('quickActions.save', error, 'warn');
    }
  };

  const handleExport = () => {
    const json = JSON.stringify(quickActions, null, 2);
    const blob = new Blob([json], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'quick-actions.json';
    a.click();
    URL.revokeObjectURL(url);
    message.success(tText('quickActions.exportSuccess'));
  };

  const handleImport = () => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = '.json';
    input.onchange = (e: unknown) => {
      const file = (e as React.ChangeEvent<HTMLInputElement>).target.files?.[0];
      if (!file) return;
      const reader = new FileReader();
      reader.onload = async (ev) => {
        try {
          const imported = JSON.parse(ev.target?.result as string);
          if (Array.isArray(imported)) {
            for (const item of imported) {
              await addQuickAction({
                name: item.name,
                command: item.command,
                category: item.category || '',
                parameters: item.parameters || [],
                description: item.description || '',
                tags: item.tags || [],
                language: item.language || 'shell',
              });
            }
            message.success(tText('quickActions.imported', { count: imported.length }));
          }
        } catch {
          message.error(tText('quickActions.importFailed'));
        }
      };
      reader.readAsText(file);
    };
    input.click();
  };

  const openEdit = (action: QuickAction) => {
    setEditingAction(action);
    form.setFieldsValue({
      name: action.name,
      command: action.command,
      category: action.category,
      description: action.description,
      tags: (action.tags || []).join(', '),
      language: action.language || 'shell',
    });
    setModalOpen(true);
  };

  const openCreate = () => {
    setEditingAction(null);
    form.resetFields();
    setModalOpen(true);
  };

  const clearFilters = () => {
    setSearchQuery('');
    setSelectedCategory('全部');
    setStarredOnly(false);
    setSelectedLanguage(null);
  };

  const hasActiveFilters = selectedCategory !== '全部' || starredOnly || Boolean(selectedLanguage) || Boolean(searchQuery);
  const starredCount = quickActions.filter((action) => action.starred).length;
  const parameterizedCount = quickActions.filter((action) => parseQuickActionParams(action.command).length > 0).length;

  return (
    <div className={`page-container${embedded ? ' qa-page-embedded' : ''}`}>
      {/* Management header: title only on standalone, actions always visible */}
      <div className="page-header qa-management-header">
        {!embedded && <h2>{t('quickActions.title')}</h2>}
        <div className="qa-header-right">
          <span className="qa-stat-bar">
            <span className="qa-stat-item">{t('quickActions.actionCount', { count: quickActions.length })}</span>
            {starredCount > 0 && <span className="qa-stat-item">{t('quickActions.starredCount', { count: starredCount })}</span>}
            {parameterizedCount > 0 && <span className="qa-stat-item">{t('quickActions.paramCount', { count: parameterizedCount })}</span>}
          </span>
          <Space size={8}>
            <Button icon={<ImportOutlined />} onClick={handleImport}>{t('quickActions.import')}</Button>
            <Button icon={<ExportOutlined />} onClick={handleExport}>{t('quickActions.export')}</Button>
            <Button type="primary" icon={<PlusOutlined />} onClick={openCreate}>{t('quickActions.createAction')}</Button>
          </Space>
        </div>
      </div>

      {/* Selected host banner */}
      {selectedHostIds.length > 0 && (
        <div className="qa-selected-hint">
          {t('quickActions.selectedHostHint', { count: selectedHostIds.length })}
        </div>
      )}

      {/* Category Tabs */}
      <div className="qa-tabs">
        {categories.map((cat) => {
          const count = cat === '全部'
            ? quickActions.length
            : quickActions.filter((a) => a.category === cat).length;
          return (
            <button
              key={cat}
              type="button"
              className={`qa-tab${selectedCategory === cat ? ' qa-tab-active' : ''}`}
              onClick={() => setSelectedCategory(cat)}
            >{cat}<span className="qa-tab-count">{count}</span></button>
          );
        })}
      </div>

      {/* Toolbar */}
      <div className="qa-toolbar">
        <Input
          placeholder={tText('quickActions.searchPlaceholder')}
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          allowClear
          className="qa-search"
        />
        <Button
          size="small"
          icon={starredOnly ? <StarFilled style={{ color: '#faad14' }} /> : <StarOutlined />}
          onClick={() => setStarredOnly(!starredOnly)}
          type={starredOnly ? 'primary' : 'default'}
        >{starredOnly ? t('quickActions.starred') : t('quickActions.star')}</Button>
        <Select
          allowClear
          placeholder={tText('scriptLib.language')}
          value={selectedLanguage}
          onChange={(v) => setSelectedLanguage(v || null)}
          className="qa-lang-select"
          options={[
            { value: 'shell', label: 'Shell' },
            { value: 'python', label: 'Python' },
            { value: 'powershell', label: 'PowerShell' },
          ]}
        />
        <span className="qa-count">{filtered.length} / {quickActions.length}</span>
        {hasActiveFilters && (
          <Button type="link" size="small" onClick={clearFilters}>{t('quickActions.clearFilters')}</Button>
        )}
      </div>

      {/* Card Grid */}
      <div className="qa-card-grid">
        {filtered.map((action) => {
          const params = parseQuickActionParams(action.command);
          const statusInfo = STATUS_KEY_MAP[action.lastStatus];
          const lang = action.language || 'shell';
          return (
            <div key={action.id} className="qa-card">
              <div className="qa-card-header">
                <div className="qa-card-title-row">
                  <span className="qa-card-lang-badge" data-lang={lang}>
                    {LANGUAGE_ICONS[lang]} {LANGUAGE_LABELS[lang]}
                  </span>
                  <span className="qa-card-name">{action.name}</span>
                </div>
                <Tooltip title={action.starred ? tText('quickActions.unstar') : tText('quickActions.star')}>
                  <button
                    type="button"
                    className="qa-star-btn"
                    onClick={() => toggleStarQuickAction(action.id)}
                  >
                    {action.starred ? <StarFilled style={{ color: '#faad14', fontSize: 16 }} /> : <StarOutlined style={{ fontSize: 16 }} />}
                  </button>
                </Tooltip>
              </div>
              {action.description && (
                <div className="qa-card-desc">{action.description}</div>
              )}
              <code className="qa-card-code">{action.command}</code>
              {((action.tags || []).length > 0 || params.length > 0) && (
                <div className="qa-card-tags">
                  {(action.tags || []).map((t, i) => (
                    <Tag key={i} className="qa-tag">{t}</Tag>
                  ))}
                  {params.length > 0 && (
                    <Tooltip title={params.map((p) => `${p.name}${p.defaultValue ? '=' + p.defaultValue : ''}`).join(', ')}>
                      <Tag color="blue">{t('quickActions.paramCountTag', { count: params.length })}</Tag>
                    </Tooltip>
                  )}
                </div>
              )}
              <div className="qa-card-footer">
                <div className="qa-card-status">
                  {action.category && <Tag color={CATEGORY_COLORS[action.category] || 'default'}>{action.category}</Tag>}
                  {statusInfo && (
                    <span className="qa-status-chip" style={{ color: statusInfo.color }}>
                      <span className="qa-status-dot" style={{ background: statusInfo.color }} />
                      {t(statusInfo.textKey)}
                    </span>
                  )}
                  {action.lastRunAt && (
                    <span className="qa-last-run">{formatLastRun(action.lastRunAt, tText)}</span>
                  )}
                </div>
                <Space size={4} className="qa-card-actions">
                  <Tooltip title={tText('quickActions.execute')}>
                    <Button type="text" size="small" icon={<PlayCircleOutlined />}
                      disabled={selectedHostIds.length === 0}
                      onClick={() => handleExecuteAction(action)}
                    />
                  </Tooltip>
                  <Tooltip title={tText('common.edit')}>
                    <Button type="text" size="small" icon={<EditOutlined />}
                      onClick={() => openEdit(action)}
                    />
                  </Tooltip>
                  <Popconfirm title={tText('common.confirmDelete')} onConfirm={async () => {
                    await deleteQuickAction(action.id);
                    message.success(tText('library.deleted'));
                  }}>
                    <Button type="text" size="small" danger icon={<DeleteOutlined />} />
                  </Popconfirm>
                </Space>
              </div>
            </div>
          );
        })}
        {filtered.length === 0 && (
          <div className="qa-empty">
            <div className="qa-empty-icon">📋</div>
            {quickActions.length === 0 ? (
              <>
                <h3 className="qa-empty-title">{t('quickActions.noActions')}</h3>
                <p className="qa-empty-desc">{t('quickActions.createFirst')}</p>
                <div className="qa-empty-actions">
                  <Button type="primary" icon={<PlusOutlined />} onClick={openCreate}>{t('quickActions.createAction')}</Button>
                  <Button icon={<ImportOutlined />} onClick={handleImport}>{t('quickActions.importFromFile')}</Button>
                </div>
              </>
            ) : (
              <>
                <h3 className="qa-empty-title">{t('quickActions.noMatch')}</h3>
                <p className="qa-empty-desc">{t('quickActions.noMatchHint')}</p>
                <div className="qa-empty-actions">
                  <Button onClick={clearFilters}>{t('quickActions.clearFilters')}</Button>
                </div>
              </>
            )}
          </div>
        )}
      </div>

      {/* Create/Edit Modal */}
      <Modal title={editingAction ? tText('quickActions.editAction') : tText('quickActions.newAction')} open={modalOpen}
        onOk={handleSave} onCancel={() => { setModalOpen(false); setEditingAction(null); }}
        destroyOnHidden width={540} okText={editingAction ? tText('common.save') : tText('quickActions.create')} cancelText={tText('common.cancel')}>
        <Form form={form} layout="vertical" style={{ marginTop: 16 }}>
          {/* Row 1: Name + Category */}
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
            <Form.Item name="name" label={tText('quickActions.actionName')} rules={[{ required: true, message: tText('quickActions.enterActionName') }]}>
              <Input placeholder={tText('quickActions.actionNamePlaceholder')} />
            </Form.Item>
            <Form.Item name="category" label={tText('quickActions.category')} rules={[{ required: true, message: tText('scriptLib.selectCategoryRequired') }]}>
              <Select options={CATEGORY_OPTIONS} placeholder={tText('quickActions.selectCategory')} />
            </Form.Item>
          </div>

          {/* Command */}
          <Form.Item name="command" label={tText('quickActions.commandContent')} rules={[{ required: true, message: tText('quickActions.enterCommandContent') }]}
            extra={<span style={{ fontSize: 12, color: 'var(--color-text-muted)' }}>{t('quickActions.paramPlaceholder', { open: '{', close: '}' })}</span>}>
            <Input.TextArea rows={3} placeholder={tText('quickActions.commandPlaceholder')} style={{ fontFamily: 'monospace', fontSize: 13 }} />
          </Form.Item>

          {/* Description */}
          <Form.Item name="description" label={tText('quickActions.description')}>
            <Input placeholder={tText('quickActions.descriptionPlaceholder')} />
          </Form.Item>

          {/* Row 2: Language + Tags */}
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
            <Form.Item name="language" label={tText('quickActions.language')} initialValue="shell">
              <Select options={LANGUAGE_OPTIONS} />
            </Form.Item>
            <Form.Item name="tags" label={tText('quickActions.tags')}>
              <Input placeholder={tText('quickActions.tagsPlaceholder')} />
            </Form.Item>
          </div>
        </Form>
      </Modal>

      {/* Parameter Modal */}
      <Modal
        title={tText('quickActions.fillParams', { name: paramAction?.name || '' })}
        open={paramModalOpen}
        onOk={handleParamSubmit}
        onCancel={() => { setParamModalOpen(false); setParamAction(null); paramForm.resetFields(); }}
        destroyOnHidden
        okText={tText('quickActions.execute')}
        cancelText={tText('common.cancel')}
        width={450}
      >
        <div style={{ marginBottom: 12, color: '#8b949e', fontSize: 13 }}>
          {t('quickActions.commandLabel')} <code style={{ background: 'rgba(139,148,158,0.1)', padding: '2px 6px', borderRadius: 3 }}>{paramAction?.command}</code>
        </div>
        <Form form={paramForm} layout="vertical">
          {parsedParams.map((p) => (
            <Form.Item
              key={p.name}
              name={p.name}
              label={p.name}
              rules={[{ required: true, message: tText('quickActions.enterParam', { name: p.name }) }]}
              initialValue={p.defaultValue}
            >
              <Input placeholder={p.defaultValue ? tText('quickActions.paramDefault', { value: p.defaultValue }) : tText('quickActions.enterParam', { name: p.name })} />
            </Form.Item>
          ))}
        </Form>
      </Modal>
    </div>
  );
}
