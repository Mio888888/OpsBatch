import { useEffect, useState, useMemo, useCallback, memo, type ReactNode } from 'react';
import { Input, Tag, Empty, Tooltip, Modal } from './ui';
import { SearchOutlined, PlayCircleOutlined, StarOutlined, StarFilled } from './ui/icons';
import { useLibraryStore } from '../stores/library';
import { useTranslation } from '../i18n';
import type { TranslationKey } from '../i18n';
import type { ScriptEntry } from '../types';
import { buildRemoteScriptCommand } from '../utils/remoteScriptCommand';

const RISK_COLORS: Record<string, string> = { low: 'green', medium: 'orange', high: 'red', critical: 'magenta' };
const RISK_LABEL_KEYS: Record<string, TranslationKey> = { low: 'library.risk.low', medium: 'library.risk.medium', high: 'library.risk.high', critical: 'library.risk.critical' };
const LANG_COLORS: Record<string, string> = { shell: 'blue', python: 'green', powershell: 'purple' };

interface Props {
  insertCommand?: (command: string) => void | Promise<void>;
}

// ---------------------------------------------------------------------------
// Shell syntax highlighter (shared with CommandsInlinePanel)
// ---------------------------------------------------------------------------

const SHELL_KEYWORDS = new Set([
  'if', 'then', 'else', 'elif', 'fi', 'for', 'while', 'until', 'do', 'done',
  'case', 'esac', 'in', 'function', 'select', 'time', 'coproc', 'return',
  'exit', 'export', 'local', 'declare', 'readonly', 'set', 'unset', 'shift',
  'source', 'alias', 'unalias', 'trap', 'eval', 'exec', 'read', 'echo',
  'printf', 'test', 'true', 'false', 'let', 'typeset', 'hash', 'type',
  'command', 'builtin', 'break', 'continue', 'getopts', 'wait', 'jobs',
]);

const SHELL_BUILTINS = new Set([
  'sudo', 'apt', 'yum', 'dnf', 'rpm', 'dpkg', 'snap', 'pip', 'npm', 'npx',
  'git', 'docker', 'kubectl', 'ssh', 'scp', 'rsync', 'curl', 'wget',
  'systemctl', 'journalctl', 'service', 'crontab', 'chmod', 'chown',
  'mkdir', 'rmdir', 'rm', 'cp', 'mv', 'ln', 'touch', 'find', 'grep',
  'awk', 'sed', 'sort', 'uniq', 'wc', 'head', 'tail', 'cat', 'tee',
  'xargs', 'tr', 'cut', 'paste', 'diff', 'patch', 'tar', 'gzip', 'gunzip',
  'zip', 'unzip', 'mount', 'umount', 'fdisk', 'mkfs', 'fsck', 'du', 'df',
  'ls', 'lsblk', 'blkid', 'top', 'htop', 'ps', 'kill', 'killall', 'nice',
  'renice', 'nohup', 'timeout', 'sleep', 'date', 'cal', 'uptime',
  'free', 'vmstat', 'iostat', 'mpstat', 'lsof', 'ss', 'ip', 'ifconfig',
  'ping', 'traceroute', 'dig', 'nslookup', 'host', 'netstat', 'nc',
  'iptables', 'ufw', 'firewall-cmd', 'badblocks', 'hostnamectl',
]);

type TokenType = 'keyword' | 'builtin' | 'string' | 'variable' | 'operator' | 'comment' | 'number' | 'text';

interface Token {
  type: TokenType;
  text: string;
}

function tokenizeShell(code: string): Token[] {
  const tokens: Token[] = [];
  let i = 0;
  const len = code.length;

  while (i < len) {
    if (code[i] === '#') {
      const start = i;
      while (i < len && code[i] !== '\n') i++;
      tokens.push({ type: 'comment', text: code.slice(start, i) });
      continue;
    }
    if (code[i] === "'") {
      const start = i; i++;
      while (i < len && code[i] !== "'") i++;
      if (i < len) i++;
      tokens.push({ type: 'string', text: code.slice(start, i) });
      continue;
    }
    if (code[i] === '"') {
      const start = i; i++;
      while (i < len && code[i] !== '"') { if (code[i] === '\\' && i + 1 < len) i++; i++; }
      if (i < len) i++;
      tokens.push({ type: 'string', text: code.slice(start, i) });
      continue;
    }
    if (code[i] === '$') {
      if (i + 1 < len && code[i + 1] === '{') {
        const start = i; i += 2;
        while (i < len && code[i] !== '}') i++;
        if (i < len) i++;
        tokens.push({ type: 'variable', text: code.slice(start, i) });
        continue;
      }
      if (i + 1 < len && /[a-zA-Z_? !$\-]/.test(code[i + 1])) {
        const start = i; i++;
        if (/[a-zA-Z_]/.test(code[i])) { while (i < len && /[a-zA-Z0-9_]/.test(code[i])) i++; } else { i++; }
        tokens.push({ type: 'variable', text: code.slice(start, i) });
        continue;
      }
    }
    if ('|&;><'.includes(code[i])) {
      const start = i; i++;
      if (i < len && code[i] === code[start] && code[start] !== ';') i++;
      tokens.push({ type: 'operator', text: code.slice(start, i) });
      continue;
    }
    if (/[0-9]/.test(code[i]) && (i === 0 || /[\s/=<>]/.test(code[i - 1]))) {
      const start = i;
      while (i < len && /[0-9]/.test(code[i])) i++;
      tokens.push({ type: 'number', text: code.slice(start, i) });
      continue;
    }
    if (/[a-zA-Z_.\/~\-]/.test(code[i])) {
      const start = i;
      while (i < len && /[a-zA-Z0-9_.\/~\-]/.test(code[i])) i++;
      const word = code.slice(start, i);
      tokens.push({
        type: SHELL_KEYWORDS.has(word) ? 'keyword' : SHELL_BUILTINS.has(word) ? 'builtin' : 'text',
        text: word,
      });
      continue;
    }
    tokens.push({ type: 'text', text: code[i] });
    i++;
  }
  return tokens;
}

function ShellHighlight({ code }: { code: string }): ReactNode {
  const tokens = useMemo(() => tokenizeShell(code), [code]);
  return <>{tokens.map((t, i) => <span key={i} className={`shell-hl-${t.type}`}>{t.text}</span>)}</>;
}

// ---------------------------------------------------------------------------
// Script execution helpers
// ---------------------------------------------------------------------------

function buildLocalScriptCommand(content: string): string {
  return content;
}

function buildEffectiveScriptCommand(script: ScriptEntry, paramValues: Record<string, string>): string {
  if (script.url) {
    return buildRemoteScriptCommand(script.url, script.parameters, paramValues);
  }
  return buildLocalScriptCommand(script.content);
}

// ---------------------------------------------------------------------------
// Script params dialog
// ---------------------------------------------------------------------------

const ScriptParamsDialog = memo(function ScriptParamsDialog({
  script,
  open,
  onConfirm,
  onCancel,
}: {
  script: ScriptEntry;
  open: boolean;
  onConfirm: (resolvedCommand: string) => void;
  onCancel: () => void;
}) {
  const { t, tText } = useTranslation();
  const [paramValues, setParamValues] = useState<Record<string, string>>({});
  const [validationError, setValidationError] = useState<string | null>(null);
  const [previewTab, setPreviewTab] = useState<'command' | 'source'>('command');
  const [remoteSource, setRemoteSource] = useState<string | null>(null);
  const [remoteLoading, setRemoteLoading] = useState(false);
  const [remoteError, setRemoteError] = useState<string | null>(null);

  // Reset when dialog opens
  useEffect(() => {
    if (open) {
      const defaults: Record<string, string> = {};
      for (const param of script.parameters) {
        defaults[param.name] = param.defaultValue ?? '';
      }
      setParamValues(defaults);
      setValidationError(null);
      setPreviewTab('command');
      setRemoteSource(null);
      setRemoteError(null);
    }
  }, [open, script.parameters]);

  // Fetch remote source
  useEffect(() => {
    if (!open || !script.url) return;
    let disposed = false;
    setRemoteLoading(true);
    setRemoteError(null);

    fetch(script.url)
      .then(async (res) => {
        if (disposed) return;
        if (!res.ok) { setRemoteError(`HTTP ${res.status} ${res.statusText}`); return; }
        const text = await res.text();
        if (disposed) return;
        setRemoteSource(text);
      })
      .catch((err: unknown) => {
        if (disposed) return;
        setRemoteError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => { if (!disposed) setRemoteLoading(false); });

    return () => { disposed = true; };
  }, [open, script.url]);

  const handleParamChange = useCallback((name: string, value: string) => {
    setParamValues((prev) => ({ ...prev, [name]: value }));
    setValidationError(null);
  }, []);

  const resolvedCommand = useMemo(
    () => buildEffectiveScriptCommand(script, paramValues),
    [script, paramValues],
  );

  const handleConfirm = useCallback(() => {
    for (const param of script.parameters) {
      if (param.required && !paramValues[param.name]?.trim()) {
        setValidationError(tText('inlinePanel.paramRequired', { name: param.name }));
        return;
      }
    }
    onConfirm(resolvedCommand);
  }, [script.parameters, paramValues, resolvedCommand, onConfirm]);

  const hasParams = script.parameters.length > 0;
  const isHighRisk = script.risk === 'high' || script.risk === 'critical';

  return (
    <Modal
      open={open}
      title={null}
      onCancel={onCancel}
      footer={null}
      width={560}
      className="command-exec-dialog"
    >
      <div className="command-exec-dialog-content">
        {/* Header */}
        <div className="command-exec-dialog-header">
          <div className="command-exec-dialog-header-left">
            <span className="command-exec-dialog-risk-dot" style={{ background: isHighRisk ? 'var(--terminal-red, #f85149)' : 'var(--terminal-accent, #1677ff)' }} />
            <h3 className="command-exec-dialog-title">{script.name}</h3>
          </div>
          <div className="command-exec-dialog-header-right">
            <Tag color={LANG_COLORS[script.language]}>{script.language}</Tag>
            {script.url && <Tag color="cyan">{t('inlinePanel.remoteScript')}</Tag>}
            <Tag color={RISK_COLORS[script.risk]}>{t(RISK_LABEL_KEYS[script.risk])}</Tag>
          </div>
        </div>
        {script.description && (
          <p className="command-exec-dialog-desc">{script.description}</p>
        )}

        {/* Parameters */}
        {hasParams && (
          <div className="command-exec-dialog-params">
            <div className="command-exec-dialog-params-title">{t('inlinePanel.params')}</div>
            {script.parameters.map((param) => (
              <div key={param.name} className="command-exec-dialog-param-row">
                <label className="command-exec-dialog-param-label">
                  {param.name}
                  {param.required && <span className="command-exec-dialog-param-required">*</span>}
                  {param.description && (
                    <span className="command-exec-dialog-param-desc-inline"> — {param.description}</span>
                  )}
                </label>
                <Input
                  size="small"
                  value={paramValues[param.name] ?? ''}
                  onChange={(e) => handleParamChange(param.name, e.target.value)}
                  placeholder={param.defaultValue || tText('inlinePanel.paramPlaceholder', { name: param.name })}
                />
              </div>
            ))}
            {validationError && (
              <div className="command-exec-dialog-validation-error">{validationError}</div>
            )}
          </div>
        )}

        {/* Preview tabs: command + source */}
        <div className="command-exec-dialog-preview">
          <div className="command-exec-dialog-preview-bar">
            <button
              className={`command-exec-dialog-preview-tab ${previewTab === 'command' ? 'command-exec-dialog-preview-tab-active' : ''}`}
              onClick={() => setPreviewTab('command')}
            >
              {t('inlinePanel.commandPreview')}
            </button>
            {script.url && (
              <button
                className={`command-exec-dialog-preview-tab ${previewTab === 'source' ? 'command-exec-dialog-preview-tab-active' : ''}`}
                onClick={() => setPreviewTab('source')}
              >
                {t('inlinePanel.scriptSource')}
              </button>
            )}
          </div>
          <div className="command-exec-dialog-preview-body">
            {previewTab === 'command' ? (
              <pre className="command-exec-dialog-preview-code"><ShellHighlight code={resolvedCommand} /></pre>
            ) : (
              <div className="command-exec-dialog-source-preview">
                {remoteLoading ? (
                  <div className="command-exec-dialog-source-status">
                    <span className="command-exec-dialog-source-spinner" />
                    {t('inlinePanel.loadingScript')}
                  </div>
                ) : remoteError ? (
                  <div className="command-exec-dialog-source-status command-exec-dialog-source-error">{t('inlinePanel.loadFailed', { error: remoteError })}</div>
                ) : remoteSource ? (
                  <pre className="command-exec-dialog-preview-code"><ShellHighlight code={remoteSource} /></pre>
                ) : (
                  <div className="command-exec-dialog-source-status">{t('inlinePanel.noSource')}</div>
                )}
              </div>
            )}
          </div>
        </div>

        {/* Actions */}
        <div className="command-exec-dialog-actions">
          <button className="command-exec-dialog-btn command-exec-dialog-btn-cancel" onClick={onCancel}>
            {t('common.cancel')}
          </button>
          <button
            className={`command-exec-dialog-btn command-exec-dialog-btn-confirm ${isHighRisk ? 'command-exec-dialog-btn-danger' : ''}`}
            onClick={handleConfirm}
          >
            {t('inlinePanel.insertTerminal')}
          </button>
        </div>
      </div>
    </Modal>
  );
});
ScriptParamsDialog.displayName = 'ScriptParamsDialog';

// ---------------------------------------------------------------------------
// Scripts list panel
// ---------------------------------------------------------------------------

const ScriptsInlinePanel = memo(function ScriptsInlinePanel({ insertCommand }: Props) {
  const scripts = useLibraryStore((s) => s.scripts);
  const loadScripts = useLibraryStore((s) => s.loadScripts);
  const toggleStarScript = useLibraryStore((s) => s.toggleStarScript);
  const { t, tText } = useTranslation();
  const [searchQuery, setSearchQuery] = useState('');
  const [selectedCategory, setSelectedCategory] = useState<string | null>(null);
  const [dialogScript, setDialogScript] = useState<ScriptEntry | null>(null);

  useEffect(() => { loadScripts(); }, [loadScripts]);

  const categories = useMemo(() => {
    const cats = new Set(scripts.map((s) => s.category));
    return Array.from(cats).sort();
  }, [scripts]);

  const filtered = useMemo(() => {
    return scripts.filter((s) => {
      if (selectedCategory && s.category !== selectedCategory) return false;
      if (searchQuery) {
        const q = searchQuery.toLowerCase();
        return s.name.toLowerCase().includes(q) || s.description.toLowerCase().includes(q) || s.content.toLowerCase().includes(q);
      }
      return true;
    });
  }, [scripts, searchQuery, selectedCategory]);

  const handleInsert = useCallback((script: ScriptEntry) => {
    setDialogScript(script);
  }, []);

  const handleDialogConfirm = useCallback((resolvedCommand: string) => {
    setDialogScript(null);
    void insertCommand?.(resolvedCommand);
  }, [insertCommand]);

  const handleDialogCancel = useCallback(() => {
    setDialogScript(null);
  }, []);

  const handleToggleStar = useCallback(async (id: string) => {
    await toggleStarScript(id);
  }, [toggleStarScript]);

  return (
    <div className="inline-panel-shell">
      <div className="inline-panel-toolbar">
        <Input
          prefix={<SearchOutlined />}
          placeholder={tText('inlinePanel.searchScripts')}
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          allowClear
          size="small"
        />
        <div className="inline-panel-categories">
          <button
            className={`inline-panel-cat-btn ${!selectedCategory ? 'inline-panel-cat-btn-active' : ''}`}
            onClick={() => setSelectedCategory(null)}
          >
            {t('inlinePanel.all')}
          </button>
          {categories.map((cat) => (
            <button
              key={cat}
              className={`inline-panel-cat-btn ${selectedCategory === cat ? 'inline-panel-cat-btn-active' : ''}`}
              onClick={() => setSelectedCategory(cat === selectedCategory ? null : cat)}
            >
              {cat}
            </button>
          ))}
        </div>
      </div>
      <div className="inline-panel-list">
        {filtered.length === 0 ? (
          <Empty description={t('inlinePanel.noScripts')} image={Empty.PRESENTED_IMAGE_SIMPLE} />
        ) : (
          filtered.map((script) => (
            <div key={script.id} className="inline-panel-item">
              <div className="inline-panel-item-header">
                <span className="inline-panel-item-name">{script.name}</span>
                <div className="inline-panel-item-actions">
                  <Tag color={LANG_COLORS[script.language]}>{script.language}</Tag>
                  <Tag color={RISK_COLORS[script.risk]}>{t(RISK_LABEL_KEYS[script.risk])}</Tag>
                  {script.url && <Tag color="cyan">{t('library.remote')}</Tag>}
                  {script.parameters.length > 0 && (
                    <Tag color="blue">{t('inlinePanel.params')}</Tag>
                  )}
                  <Tooltip title={script.starred ? tText('inlinePanel.unstar') : tText('inlinePanel.star')}>
                    <button className="inline-panel-icon-btn" onClick={() => handleToggleStar(script.id)}>
                      {script.starred ? <StarFilled style={{ color: '#faad14' }} /> : <StarOutlined />}
                    </button>
                  </Tooltip>
                  {insertCommand && (
                    <Tooltip title={tText('inlinePanel.insertTerminal')}>
                      <button className="inline-panel-icon-btn inline-panel-icon-btn-run" onClick={() => handleInsert(script)}>
                        <PlayCircleOutlined />
                      </button>
                    </Tooltip>
                  )}
                </div>
              </div>
              <div className="inline-panel-item-desc">{script.description}</div>
              <code className="inline-panel-item-code">
                {script.url
                  ? `curl -sSL '${script.url}' | bash`
                  : (script.content.length > 120 ? script.content.slice(0, 120) + '…' : script.content)}
              </code>
            </div>
          ))
        )}
      </div>

      {dialogScript && (
        <ScriptParamsDialog
          script={dialogScript}
          open={!!dialogScript}
          onConfirm={handleDialogConfirm}
          onCancel={handleDialogCancel}
        />
      )}
    </div>
  );
});
ScriptsInlinePanel.displayName = 'ScriptsInlinePanel';
export default ScriptsInlinePanel;
