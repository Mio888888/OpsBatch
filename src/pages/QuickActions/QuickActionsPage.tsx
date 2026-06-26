import { useEffect, useState, useMemo } from 'react';
import {
  Input, Button, Select, Tooltip, message,
  Modal, Form, Tag,
} from '../../components/ui';
import {
  SearchOutlined, PlusOutlined,
  PlayCircleOutlined, EditOutlined, DeleteOutlined,
  ImportOutlined, ExportOutlined,
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
  /** 嵌入设置面板时使用，隐藏外层标题并去掉卡片边框 */
  embedded?: boolean;
}

const LANGUAGE_OPTIONS = [
  { value: 'shell', label: 'Shell' },
  { value: 'python', label: 'Python' },
  { value: 'powershell', label: 'PowerShell' },
] as const;

const LANG_LABEL: Record<string, string> = {
  shell: 'Shell',
  python: 'Python',
  powershell: 'PS',
};

const STATUS_KEY_MAP: Record<string, { color: string; textKey: TranslationKey }> = {
  success: { color: '#52c41a', textKey: 'quickActions.status.success' },
  failed: { color: '#ff4d4f', textKey: 'quickActions.status.failed' },
  partial: { color: '#faad14', textKey: 'quickActions.status.partial' },
};

// 预设分类：保证空库首次创建时也能选中分类（Select 不支持自由输入）
const PRESET_CATEGORIES = ['系统', '磁盘', '内存', '网络', '进程', '安全', '其他'];

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
  const { quickActions, loadQuickActions, addQuickAction, updateQuickAction, deleteQuickAction } = useLibraryStore();
  const executeCommand = useExecutionStore((s) => s.executeCommand);
  const { t, tText } = useTranslation();
  const [searchQuery, setSearchQuery] = useState('');
  const [selectedCategory, setSelectedCategory] = useState<string | null>(null);
  const [modalOpen, setModalOpen] = useState(false);
  const [editingAction, setEditingAction] = useState<QuickAction | null>(null);
  const [form] = Form.useForm<QuickActionFormValues>();

  // 参数填写弹窗
  const [paramModalOpen, setParamModalOpen] = useState(false);
  const [paramAction, setParamAction] = useState<QuickAction | null>(null);
  const [paramForm] = Form.useForm<Record<string, string>>();

  useEffect(() => {
    loadQuickActions();
  }, [loadQuickActions]);

  // 按类目聚合计数，供左侧导航显示
  const categoryCounts = useMemo(() => {
    const map = new Map<string, number>();
    for (const action of quickActions) {
      const category = action.category.trim();
      if (category) map.set(category, (map.get(category) ?? 0) + 1);
    }
    return map;
  }, [quickActions]);

  const categories = useMemo(() => Array.from(categoryCounts.keys()).sort(), [categoryCounts]);

  const filtered = useMemo(() => {
    const query = searchQuery.trim().toLowerCase();
    return quickActions.filter((a) => {
      if (selectedCategory && a.category !== selectedCategory) return false;
      if (query) {
        return (
          a.name.toLowerCase().includes(query) ||
          a.command.toLowerCase().includes(query) ||
          (a.description || '').toLowerCase().includes(query)
        );
      }
      return true;
    });
  }, [quickActions, selectedCategory, searchQuery]);

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
          tags: values.tags ? values.tags.split(',').map((tg: string) => tg.trim()).filter(Boolean) : [],
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
          tags: values.tags ? values.tags.split(',').map((tg: string) => tg.trim()).filter(Boolean) : [],
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

  // 下拉选项：预设分类在前，再补充已存在但不在预设内的分类
  const categoryOptions = useMemo(() => {
    const merged = [...PRESET_CATEGORIES];
    for (const category of categories) {
      if (!merged.includes(category)) merged.push(category);
    }
    return merged.map((category) => ({ value: category, label: category }));
  }, [categories]);

  return (
    <div className={`qa${embedded ? ' qa-embedded' : ''}`}>
      {/* 顶部：标题 / 搜索 / 语言 / 收藏筛选 / 计数 / 操作 */}
      <div className="qa-topbar">
        {!embedded && <h2 className="qa-title">{t('quickActions.title')}</h2>}
        <div className="qa-search">
          <Input
            placeholder={tText('quickActions.searchPlaceholder')}
            prefix={<SearchOutlined />}
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            allowClear
          />
        </div>
        <span className="qa-count">
          {t('quickActions.count', { filtered: filtered.length, total: quickActions.length })}
        </span>
        <span className="qa-topbar-divider" aria-hidden="true" />
        <Tooltip title={tText('quickActions.import')}>
          <Button
            icon={<ImportOutlined />}
            onClick={handleImport}
            className="qa-topbar-btn"
          >
            {t('quickActions.import')}
          </Button>
        </Tooltip>
        <Tooltip title={quickActions.length === 0 ? tText('quickActions.exportDisabled') : tText('quickActions.export')}>
          <Button
            icon={<ExportOutlined />}
            onClick={handleExport}
            disabled={quickActions.length === 0}
            className="qa-topbar-btn"
          >
            {t('quickActions.export')}
          </Button>
        </Tooltip>
        <Button type="primary" icon={<PlusOutlined />} onClick={openCreate}>
          {t('quickActions.createAction')}
        </Button>
      </div>

      {/* 已选中主机提示 */}
      {selectedHostIds.length > 0 && (
        <div className="qa-selected-hint">
          {t('quickActions.selectedHostHint', { count: selectedHostIds.length })}
        </div>
      )}

      {/* 主体：左侧类目栏 + 右侧指令列表 */}
      <div className="qa-body">
        <aside className="qa-categories" aria-label={tText('quickActions.category')}>
          <button
            type="button"
            className={`qa-cat${!selectedCategory ? ' qa-cat-active' : ''}`}
            onClick={() => setSelectedCategory(null)}
          >
            <span className="qa-cat-name">{t('library.category.all')}</span>
            <span className="qa-cat-count">{quickActions.length}</span>
          </button>
          {categories.map((cat) => (
            <button
              key={cat}
              type="button"
              className={`qa-cat${selectedCategory === cat ? ' qa-cat-active' : ''}`}
              onClick={() => setSelectedCategory(cat)}
              title={cat}
            >
              <span className="qa-cat-name">{cat}</span>
              <span className="qa-cat-count">{categoryCounts.get(cat) ?? 0}</span>
            </button>
          ))}
        </aside>

        <div className="qa-list">
          {filtered.length === 0 ? (
            <div className="qa-empty">{t('quickActions.noMatch')}</div>
          ) : (
            filtered.map((action) => {
              const params = parseQuickActionParams(action.command);
              const statusInfo = STATUS_KEY_MAP[action.lastStatus];
              const lang = action.language || 'shell';
              const disabled = selectedHostIds.length === 0;
              return (
                <div key={action.id} className="qa-row" title={action.description || action.name}>
                  <div className="qa-row-main">
                    <div className="qa-row-head">
                      <span className="qa-row-lang" data-lang={lang}>{LANG_LABEL[lang]}</span>
                      <span className="qa-row-name">{action.name}</span>
                      {params.length > 0 && (
                        <Tag className="qa-row-tag">{t('library.parameters')}</Tag>
                      )}
                    </div>
                    <code className="qa-row-code" title={action.command}>{action.command}</code>
                  </div>

                  <div className="qa-row-meta">
                    {statusInfo && (
                      <span
                        className={`qa-status ${action.lastStatus ? `qa-status-${action.lastStatus}` : ''}`}
                        style={{ color: statusInfo.color }}
                      >
                        <span className="qa-status-dot" style={{ background: statusInfo.color }} />
                        {t(statusInfo.textKey)}
                      </span>
                    )}
                    {action.lastRunAt && (
                      <span className="qa-row-last-run">{formatLastRun(action.lastRunAt, tText)}</span>
                    )}
                    <span className="qa-row-category">{action.category}</span>
                  </div>

                  <div className="qa-row-actions">
                    <Tooltip title={tText('quickActions.execute')}>
                      <button
                        type="button"
                        className="qa-act qa-act-execute"
                        onClick={() => handleExecuteAction(action)}
                        disabled={disabled}
                        aria-label={tText('quickActions.execute')}
                      >
                        <PlayCircleOutlined />
                      </button>
                    </Tooltip>
                    <Tooltip title={tText('common.edit')}>
                      <button
                        type="button"
                        className="qa-act"
                        onClick={() => openEdit(action)}
                        aria-label={tText('common.edit')}
                      >
                        <EditOutlined />
                      </button>
                    </Tooltip>
                    <Tooltip title={tText('common.delete')}>
                      <button
                        type="button"
                        className="qa-act qa-act-danger"
                        onClick={async () => {
                          await deleteQuickAction(action.id);
                          message.success(tText('library.deleted'));
                        }}
                        aria-label={tText('common.delete')}
                      >
                        <DeleteOutlined />
                      </button>
                    </Tooltip>
                  </div>
                </div>
              );
            })
          )}
        </div>
      </div>

      {/* 新建 / 编辑弹窗 */}
      <Modal
        title={editingAction ? tText('quickActions.editAction') : tText('quickActions.newAction')}
        open={modalOpen}
        onOk={handleSave}
        onCancel={() => { setModalOpen(false); setEditingAction(null); }}
        destroyOnHidden
        width={540}
        okText={editingAction ? tText('common.save') : tText('quickActions.create')}
        cancelText={tText('common.cancel')}
      >
        <Form form={form} layout="vertical" style={{ marginTop: 16 }}>
          {/* Row 1: 名称 + 分类 */}
          <div className="qa-form-row">
            <Form.Item name="name" label={tText('quickActions.actionName')} rules={[{ required: true, message: tText('quickActions.enterActionName') }]}>
              <Input placeholder={tText('quickActions.actionNamePlaceholder')} />
            </Form.Item>
            <Form.Item name="category" label={tText('quickActions.category')} rules={[{ required: true, message: tText('scriptLib.selectCategoryRequired') }]}>
              <Select options={categoryOptions} placeholder={tText('quickActions.selectCategory')} />
            </Form.Item>
          </div>

          {/* 命令内容 */}
          <Form.Item
            name="command"
            label={tText('quickActions.commandContent')}
            rules={[{ required: true, message: tText('quickActions.enterCommandContent') }]}
            extra={<span style={{ fontSize: 12, color: 'var(--color-text-muted)' }}>{t('quickActions.paramPlaceholder', { open: '{', close: '}' })}</span>}
          >
            <Input.TextArea rows={3} placeholder={tText('quickActions.commandPlaceholder')} style={{ fontFamily: 'monospace', fontSize: 13 }} />
          </Form.Item>

          {/* 描述 */}
          <Form.Item name="description" label={tText('quickActions.description')}>
            <Input placeholder={tText('quickActions.descriptionPlaceholder')} />
          </Form.Item>

          {/* Row 2: 语言 + 标签 */}
          <div className="qa-form-row">
            <Form.Item name="language" label={tText('quickActions.language')} initialValue="shell">
              <Select options={LANGUAGE_OPTIONS.map((o) => ({ value: o.value, label: o.label }))} />
            </Form.Item>
            <Form.Item name="tags" label={tText('quickActions.tags')}>
              <Input placeholder={tText('quickActions.tagsPlaceholder')} />
            </Form.Item>
          </div>
        </Form>
      </Modal>

      {/* 参数填写弹窗 */}
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
        <div style={{ marginBottom: 12, color: 'var(--color-text-muted)', fontSize: 13 }}>
          {t('quickActions.commandLabel')} <code style={{ background: 'color-mix(in srgb, var(--color-text-muted) 10%, transparent)', padding: '2px 6px', borderRadius: 3 }}>{paramAction?.command}</code>
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
