import { useEffect, useState, useMemo, useCallback, memo, type ReactNode } from 'react';
import { Input, Tag, Empty, Tooltip, Modal } from './ui';
import { SearchOutlined, PlayCircleOutlined, StarOutlined, StarFilled } from './ui/icons';
import { useLibraryStore } from '../stores/library';
import { useTranslation } from '../i18n';
import type { TranslationKey } from '../i18n';
import type { CommandEntry, CommandParameter } from '../types';
import { buildRemoteScriptCommand } from '../utils/remoteScriptCommand';
import '../styles/panels/inline-panels.css';
import '../styles/dialogs/command-execution.css';

const RISK_COLORS: Record<string, string> = { low: 'green', medium: 'orange', high: 'red', critical: 'magenta' };
const RISK_LABEL_KEYS: Record<string, TranslationKey> = { low: 'library.risk.low', medium: 'library.risk.medium', high: 'library.risk.high', critical: 'library.risk.critical' };

interface Props {
  insertCommand?: (command: string) => void | Promise<void>;
}

function resolveCommandParams(command: string, params: CommandParameter[], values: Record<string, string>): string {
  let resolved = command;
  for (const param of params) {
    const value = values[param.name] ?? param.defaultValue ?? '';
    // Replace ${PARAM_NAME} with the provided value
    resolved = resolved.replace(new RegExp(`\\$\\{${param.name}\\}`, 'g'), value);
  }
  return resolved;
}

/**
 * Build the effective command for a given CommandEntry + parameter values.
 * Uses URL-based remote execution when command.url is present; otherwise
 * falls back to local command template substitution.
 */
function buildEffectiveCommand(command: CommandEntry, paramValues: Record<string, string>): string {
  if (command.url) {
    return buildRemoteScriptCommand(command.url, command.parameters, paramValues);
  }
  return resolveCommandParams(command.command, command.parameters, paramValues);
}

// ---------------------------------------------------------------------------
// Lightweight shell syntax highlighter
// ---------------------------------------------------------------------------

const SHELL_KEYWORDS = new Set([
  'if', 'then', 'else', 'elif', 'fi', 'for', 'while', 'until', 'do', 'done',
  'case', 'esac', 'in', 'function', 'select', 'time', 'coproc', 'return',
  'exit', 'export', 'local', 'declare', 'readonly', 'set', 'unset', 'shift',
  'source', 'alias', 'unalias', 'trap', 'eval', 'exec', 'read', 'echo',
  'printf', 'test', 'true', 'false', 'let', 'typeset', 'hash', 'type',
  'command', 'builtin', 'break', 'continue', 'getopts', 'wait', 'jobs',
  'bg', 'fg', 'disown', 'suspend', 'logout', 'mapfile', 'readarray',
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
  'uname', 'whoami', 'id', 'w', 'who', 'last', 'lastb', 'dmesg',
  'lscpu', 'lsmod', 'modprobe', 'insmod', 'rmmod', 'dmidecode',
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
    // Comment
    if (code[i] === '#') {
      const start = i;
      while (i < len && code[i] !== '\n') i++;
      tokens.push({ type: 'comment', text: code.slice(start, i) });
      continue;
    }

    // Single-quoted string
    if (code[i] === "'") {
      const start = i;
      i++;
      while (i < len && code[i] !== "'") i++;
      if (i < len) i++;
      tokens.push({ type: 'string', text: code.slice(start, i) });
      continue;
    }

    // Double-quoted string
    if (code[i] === '"') {
      const start = i;
      i++;
      while (i < len && code[i] !== '"') {
        if (code[i] === '\\' && i + 1 < len) i++;
        i++;
      }
      if (i < len) i++;
      tokens.push({ type: 'string', text: code.slice(start, i) });
      continue;
    }

    // Variable: ${...} or $VARNAME
    if (code[i] === '$') {
      if (i + 1 < len && code[i + 1] === '{') {
        const start = i;
        i += 2;
        while (i < len && code[i] !== '}') i++;
        if (i < len) i++;
        tokens.push({ type: 'variable', text: code.slice(start, i) });
        continue;
      }
      if (i + 1 < len && (code[i + 1] === '?' || code[i + 1] === '!' || code[i + 1] === '$' || code[i + 1] === '-' || code[i + 1] === '_' || /[a-zA-Z]/.test(code[i + 1]))) {
        const start = i;
        i++;
        if (/[a-zA-Z_]/.test(code[i])) {
          while (i < len && /[a-zA-Z0-9_]/.test(code[i])) i++;
        } else {
          i++;
        }
        tokens.push({ type: 'variable', text: code.slice(start, i) });
        continue;
      }
      // Lone $ — fall through
    }

    // Operators: |, ||, &&, >, >>, <, <<, ;, ;;, &
    if ('|&;><'.includes(code[i])) {
      const start = i;
      i++;
      if (i < len && code[i] === code[start] && code[start] !== ';') i++; // ||, &&, >>, <<
      if (start === 0 || /[\s;|&(<>'"]/.test(code[start - 1])) {
        // treat as operator
      }
      tokens.push({ type: 'operator', text: code.slice(start, i) });
      continue;
    }

    // Number
    if (/[0-9]/.test(code[i]) && (i === 0 || /[\s/=<>]/.test(code[i - 1]))) {
      const start = i;
      while (i < len && /[0-9]/.test(code[i])) i++;
      tokens.push({ type: 'number', text: code.slice(start, i) });
      continue;
    }

    // Word (identifier, keyword, builtin, or path)
    if (/[a-zA-Z_.\/~\-]/.test(code[i])) {
      const start = i;
      while (i < len && /[a-zA-Z0-9_.\/~\-]/.test(code[i])) i++;
      const word = code.slice(start, i);
      if (SHELL_KEYWORDS.has(word)) {
        tokens.push({ type: 'keyword', text: word });
      } else if (SHELL_BUILTINS.has(word)) {
        tokens.push({ type: 'builtin', text: word });
      } else {
        tokens.push({ type: 'text', text: word });
      }
      continue;
    }

    // Whitespace / other
    tokens.push({ type: 'text', text: code[i] });
    i++;
  }

  return tokens;
}

function ShellHighlight({ code }: { code: string }): ReactNode {
  const tokens = useMemo(() => tokenizeShell(code), [code]);
  return (
    <>
      {tokens.map((token, idx) => (
        <span key={idx} className={`shell-hl-${token.type}`}>{token.text}</span>
      ))}
    </>
  );
}

function CommandParamsDialogInner({
  command,
  open,
  onConfirm,
  onCancel,
}: {
  command: CommandEntry;
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

  // Reset values when command changes
  useEffect(() => {
    if (open) {
      const defaults: Record<string, string> = {};
      for (const param of command.parameters) {
        defaults[param.name] = param.defaultValue ?? '';
      }
      setParamValues(defaults);
      setValidationError(null);
      setPreviewTab('command');
      setRemoteSource(null);
      setRemoteError(null);
    }
  }, [open, command.parameters]);

  // Fetch remote script source when dialog opens and command has a URL
  useEffect(() => {
    if (!open || !command.url) return;

    let disposed = false;
    setRemoteLoading(true);
    setRemoteError(null);

    fetch(command.url)
      .then(async (res) => {
        if (disposed) return;
        if (!res.ok) {
          setRemoteError(`HTTP ${res.status} ${res.statusText}`);
          return;
        }
        const text = await res.text();
        if (disposed) return;
        setRemoteSource(text);
      })
      .catch((err: unknown) => {
        if (disposed) return;
        setRemoteError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (!disposed) setRemoteLoading(false);
      });

    return () => { disposed = true; };
  }, [open, command.url]);

  const handleParamChange = useCallback((name: string, value: string) => {
    setParamValues((prev) => ({ ...prev, [name]: value }));
    setValidationError(null);
  }, []);

  const resolvedCommand = useMemo(
    () => buildEffectiveCommand(command, paramValues),
    [command, paramValues],
  );

  const handleConfirm = useCallback(() => {
    // Validate required parameters
    for (const param of command.parameters) {
      if (param.required && !paramValues[param.name]?.trim()) {
        setValidationError(tText('inlinePanel.paramRequired', { name: param.name }));
        return;
      }
    }
    onConfirm(resolvedCommand);
  }, [command.parameters, paramValues, resolvedCommand, onConfirm]);

  const hasParams = command.parameters.length > 0;
  const isHighRisk = command.risk === 'high' || command.risk === 'critical';

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
            <h3 className="command-exec-dialog-title">{command.name}</h3>
          </div>
          <div className="command-exec-dialog-header-right">
            {command.url && <Tag color="cyan">{t('inlinePanel.remoteScript')}</Tag>}
            <Tag color={RISK_COLORS[command.risk]}>{t(RISK_LABEL_KEYS[command.risk])}</Tag>
          </div>
        </div>
        {command.description && (
          <p className="command-exec-dialog-desc">{command.description}</p>
        )}

        {/* Parameters */}
        {hasParams && (
          <div className="command-exec-dialog-params">
            <div className="command-exec-dialog-params-title">{t('inlinePanel.params')}</div>
            {command.parameters.map((param) => (
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
            {command.url && (
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
}
export const CommandParamsDialog = memo(CommandParamsDialogInner);
CommandParamsDialog.displayName = 'CommandParamsDialog';

const CommandsInlinePanel = memo(function CommandsInlinePanel({ insertCommand }: Props) {
  const commands = useLibraryStore((s) => s.commands);
  const loadCommands = useLibraryStore((s) => s.loadCommands);
  const toggleStarCommand = useLibraryStore((s) => s.toggleStarCommand);
  const { t, tText } = useTranslation();
  const [searchQuery, setSearchQuery] = useState('');
  const [selectedCategory, setSelectedCategory] = useState<string | null>(null);
  const [dialogCommand, setDialogCommand] = useState<CommandEntry | null>(null);

  useEffect(() => { loadCommands(); }, [loadCommands]);

  const categories = useMemo(() => {
    const cats = new Set(commands.map((c) => c.category));
    return Array.from(cats).sort();
  }, [commands]);

  const filtered = useMemo(() => {
    return commands.filter((c) => {
      if (selectedCategory && c.category !== selectedCategory) return false;
      if (searchQuery) {
        const q = searchQuery.toLowerCase();
        return c.name.toLowerCase().includes(q) || c.command.toLowerCase().includes(q) || c.description.toLowerCase().includes(q);
      }
      return true;
    });
  }, [commands, searchQuery, selectedCategory]);

  const handleInsert = useCallback((cmd: CommandEntry) => {
    setDialogCommand(cmd);
  }, []);

  const handleDialogConfirm = useCallback((resolvedCommand: string) => {
    setDialogCommand(null);
    void insertCommand?.(resolvedCommand);
  }, [insertCommand]);

  const handleDialogCancel = useCallback(() => {
    setDialogCommand(null);
  }, []);

  const handleToggleStar = useCallback(async (id: string) => {
    await toggleStarCommand(id);
  }, [toggleStarCommand]);

  return (
    <div className="inline-panel-shell">
      <div className="inline-panel-toolbar">
        <Input
          prefix={<SearchOutlined />}
          placeholder={tText('inlinePanel.searchCommands')}
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
          <Empty description={t('inlinePanel.noCommands')} image={Empty.PRESENTED_IMAGE_SIMPLE} />
        ) : (
          filtered.map((cmd) => (
            <div key={cmd.id} className="inline-panel-item">
              <div className="inline-panel-item-header">
                <span className="inline-panel-item-name">{cmd.name}</span>
                <div className="inline-panel-item-actions">
                  <Tag color={RISK_COLORS[cmd.risk]}>{t(RISK_LABEL_KEYS[cmd.risk])}</Tag>
                  {cmd.url && (
                    <Tag color="cyan">{t('library.remote')}</Tag>
                  )}
                  {cmd.parameters.length > 0 && (
                    <Tag color="blue">{t('inlinePanel.params')}</Tag>
                  )}
                  <Tooltip title={cmd.starred ? tText('inlinePanel.unstar') : tText('inlinePanel.star')}>
                    <button className="inline-panel-icon-btn" onClick={() => handleToggleStar(cmd.id)}>
                      {cmd.starred ? <StarFilled style={{ color: '#faad14' }} /> : <StarOutlined />}
                    </button>
                  </Tooltip>
                  {insertCommand && (
                    <Tooltip title={tText('inlinePanel.insertTerminal')}>
                      <button className="inline-panel-icon-btn inline-panel-icon-btn-run" onClick={() => handleInsert(cmd)}>
                        <PlayCircleOutlined />
                      </button>
                    </Tooltip>
                  )}
                </div>
              </div>
              <div className="inline-panel-item-desc">{cmd.description}</div>
              <code className="inline-panel-item-code">{cmd.url ? `curl -sSL '${cmd.url}' | bash` : cmd.command}</code>
            </div>
          ))
        )}
      </div>

      {dialogCommand && (
        <CommandParamsDialog
          command={dialogCommand}
          open={!!dialogCommand}
          onConfirm={handleDialogConfirm}
          onCancel={handleDialogCancel}
        />
      )}
    </div>
  );
});
CommandsInlinePanel.displayName = 'CommandsInlinePanel';
export default CommandsInlinePanel;
