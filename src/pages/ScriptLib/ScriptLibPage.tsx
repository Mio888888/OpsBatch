import { lazy, Suspense, useEffect, useMemo, useState } from 'react';
import { Input, Button, Select, Tooltip, message, Modal, Form, Tag, Space } from '../../components/ui';
import {
  SearchOutlined, PlusOutlined,
  EyeOutlined, EditOutlined, HistoryOutlined, DeleteOutlined,
  ThunderboltOutlined,
} from '../../components/ui/icons';
import { useLibraryStore } from '../../stores/library';
import { useTranslation } from '../../i18n';
import type { TranslationKey } from '../../i18n';
import type { ScriptEntry } from '../../types';
import { invoke } from '@tauri-apps/api/core';
import { logHandledError } from '../../utils/globalLogger';
import '../../styles/pages/script-lib.css';

const LazyCodeEditor = lazy(() => import('../../components/CodeEditor'));

// 语言 → 徽章配色与显示名
const LANGUAGES: { value: ScriptEntry['language']; label: string }[] = [
  { value: 'shell', label: 'Shell' },
  { value: 'python', label: 'Python' },
  { value: 'powershell', label: 'PowerShell' },
];

const LANG_LABEL: Record<string, string> = {
  shell: 'Shell',
  python: 'Python',
  powershell: 'PS',
};

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

interface VersionRecord {
  id: string;
  content: string;
  label: string;
  created_at: string;
}

interface ScriptFormValues extends Omit<ScriptEntry, 'id' | 'parameters' | 'starred' | 'isBuiltin'> {}

function CodeEditorFallback({ height = '280px' }: { height?: string }) {
  return <div style={{ height, background: 'var(--color-bg-elevated)', borderRadius: 8 }} />;
}

interface ScriptLibPageProps {
  /** 嵌入设置面板时使用，隐藏外层标题并去掉卡片边框 */
  embedded?: boolean;
}

export default function ScriptLibPage({ embedded = false }: ScriptLibPageProps) {
  const { scripts, loadScripts, addScript, updateScript, deleteScript, addQuickAction } = useLibraryStore();
  const { t, tText } = useTranslation();
  const [searchQuery, setSearchQuery] = useState('');
  const [selectedCategory, setSelectedCategory] = useState<string | null>(null);
  const [viewScript, setViewScript] = useState<ScriptEntry | null>(null);
  const [editScript, setEditScript] = useState<ScriptEntry | null>(null);
  const [addModalOpen, setAddModalOpen] = useState(false);
  const [versionHistory, setVersionHistory] = useState<VersionRecord[]>([]);
  const [showVersions, setShowVersions] = useState(false);
  const [form] = Form.useForm<ScriptFormValues>();

  useEffect(() => {
    loadScripts();
  }, [loadScripts]);

  // 按类目聚合计数，供左侧导航显示
  const categoryCounts = useMemo(() => {
    const map = new Map<string, number>();
    for (const script of scripts) {
      const category = script.category.trim();
      if (category) map.set(category, (map.get(category) ?? 0) + 1);
    }
    return map;
  }, [scripts]);

  const categories = useMemo(() => Array.from(categoryCounts.keys()).sort(), [categoryCounts]);

  const filtered = useMemo(() => {
    const query = searchQuery.trim().toLowerCase();
    return scripts.filter((s) => {
      if (selectedCategory && s.category !== selectedCategory) return false;
      if (query) {
        return (
          s.name.toLowerCase().includes(query) ||
          s.description.toLowerCase().includes(query) ||
          s.tags.some((tag) => tag.toLowerCase().includes(query))
        );
      }
      return true;
    });
  }, [scripts, selectedCategory, searchQuery]);

  const loadVersionHistory = async (scriptId: string) => {
    try {
      const versions = await invoke<VersionRecord[]>('list_script_versions', { scriptId });
      setVersionHistory(versions);
    } catch {
      setVersionHistory([]);
    }
  };

  const saveVersion = async (scriptId: string, content: string, label: string) => {
    try {
      await invoke('save_script_version', { scriptId, content, label });
    } catch (e) {
      console.error('Failed to save script version:', e);
    }
  };

  const handleEditSave = async () => {
    if (!editScript) return;
    const original = scripts.find((s) => s.id === editScript.id);
    if (original) {
      await saveVersion(editScript.id, original.content, tText('scriptLib.autoSaveBeforeEdit'));
    }
    await updateScript(editScript);
    message.success(tText('library.savedWithVersion'));
    setEditScript(null);
  };

  const handleRestoreVersion = (version: VersionRecord) => {
    const updated = { ...editScript!, content: version.content } as ScriptEntry;
    setEditScript(updated);
    message.success(tText('library.restoredVersion', { label: version.label }));
    setShowVersions(false);
  };

  const handleAddToQuickActions = async (script: ScriptEntry) => {
    await addQuickAction({
      name: script.name,
      command: script.content.split('\n')[0] || script.content,
      category: script.category,
      parameters: [],
      description: script.description || '',
      tags: [],
      language: script.language,
    });
    message.success(tText('library.addedToQuickActions'));
  };

  const handleAdd = async () => {
    try {
      const values = await form.validateFields();
      await addScript({
        ...values,
        platform: values.platform || 'linux',
        tags: values.tags || [],
        parameters: [],
        starred: false,
      });
      message.success(tText('scriptLib.added'));
      setAddModalOpen(false);
      form.resetFields();
    } catch (error) {
      void logHandledError('scriptLib.add', error, 'warn');
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

  const renderScriptText = (script: ScriptEntry) =>
    script.url ? `curl -sSL '${script.url}' | bash` : script.content;

  return (
    <div className={`sl${embedded ? ' sl-embedded' : ''}`}>
      {/* 顶部：标题 / 搜索 / 语言 / 操作 */}
      <div className="sl-topbar">
        {!embedded && <h2 className="sl-title">{t('scriptLib.title')}</h2>}
        <div className="sl-search">
          <Input
            placeholder={tText('scriptLib.searchPlaceholder')}
            prefix={<SearchOutlined />}
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            allowClear
          />
        </div>
        <span className="sl-count">
          {t('scriptLib.count', { filtered: filtered.length, total: scripts.length })}
        </span>
        <Button type="primary" icon={<PlusOutlined />} onClick={() => setAddModalOpen(true)}>
          {t('scriptLib.addScript')}
        </Button>
      </div>

      {/* 主体：左侧类目栏 + 右侧脚本列表 */}
      <div className="sl-body">
        <aside className="sl-categories" aria-label={tText('scriptLib.category')}>
          <button
            type="button"
            className={`sl-cat${!selectedCategory ? ' sl-cat-active' : ''}`}
            onClick={() => setSelectedCategory(null)}
          >
            <span className="sl-cat-name">{t('library.category.all')}</span>
            <span className="sl-cat-count">{scripts.length}</span>
          </button>
          {categories.map((cat) => (
            <button
              key={cat}
              type="button"
              className={`sl-cat${selectedCategory === cat ? ' sl-cat-active' : ''}`}
              onClick={() => setSelectedCategory(cat)}
              title={cat}
            >
              <span className="sl-cat-name">{cat}</span>
              <span className="sl-cat-count">{categoryCounts.get(cat) ?? 0}</span>
            </button>
          ))}
        </aside>

        <div className="sl-list">
          {filtered.length === 0 ? (
            <div className="sl-empty">{t('scriptLib.noScripts')}</div>
          ) : (
            filtered.map((script) => {
              const isHighRisk = script.risk === 'high' || script.risk === 'critical';
              return (
                <div key={script.id} className="sl-row" title={script.description || script.name}>
                  <div className="sl-row-main">
                    <div className="sl-row-head">
                      <span className="sl-row-lang" data-lang={script.language}>
                        {LANG_LABEL[script.language]}
                      </span>
                      <span className="sl-row-name">{script.name}</span>
                      {script.url && (
                        <Tag className="sl-row-tag sl-row-tag-remote">{t('library.remote')}</Tag>
                      )}
                      {script.parameters.length > 0 && (
                        <Tag className="sl-row-tag">{t('library.parameters')}</Tag>
                      )}
                    </div>
                    <code className={`sl-row-code${script.url ? ' sl-row-code-remote' : ''}`} title={renderScriptText(script)}>
                      {renderScriptText(script).length > 120
                        ? renderScriptText(script).slice(0, 120) + '…'
                        : renderScriptText(script)}
                    </code>
                  </div>

                  <div className="sl-row-meta">
                    <span className={`sl-risk ${isHighRisk ? 'sl-risk-high' : `sl-risk-${script.risk}`}`}>
                      <span className="sl-risk-dot" />
                      {t(RISK_LABEL_KEYS[script.risk])}
                    </span>
                    <span className="sl-row-category">{script.category}</span>
                  </div>

                  <div className="sl-row-actions">
                    <Tooltip title={tText('library.view')}>
                      <button
                        type="button"
                        className="sl-act"
                        onClick={() => setViewScript(script)}
                        aria-label={tText('library.view')}
                      >
                        <EyeOutlined />
                      </button>
                    </Tooltip>
                    <Tooltip title={tText('library.addToQuickActions')}>
                      <button
                        type="button"
                        className="sl-act"
                        onClick={() => handleAddToQuickActions(script)}
                        aria-label={tText('library.addToQuickActions')}
                      >
                        <ThunderboltOutlined />
                      </button>
                    </Tooltip>
                    {!script.isBuiltin && (
                      <Tooltip title={tText('common.edit')}>
                        <button
                          type="button"
                          className="sl-act"
                          onClick={async () => {
                            await saveVersion(script.id, script.content, tText('scriptLib.autoSaveBeforeEdit'));
                            setEditScript({ ...script });
                          }}
                          aria-label={tText('common.edit')}
                        >
                          <EditOutlined />
                        </button>
                      </Tooltip>
                    )}
                    {!script.isBuiltin && (
                      <Tooltip title={tText('common.delete')}>
                        <button
                          type="button"
                          className="sl-act sl-act-danger"
                          onClick={async () => {
                            await deleteScript(script.id);
                            message.success(tText('library.deleted'));
                          }}
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

      {/* 查看脚本详情 */}
      <Modal
        title={viewScript?.name}
        open={!!viewScript}
        onCancel={() => setViewScript(null)}
        footer={<Button onClick={() => setViewScript(null)}>{t('common.close')}</Button>}
        width={750}
      >
        {viewScript && (
          <div>
            <Space style={{ marginBottom: 12 }} wrap>
              <Tag color="blue">{viewScript.language}</Tag>
              <Tag>{t('library.riskPrefix', { risk: t(RISK_LABEL_KEYS[viewScript.risk]) })}</Tag>
              <Tag>{viewScript.category}</Tag>
              {viewScript.tags.map((tag) => <Tag key={tag}>{tag}</Tag>)}
            </Space>
            <p style={{ marginBottom: 12, color: 'var(--color-text-muted)' }}>{viewScript.description}</p>
            {viewScript.parameters.length > 0 && (
              <div style={{ marginBottom: 12 }}>
                <strong>{t('scriptLib.params')}</strong>
                {viewScript.parameters.map((p) => (
                  <Tag key={p.name}>{p.name}={p.defaultValue} ({p.description})</Tag>
                ))}
              </div>
            )}
            <Suspense fallback={<CodeEditorFallback height="350px" />}>
              <LazyCodeEditor value={viewScript.content} language={viewScript.language} readOnly height="350px" />
            </Suspense>
          </div>
        )}
      </Modal>

      {/* 在线编辑器 */}
      <Modal
        title={tText('scriptLib.editTitle', { name: editScript?.name || '' })}
        open={!!editScript && !viewScript}
        onCancel={() => { setEditScript(null); setShowVersions(false); }}
        width={800}
        footer={
          <Space>
            <Button icon={<HistoryOutlined />} onClick={async () => {
              if (editScript) {
                await loadVersionHistory(editScript.id);
              }
              setShowVersions(!showVersions);
            }}>
              {showVersions ? t('common.hideVersions') : t('common.versionHistory')} ({versionHistory.length})
            </Button>
            <Button onClick={() => { setEditScript(null); setShowVersions(false); }}>{t('common.cancel')}</Button>
            <Button type="primary" onClick={handleEditSave}>{t('common.save')}</Button>
          </Space>
        }
      >
        {editScript && (
          <div>
            {showVersions && versionHistory.length > 0 && (
              <div className="sl-versions">
                {versionHistory.map((v, i) => (
                  <div key={v.id} className="sl-version-item">
                    <span>
                      <Tag color={i === 0 ? 'blue' : 'default'}>
                        {i === 0 ? t('common.latest') : `v${versionHistory.length - i}`}
                      </Tag>
                      <span className="sl-version-meta">{v.label} - {new Date(v.created_at).toLocaleString()}</span>
                    </span>
                    <Button size="small" onClick={() => handleRestoreVersion(v)}>{t('common.restoreVersion')}</Button>
                  </div>
                ))}
              </div>
            )}
            <Space style={{ marginBottom: 8 }} wrap>
              <Input value={editScript.name} placeholder={tText('scriptLib.scriptName')} style={{ width: 200 }}
                onChange={(e) => setEditScript({ ...editScript, name: e.target.value })} />
              <Select value={editScript.language} style={{ width: 120 }}
                options={LANGUAGES.map((l) => ({ value: l.value, label: l.label }))}
                onChange={(v) => setEditScript({ ...editScript, language: v as ScriptEntry['language'] })} />
              <Select value={editScript.risk} style={{ width: 100 }}
                options={RISK_OPTION_KEYS.map((r) => ({ value: r.value, label: tText(r.labelKey) }))}
                onChange={(v) => setEditScript({ ...editScript, risk: v as ScriptEntry['risk'] })} />
            </Space>
            <Suspense fallback={<CodeEditorFallback height="400px" />}>
              <LazyCodeEditor
                value={editScript.content}
                language={editScript.language}
                onChange={(value) => setEditScript({ ...editScript, content: value })}
                height="400px"
                placeholder={tText('scriptLib.scriptCode')}
              />
            </Suspense>
            <Input.TextArea value={editScript.description} placeholder={tText('scriptLib.scriptDescription')} rows={1}
              style={{ marginTop: 8 }} onChange={(e) => setEditScript({ ...editScript, description: e.target.value })} />
          </div>
        )}
      </Modal>

      {/* 添加脚本 */}
      <Modal
        title={tText('scriptLib.addCustomScript')}
        open={addModalOpen}
        onOk={handleAdd}
        onCancel={() => { setAddModalOpen(false); form.resetFields(); }}
        width={720}
        destroyOnHidden
        okText={tText('common.add')}
        cancelText={tText('common.cancel')}
      >
        <Form form={form} layout="vertical" style={{ marginTop: 16 }}>
          {/* Row 1: Name + Language */}
          <div className="sl-form-row">
            <Form.Item name="name" label={tText('scriptLib.scriptName')} rules={[{ required: true, message: tText('scriptLib.enterName') }]}>
              <Input placeholder={tText('scriptLib.namePlaceholder')} />
            </Form.Item>
            <Form.Item name="language" label={tText('scriptLib.language')} rules={[{ required: true }]} initialValue="shell">
              <Select options={LANGUAGES.map((l) => ({ value: l.value, label: l.label }))} />
            </Form.Item>
          </div>

          {/* Row 2: Category + Risk */}
          <div className="sl-form-row">
            <Form.Item name="category" label={tText('scriptLib.category')} rules={[{ required: true, message: tText('scriptLib.selectCategoryRequired') }]}>
              <Select options={categoryOptions} placeholder={tText('scriptLib.selectCategory')} />
            </Form.Item>
            <Form.Item name="risk" label={tText('scriptLib.riskLevel')} initialValue="low">
              <Select options={riskOptions} />
            </Form.Item>
          </div>

          {/* Description */}
          <Form.Item name="description" label={tText('scriptLib.description')}>
            <Input placeholder={tText('scriptLib.descriptionPlaceholder')} />
          </Form.Item>

          {/* Hidden content field for validation */}
          <Form.Item name="content" rules={[{ required: true, message: tText('scriptLib.writeCode') }]}>
            <Input.TextArea rows={1} hidden />
          </Form.Item>

          {/* Code Editor */}
          <div style={{ marginBottom: 6, display: 'flex', alignItems: 'center', gap: 8 }}>
            <span style={{ fontWeight: 500, fontSize: 13, color: 'var(--color-text)' }}>{t('scriptLib.scriptCode')}</span>
          </div>
          <Form.Item noStyle shouldUpdate={(prev, cur) => prev.language !== cur.language}>
            {({ getFieldValue, setFieldValue }) => {
              const lang = (getFieldValue('language') as ScriptEntry['language'] | undefined) || 'shell';
              return (
                <div style={{ border: '1px solid var(--color-border)', borderRadius: 8, overflow: 'hidden' }}>
                  <Suspense fallback={<CodeEditorFallback height="280px" />}>
                    <LazyCodeEditor
                      value={(getFieldValue('content') as string | undefined) || ''}
                      language={lang}
                      onChange={(value) => setFieldValue('content', value)}
                      height="280px"
                      placeholder={tText('scriptLib.codePlaceholder')}
                    />
                  </Suspense>
                </div>
              );
            }}
          </Form.Item>
        </Form>
      </Modal>
    </div>
  );
}
