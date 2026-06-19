import { memo, useCallback, useEffect, useMemo, useState } from 'react';
import { Empty, Tag, Tooltip, message, Input } from './ui';
import { SearchOutlined, PlayCircleOutlined } from './ui/icons';
import { useTranslation, type TranslationKey } from '../i18n';
import { useLibraryStore } from '../stores/library';
import type { CommandEntry } from '../types';
import { CommandParamsDialog } from './CommandsInlinePanel';
import '../styles/panels/inline-panels.css';
import '../styles/panels/docker-panel.css';

interface DockerInlinePanelProps {
  executeCommand?: (command: string, options?: { timeoutMs?: number }) => Promise<unknown> | undefined;
  insertCommand?: (command: string) => Promise<unknown> | undefined;
}

const RISK_LABEL_KEYS: Record<string, TranslationKey> = {
  low: 'library.risk.low',
  medium: 'library.risk.medium',
  high: 'library.risk.high',
  critical: 'library.risk.critical',
};

const RISK_COLORS: Record<string, string> = {
  low: 'green',
  medium: 'orange',
  high: 'red',
  critical: 'magenta',
};

function DockerInlinePanel({ executeCommand, insertCommand }: DockerInlinePanelProps) {
  const commands = useLibraryStore((s) => s.commands);
  const loadCommands = useLibraryStore((s) => s.loadCommands);
  const { t, tText } = useTranslation();
  const [searchQuery, setSearchQuery] = useState('');
  const [selectedCategory, setSelectedCategory] = useState<string | null>(null);
  const [dialogCommand, setDialogCommand] = useState<CommandEntry | null>(null);

  useEffect(() => {
    loadCommands();
  }, [loadCommands]);

  const dockerCommands = useMemo(() => commands.filter((cmd) => cmd.kind === 'docker'), [commands]);

  const categories = useMemo(() => {
    const categorySet = new Set<string>();
    for (const command of dockerCommands) {
      if (command.category.trim()) categorySet.add(command.category);
    }
    return Array.from(categorySet).sort();
  }, [dockerCommands]);

  const filtered = useMemo(() => {
    return dockerCommands.filter((cmd) => {
      if (selectedCategory && cmd.category !== selectedCategory) return false;
      if (!searchQuery) return true;
      const q = searchQuery.toLowerCase();
      return cmd.name.toLowerCase().includes(q)
        || cmd.command.toLowerCase().includes(q)
        || cmd.description.toLowerCase().includes(q)
        || cmd.tags.some((tag) => tag.toLowerCase().includes(q));
    });
  }, [dockerCommands, searchQuery, selectedCategory]);

  const handleAction = useCallback(async (command: CommandEntry) => {
    if (command.parameters.length > 0) {
      setDialogCommand(command);
      return;
    }

    if (insertCommand) {
      await insertCommand(command.url ? `curl -sSL '${command.url}' | bash` : command.command);
      message.success(tText('docker.commandInserted'));
      return;
    }

    if (!executeCommand) {
      message.warning(tText('docker.terminalNotReady'));
      return;
    }

    try {
      await executeCommand(command.command, { timeoutMs: 30000 });
    } catch (error) {
      message.error(tText('docker.executeFailed', { error: error instanceof Error ? error.message : String(error) }));
    }
  }, [executeCommand, insertCommand, tText]);

  const handleDialogConfirm = useCallback(async (resolvedCommand: string) => {
    setDialogCommand(null);
    if (insertCommand) {
      await insertCommand(resolvedCommand);
      message.success(tText('docker.commandInserted'));
      return;
    }
    if (!executeCommand) {
      message.warning(tText('docker.terminalNotReady'));
      return;
    }
    try {
      await executeCommand(resolvedCommand, { timeoutMs: 30000 });
    } catch (error) {
      message.error(tText('docker.executeFailed', { error: error instanceof Error ? error.message : String(error) }));
    }
  }, [executeCommand, insertCommand, tText]);

  const handleDialogCancel = useCallback(() => {
    setDialogCommand(null);
  }, []);

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
          filtered.map((action) => (
            <div key={action.id} className="inline-panel-item docker-panel-inline-item">
              <div className="inline-panel-item-header">
                <span className="inline-panel-item-name">{action.name}</span>
                <div className="inline-panel-item-actions">
                  <Tag color="blue">{action.category || 'Docker'}</Tag>
                  <Tag color={RISK_COLORS[action.risk]}>{t(RISK_LABEL_KEYS[action.risk])}</Tag>
                  {action.parameters.length > 0 && <Tag color="cyan">{t('inlinePanel.params')}</Tag>}
                  <Tooltip title={tText('docker.tooltip.insert')}>
                    <button
                      className="inline-panel-icon-btn inline-panel-icon-btn-run"
                      onClick={() => void handleAction(action)}
                    >
                      <PlayCircleOutlined />
                    </button>
                  </Tooltip>
                </div>
              </div>
              <div className="inline-panel-item-desc">{action.description}</div>
              <code className="inline-panel-item-code">{action.url ? `curl -sSL '${action.url}' | bash` : action.command}</code>
            </div>
          ))
        )}
      </div>

      {dialogCommand && (
        <CommandParamsDialog
          command={dialogCommand}
          open={!!dialogCommand}
          onConfirm={(resolvedCommand) => { void handleDialogConfirm(resolvedCommand); }}
          onCancel={handleDialogCancel}
        />
      )}
    </div>
  );
}

export default memo(DockerInlinePanel);
