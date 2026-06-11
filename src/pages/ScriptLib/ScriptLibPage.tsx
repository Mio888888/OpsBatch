import { useEffect, useMemo, useState } from 'react';
import { Tag, Input, Space, Button, Select, Tooltip, message, Modal, Form } from '../../components/ui';
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
import CodeEditor from '../../components/CodeEditor';
import { logHandledError } from '../../utils/globalLogger';

const LANGUAGES = [
  { value: 'shell', label: 'Shell', icon: '🐚', color: 'blue' },
  { value: 'python', label: 'Python', icon: '🐍', color: 'green' },
  { value: 'powershell', label: 'PowerShell', icon: '⚡', color: 'purple' },
];

const RISK_COLORS: Record<string, string> = { low: 'green', medium: 'orange', high: 'red', critical: 'magenta' };
const RISK_LABEL_KEYS: Record<string, TranslationKey> = { low: 'library.risk.low', medium: 'library.risk.medium', high: 'library.risk.high', critical: 'library.risk.critical' };

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

export default function ScriptLibPage() {
  const { scripts, loadScripts, addScript, updateScript, deleteScript, addQuickAction } = useLibraryStore();
  const { t, tText } = useTranslation();
  const [searchQuery, setSearchQuery] = useState('');
  const [selectedCategory, setSelectedCategory] = useState<string | null>(null);
  const [selectedLang, setSelectedLang] = useState<string | null>(null);
  const [viewScript, setViewScript] = useState<ScriptEntry | null>(null);
  const [editScript, setEditScript] = useState<ScriptEntry | null>(null);
  const [addModalOpen, setAddModalOpen] = useState(false);
  const [versionHistory, setVersionHistory] = useState<VersionRecord[]>([]);
  const [showVersions, setShowVersions] = useState(false);
  const [form] = Form.useForm<ScriptFormValues>();

  useEffect(() => {
    loadScripts();
  }, [loadScripts]);

  const scriptCategories = useMemo(() => {
    const categorySet = new Set<string>();
    for (const script of scripts) {
      if (script.category.trim()) categorySet.add(script.category);
    }
    return Array.from(categorySet).sort();
  }, [scripts]);

  const filtered = scripts.filter((s) => {
    if (selectedCategory && s.category !== selectedCategory) return false;
    if (selectedLang && s.language !== selectedLang) return false;
    if (searchQuery) {
      const q = searchQuery.toLowerCase();
      return s.name.toLowerCase().includes(q) || s.description.toLowerCase().includes(q) || s.tags.some((t) => t.toLowerCase().includes(q));
    }
    return true;
  });

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

  const handleRestoreVersion = (_scriptId: string, version: VersionRecord) => {
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

  return (
    <div className="page-container">
      <div className="page-header">
        <h2>{t('scriptLib.title')}</h2>
        <Button type="primary" icon={<PlusOutlined />} onClick={() => setAddModalOpen(true)}>
          {t('scriptLib.addScript')}
        </Button>
      </div>

      {/* Category Tabs */}
      <div className="qa-tabs">
        <button
          className={`qa-tab${!selectedCategory ? ' qa-tab-active' : ''}`}
          onClick={() => setSelectedCategory(null)}
        >{t('library.category.all')}</button>
        {scriptCategories.map((category) => (
          <button
            key={category}
            className={`qa-tab${selectedCategory === category ? ' qa-tab-active' : ''}`}
            onClick={() => setSelectedCategory(category)}
          >{category}</button>
        ))}
      </div>

      {/* Toolbar */}
      <div className="qa-toolbar">
        <Input placeholder={tText('scriptLib.searchPlaceholder')} prefix={<SearchOutlined />} value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)} allowClear className="qa-search" />
        <Select value={selectedLang} onChange={(value) => setSelectedLang(value || null)}
          className="qa-lang-select" allowClear placeholder={tText('scriptLib.language')} options={LANGUAGES.map(l => ({ value: l.value, label: l.label }))} />
        <span className="qa-count">{t('scriptLib.count', { filtered: filtered.length, total: scripts.length })}</span>
      </div>

      {/* Card Grid */}
      <div className="qa-card-grid">
        {filtered.map((script) => {
          const lang = LANGUAGES.find((x) => x.value === script.language);
          const isHighRisk = script.risk === 'high' || script.risk === 'critical';
          return (
            <div key={script.id} className="qa-card">
              <div className="qa-card-header">
                <div className="qa-card-title-row">
                  <span className="qa-card-lang-icon">{lang?.icon || '🐚'}</span>
                  <span className="qa-card-name">{script.name}</span>
                </div>
                <div className="qa-card-header-badges">
                  {script.url && <Tag color="cyan" className="qa-card-micro-tag">{t('library.remote')}</Tag>}
                  {script.parameters.length > 0 && <Tag color="blue" className="qa-card-micro-tag">{t('library.parameters')}</Tag>}
                  <Tooltip title={tText('library.addToQuickActions')}>
                    <button className="qa-star-btn" onClick={() => handleAddToQuickActions(script)}>
                      <ThunderboltOutlined />
                    </button>
                  </Tooltip>
                </div>
              </div>
              {script.description && (
                <div className="qa-card-desc">{script.description}</div>
              )}
              {script.url ? (
                <code className="qa-card-code qa-card-code-remote">{`curl -sSL '${script.url}' | bash`}</code>
              ) : (
                <code className="qa-card-code">{script.content.length > 120 ? script.content.slice(0, 120) + '…' : script.content}</code>
              )}
              <div className="qa-card-footer">
                <div className="qa-card-status">
                  <span className={`qa-card-risk-dot ${isHighRisk ? 'qa-card-risk-dot-high' : ''}`} />
                  <Tag className="qa-tag">{t(RISK_LABEL_KEYS[script.risk])}</Tag>
                  <Tag className="qa-tag">{script.category}</Tag>
                </div>
                <Space size={2}>
                  <Tooltip title={tText('library.view')}>
                    <Button type="text" size="small" icon={<EyeOutlined />} onClick={() => setViewScript(script)} />
                  </Tooltip>
                  {!script.isBuiltin && (
                    <Tooltip title={tText('common.edit')}>
                      <Button type="text" size="small" icon={<EditOutlined />} onClick={async () => {
                        await saveVersion(script.id, script.content, tText('scriptLib.autoSaveBeforeEdit'));
                        setEditScript({ ...script });
                      }} />
                    </Tooltip>
                  )}
                  {!script.isBuiltin && (
                    <Tooltip title={tText('common.delete')}>
                      <Button type="text" size="small" danger icon={<DeleteOutlined />}
                        onClick={async () => { await deleteScript(script.id); message.success(tText('library.deleted')); }} />
                    </Tooltip>
                  )}
                </Space>
              </div>
            </div>
          );
        })}
        {filtered.length === 0 && (
          <div className="qa-empty">{t('scriptLib.noScripts')}</div>
        )}
      </div>

      {/* 查看脚本详情 */}
      <Modal title={viewScript?.name} open={!!viewScript} onCancel={() => setViewScript(null)}
        footer={<Button onClick={() => setViewScript(null)}>{t('common.close')}</Button>} width={750}>
        {viewScript && (
          <div>
            <Space style={{ marginBottom: 12 }} wrap>
              <Tag color={LANGUAGES.find((l) => l.value === viewScript.language)?.color}>
                {viewScript.language}
              </Tag>
              <Tag color={RISK_COLORS[viewScript.risk]}>{t('library.riskPrefix', { risk: t(RISK_LABEL_KEYS[viewScript.risk]) })}</Tag>
              <Tag>{viewScript.category}</Tag>
              {viewScript.tags.map((t) => <Tag key={t}>{t}</Tag>)}
            </Space>
            <p style={{ marginBottom: 12, color: '#666' }}>{viewScript.description}</p>
            {viewScript.parameters.length > 0 && (
              <div style={{ marginBottom: 12 }}>
                <strong>{t('scriptLib.params')}</strong>
                {viewScript.parameters.map((p) => (
                  <Tag key={p.name}>{p.name}={p.defaultValue} ({p.description})</Tag>
                ))}
              </div>
            )}
            <CodeEditor value={viewScript.content} language={viewScript.language} readOnly height="350px" />
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
              <div style={{ marginBottom: 12, maxHeight: 150, overflow: 'auto', background: '#fafafa', padding: 8, borderRadius: 4 }}>
                {versionHistory.map((v, i) => (
                  <div key={v.id} style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', padding: '4px 0', borderBottom: '1px solid #f0f0f0' }}>
                    <span>
                      <Tag color={i === 0 ? 'blue' : 'default'}>{i === 0 ? t('common.latest') : `v${versionHistory.length - i}`}</Tag>
                      <span style={{ fontSize: 12, color: '#666' }}>{v.label} - {new Date(v.created_at).toLocaleString()}</span>
                    </span>
                    <Button size="small" onClick={() => handleRestoreVersion(editScript.id, v)}>{t('common.restoreVersion')}</Button>
                  </div>
                ))}
              </div>
            )}
            <Space style={{ marginBottom: 8 }} wrap>
              <Input value={editScript.name} placeholder={tText('scriptLib.scriptName')} style={{ width: 200 }}
                onChange={(e) => setEditScript({ ...editScript, name: e.target.value })} />
              <Select value={editScript.language} style={{ width: 120 }}
                options={LANGUAGES.map(l => ({ value: l.value, label: l.label }))} onChange={(v) => setEditScript({ ...editScript, language: v as ScriptEntry['language'] })} />
              <Select value={editScript.risk} style={{ width: 100 }}
                options={RISK_OPTION_KEYS.map(r => ({ value: r.value, label: tText(r.labelKey) }))}
                onChange={(v) => setEditScript({ ...editScript, risk: v as ScriptEntry['risk'] })} />
            </Space>
            <CodeEditor
              value={editScript.content}
              language={editScript.language}
              onChange={(value) => setEditScript({ ...editScript, content: value })}
              height="400px"
              placeholder={tText('scriptLib.scriptCode')}
            />
            <Input.TextArea value={editScript.description} placeholder={tText('scriptLib.scriptDescription')} rows={1}
              style={{ marginTop: 8 }} onChange={(e) => setEditScript({ ...editScript, description: e.target.value })} />
          </div>
        )}
      </Modal>

      {/* 添加脚本 */}
      <Modal title={tText('scriptLib.addCustomScript')} open={addModalOpen} onOk={handleAdd}
        onCancel={() => { setAddModalOpen(false); form.resetFields(); }}
        width={720} destroyOnHidden okText={tText('common.add')} cancelText={tText('common.cancel')}>
        <Form form={form} layout="vertical" style={{ marginTop: 16 }}>
          {/* Row 1: Name + Language */}
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 140px', gap: 12 }}>
            <Form.Item name="name" label={tText('scriptLib.scriptName')} rules={[{ required: true, message: tText('scriptLib.enterName') }]}>
              <Input placeholder={tText('scriptLib.namePlaceholder')} />
            </Form.Item>
            <Form.Item name="language" label={tText('scriptLib.language')} rules={[{ required: true }]} initialValue="shell">
              <Select options={LANGUAGES.map(l => ({ value: l.value, label: l.label }))} />
            </Form.Item>
          </div>

          {/* Row 2: Category + Risk */}
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 140px', gap: 12 }}>
            <Form.Item name="category" label={tText('scriptLib.category')} rules={[{ required: true, message: tText('scriptLib.selectCategoryRequired') }]}>
              <Select options={scriptCategories.map((category) => ({ value: category, label: category }))} placeholder={tText('scriptLib.selectCategory')} />
            </Form.Item>
            <Form.Item name="risk" label={tText('scriptLib.riskLevel')} initialValue="low">
              <Select options={RISK_OPTION_KEYS.map((r) => ({ value: r.value, label: tText(r.labelKey) }))} />
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
            <Form.Item noStyle shouldUpdate={(prev, cur) => prev.language !== cur.language}>
              {({ getFieldValue }) => {
                const lang = LANGUAGES.find(l => l.value === getFieldValue('language'));
                return lang ? (
                  <Tag color={lang.color} style={{ fontSize: 11, lineHeight: '18px' }}>{lang.icon} {lang.label}</Tag>
                ) : null;
              }}
            </Form.Item>
          </div>
          <Form.Item noStyle shouldUpdate={(prev, cur) => prev.language !== cur.language}>
            {({ getFieldValue, setFieldValue }) => {
              const lang = (getFieldValue('language') as ScriptEntry['language'] | undefined) || 'shell';
              return (
                <div style={{ border: '1px solid var(--color-border)', borderRadius: 8, overflow: 'hidden' }}>
                  <CodeEditor
                    value={(getFieldValue('content') as string | undefined) || ''}
                    language={lang}
                    onChange={(value) => setFieldValue('content', value)}
                    height="280px"
                    placeholder={tText('scriptLib.codePlaceholder')}
                  />
                </div>
              );
            }}
          </Form.Item>
        </Form>
      </Modal>
    </div>
  );
}
