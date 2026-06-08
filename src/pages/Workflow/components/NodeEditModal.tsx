import { useState, useEffect, useRef, useCallback } from 'react';
import { Modal, Input, Select, Tag } from '../../../components/ui';
import { CloudServerFilled } from '../../../components/ui/icons';
import { useAssetsStore } from '../../../stores/assets';
import { NODE_TYPES } from './nodeTypes';
import { useTranslation } from '../../../i18n';

export interface NodeEditData {
  id: string;
  name: string;
  type: string;
  config: string;
}

interface Props {
  open: boolean;
  nodeData: NodeEditData | null;
  onSave: (id: string, updates: { name: string; type: string; config: string }) => void;
  onClose: () => void;
}

export function useNodeTypeOptions() {
  const { tText } = useTranslation();
  return NODE_TYPES.map((t) => ({ value: t.value, label: tText(t.labelKey) }));
}

interface TransferConfig {
  localPath: string;
  remotePath: string;
  direction: 'upload' | 'download';
}

interface SelectHostConfig {
  hostIds: string[];
}

interface SwitchCase {
  label: string;
  value: string;
}

interface SwitchConfig {
  expression: string;
  cases: SwitchCase[];
}

// --- Snippet definitions ---
function getSnippets(nodeType: string, tText: (key: string) => string): { label: string; value: string }[] {
  const chips = [
    { label: tText('nodeEdit.snippetOutput'), value: '{{lastResult.output}}' },
    { label: tText('nodeEdit.snippetExitCode'), value: '{{lastResult.exitCode}}' },
    { label: tText('nodeEdit.snippetSuccess'), value: '{{lastResult.success}}' },
  ];

  if (nodeType === 'command' || nodeType === 'script' || nodeType === 'quickAction' || nodeType === 'rollback' || nodeType === 'transfer') {
    chips.push(
      { label: 'IP', value: '{{host.ip}}' },
      { label: tText('nodeEdit.snippetHostName'), value: '{{host.name}}' },
    );
  }

  if (nodeType === 'condition' || nodeType === 'switch') {
    chips.push(
      { label: tText('nodeEdit.snippetSucceeded'), value: '{{lastResult.exitCode}} === 0' },
      { label: tText('nodeEdit.snippetFailed'), value: '{{lastResult.exitCode}} !== 0' },
      { label: tText('nodeEdit.snippetContains'), value: '{{lastResult.output}}.includes("")' },
    );
  }

  return chips;
}

function SnippetChips({ nodeType, onSelect }: { nodeType: string; onSelect: (v: string) => void }) {
  const { tText } = useTranslation();
  const chips = getSnippets(nodeType, tText as (key: string) => string);
  if (chips.length === 0) return null;

  return (
    <div className="wf-snippet-chips">
      <span className="wf-snippet-prefix">{'{ }'}</span>
      {chips.map((item) => (
        <button
          key={item.value}
          type="button"
          className="wf-snippet-chip"
          onClick={() => onSelect(item.value)}
          title={item.value}
        >
          {item.label}
        </button>
      ))}
    </div>
  );
}

// --- Input wrapper with snippet chips ---
function SnippetInput({
  nodeType,
  value,
  onChange,
  placeholder,
  style,
  rows,
  monospace,
}: {
  nodeType: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  style?: React.CSSProperties;
  rows?: number;
  monospace?: boolean;
}) {
  const wrapRef = useRef<HTMLDivElement>(null);

  const handleSnippet = useCallback(
    (text: string) => {
      const el = wrapRef.current?.querySelector(rows ? 'textarea' : 'input') as HTMLInputElement | HTMLTextAreaElement | null;
      if (!el) {
        onChange(value + text);
        return;
      }
      const start = el.selectionStart ?? value.length;
      const end = el.selectionEnd ?? value.length;
      const next = value.slice(0, start) + text + value.slice(end);
      onChange(next);
      requestAnimationFrame(() => {
        const pos = start + text.length;
        el.setSelectionRange(pos, pos);
        el.focus();
      });
    },
    [value, onChange, rows],
  );

  const inputStyle: React.CSSProperties = {
    ...style,
    ...(monospace ? { fontFamily: 'monospace', fontSize: 12 } : {}),
  };

  return (
    <div className="wf-snippet-input-wrap" ref={wrapRef}>
      {rows ? (
        <Input.TextArea
          rows={rows}
          value={value}
          placeholder={placeholder}
          onChange={(e) => onChange(e.target.value)}
          style={{ ...inputStyle, resize: 'vertical' }}
        />
      ) : (
        <Input
          value={value}
          placeholder={placeholder}
          onChange={(e) => onChange(e.target.value)}
          style={inputStyle}
        />
      )}
      <SnippetChips nodeType={nodeType} onSelect={handleSnippet} />
    </div>
  );
}

function parseConfig<T>(raw: string, fallback: T): T {
  if (!raw) return fallback;
  try { return JSON.parse(raw); } catch { return fallback; }
}

export function ConfigForm({ type, config, onChange }: {
  type: string;
  config: string;
  onChange: (v: string) => void;
}) {
  const { tText } = useTranslation();

  if (type === 'start' || type === 'end') {
    return <div style={{ fontSize: 12, color: 'var(--color-text-muted)' }}>{tText('nodeEdit.noConfig')}</div>;
  }

  if (type === 'command' || type === 'quickAction') {
    return (
      <SnippetInput
        nodeType={type}
        value={config}
        onChange={onChange}
        placeholder={tText('nodeEdit.commandPlaceholder')}
        rows={3}
        monospace
      />
    );
  }

  if (type === 'script') {
    return (
      <SnippetInput
        nodeType={type}
        value={config}
        onChange={onChange}
        placeholder={tText('nodeEdit.scriptPlaceholder')}
        rows={5}
        monospace
      />
    );
  }

  if (type === 'transfer') {
    const cfg = parseConfig<TransferConfig>(config, { localPath: '', remotePath: '', direction: 'upload' });
    const update = (partial: Partial<TransferConfig>) => {
      onChange(JSON.stringify({ ...cfg, ...partial }));
    };
    return (
      <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
        <Select
          value={cfg.direction}
          onChange={(v) => update({ direction: v as 'upload' | 'download' })}
          options={[
            { value: 'upload', label: tText('nodeEdit.uploadLabel') },
            { value: 'download', label: tText('nodeEdit.downloadLabel') },
          ]}
          style={{ width: '100%' }}
        />
        <SnippetInput
          nodeType="transfer"
          value={cfg.localPath}
          placeholder={tText('nodeEdit.localFilePath')}
          onChange={(v) => update({ localPath: v })}
        />
        <SnippetInput
          nodeType="transfer"
          value={cfg.remotePath}
          placeholder={tText('nodeEdit.remoteFilePath')}
          onChange={(v) => update({ remotePath: v })}
        />
      </div>
    );
  }

  if (type === 'delay') {
    return (
      <SnippetInput
        nodeType="delay"
        value={config}
        onChange={onChange}
        placeholder={tText('nodeEdit.delaySeconds')}
      />
    );
  }

  if (type === 'condition') {
    return (
      <SnippetInput
        nodeType={type}
        value={config}
        onChange={onChange}
        placeholder={tText('nodeEdit.conditionPlaceholder')}
        monospace
      />
    );
  }

  if (type === 'confirm') {
    return (
      <SnippetInput
        nodeType="confirm"
        value={config}
        onChange={onChange}
        placeholder={tText('nodeEdit.confirmPlaceholder')}
        rows={2}
      />
    );
  }

  if (type === 'rollback') {
    return (
      <SnippetInput
        nodeType={type}
        value={config}
        onChange={onChange}
        placeholder={tText('nodeEdit.rollbackCommand')}
        rows={3}
        monospace
      />
    );
  }

  if (type === 'switch') {
    return <SwitchConfigForm config={config} onChange={onChange} />;
  }

  if (type === 'selectHost') {
    return <SelectHostConfigForm config={config} onChange={onChange} />;
  }

  return <Input value={config} placeholder={tText('nodeEdit.configContent')} onChange={(e) => onChange(e.target.value)} />;
}

function SwitchConfigForm({ config, onChange }: { config: string; onChange: (v: string) => void }) {
  const { tText } = useTranslation();
  const cfg = parseConfig<SwitchConfig>(config, { expression: '', cases: [{ label: tText('nodeEdit.branchDefault'), value: '*' }] });

  const updateExpression = (expression: string) => {
    onChange(JSON.stringify({ ...cfg, expression }));
  };

  const updateCase = (index: number, partial: Partial<SwitchCase>) => {
    const next = cfg.cases.map((c, i) => i === index ? { ...c, ...partial } : c);
    onChange(JSON.stringify({ ...cfg, cases: next }));
  };

  const addCase = () => {
    const next = [...cfg.cases, { label: tText('nodeEdit.branchPlaceholder', { count: cfg.cases.length + 1 }), value: '' }];
    onChange(JSON.stringify({ ...cfg, cases: next }));
  };

  const removeCase = (index: number) => {
    const next = cfg.cases.filter((_, i) => i !== index);
    onChange(JSON.stringify({ ...cfg, cases: next }));
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
      <SnippetInput
        nodeType="switch"
        value={cfg.expression}
        onChange={updateExpression}
        placeholder={tText('nodeEdit.expressionPlaceholder')}
        monospace
      />
      <div style={{ fontSize: 12, color: 'var(--color-text-muted)', marginBottom: -2 }}>{tText('nodeEdit.branchList')}</div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 4, maxHeight: 240, overflow: 'auto' }}>
        {cfg.cases.map((c, i) => (
          <div key={i} style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
            <Input
              value={c.label}
              placeholder={tText('nodeEdit.labelPlaceholder')}
              onChange={(e) => updateCase(i, { label: e.target.value })}
              style={{ flex: 1, fontSize: 12 }}
            />
            <Input
              value={c.value}
              placeholder={tText('nodeEdit.valuePlaceholder')}
              onChange={(e) => updateCase(i, { value: e.target.value })}
              style={{ flex: 1, fontSize: 12, fontFamily: 'monospace' }}
            />
            <button
              type="button"
              onClick={() => removeCase(i)}
              style={{
                border: 'none', background: 'none', cursor: 'pointer',
                color: '#ff4d4f', fontSize: 14, padding: '0 4px', lineHeight: 1,
              }}
              title={tText('nodeEdit.deleteBranch')}
            >
              x
            </button>
          </div>
        ))}
      </div>
      <button
        type="button"
        onClick={addCase}
        style={{
          border: '1px dashed var(--color-border)', background: 'none',
          padding: '4px 0', cursor: 'pointer', borderRadius: 4, fontSize: 12,
          color: 'var(--color-primary)',
        }}
      >
        {tText('nodeEdit.addBranch')}
      </button>
    </div>
  );
}

function SelectHostConfigForm({ config, onChange }: { config: string; onChange: (v: string) => void }) {
  const { tText } = useTranslation();
  const hosts = useAssetsStore((s) => s.hosts);
  const cfg = parseConfig<SelectHostConfig>(config, { hostIds: [] });
  const toggle = (id: string) => {
    const next = cfg.hostIds.includes(id)
      ? cfg.hostIds.filter((h) => h !== id)
      : [...cfg.hostIds, id];
    onChange(JSON.stringify({ hostIds: next }));
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
      <div style={{ fontSize: 12, color: 'var(--color-text-muted)', marginBottom: 4 }}>{tText('nodeEdit.selectTargetHosts')}</div>
      <div style={{ maxHeight: 200, overflow: 'auto', border: '1px solid var(--color-border)', borderRadius: 6, padding: 4 }}>
        {hosts.map((h) => {
          const selected = cfg.hostIds.includes(h.id);
          return (
            <div
              key={h.id}
              style={{
                display: 'flex', alignItems: 'center', gap: 6, padding: '4px 6px',
                borderRadius: 4, cursor: 'pointer', background: selected ? 'rgba(22,119,255,0.08)' : undefined,
              }}
              onClick={() => toggle(h.id)}
            >
              <input type="checkbox" checked={selected} onChange={() => {}} onClick={(e) => e.stopPropagation()} />
              <CloudServerFilled style={{ fontSize: 10, color: '#999' }} />
              <span style={{ fontSize: 12 }}>{h.name || h.ip}</span>
            </div>
          );
        })}
        {hosts.length === 0 && <div style={{ fontSize: 12, color: '#999', textAlign: 'center', padding: 12 }}>{tText('nodeEdit.noHosts')}</div>}
      </div>
      {cfg.hostIds.length > 0 && (
        <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4, marginTop: 4 }}>
          {cfg.hostIds.map((id) => {
            const h = hosts.find((x) => x.id === id);
            return h ? (
              <Tag key={id} color="blue" style={{ fontSize: 10, cursor: 'pointer' }} onClick={() => toggle(id)}>
                {h.name || h.ip} ×
              </Tag>
            ) : null;
          })}
        </div>
      )}
    </div>
  );
}

export default function NodeEditModal({ open, nodeData, onSave, onClose }: Props) {
  const { tText } = useTranslation();
  const nodeTypeOptions = useNodeTypeOptions();
  const [name, setName] = useState('');
  const [type, setType] = useState('');
  const [config, setConfig] = useState('');

  useEffect(() => {
    if (nodeData) {
      setName(nodeData.name);
      setType(nodeData.type);
      setConfig(nodeData.config);
    }
  }, [nodeData]);

  const handleOk = () => {
    if (!nodeData) return;
    onSave(nodeData.id, { name, type, config });
  };

  const title = nodeData ? tText('nodeEdit.editNode', { name: nodeData.name }) : tText('wfEditor.editNode');

  return (
    <Modal title={title} open={open} onOk={handleOk} onCancel={onClose} destroyOnHidden>
      <div style={{ marginTop: 16, display: 'flex', flexDirection: 'column', gap: 10 }}>
        <div style={{ display: 'flex', gap: 8 }}>
          <Input style={{ flex: 1 }} value={name} placeholder={tText('wfEditor.nodeName')} onChange={(e) => setName(e.target.value)} />
          <Select value={type} onChange={(v) => setType(v)} options={nodeTypeOptions} style={{ width: 130 }} />
        </div>
        <ConfigForm type={type} config={config} onChange={setConfig} />
      </div>
    </Modal>
  );
}
