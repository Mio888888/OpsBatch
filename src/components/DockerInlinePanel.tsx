import { memo, useMemo, useState } from 'react';
import { Empty, Tag, Tooltip, message } from './ui';
import { PlayCircleOutlined } from './ui/icons';
import { useTranslation, type TranslationKey } from '../i18n';
import type { TerminalController } from './TerminalView';
import '../styles/panels/inline-panels.css';
import '../styles/panels/docker-panel.css';

interface DockerInlinePanelProps {
  executeCommand?: (command: string, options?: Parameters<TerminalController['executeCommand']>[1]) => ReturnType<TerminalController['executeCommand']> | undefined;
  insertCommand?: (command: string) => ReturnType<TerminalController['insertCommand']> | undefined;
}

type DockerRisk = 'safe' | 'change' | 'danger';
type DockerCategory = 'overview' | 'container' | 'image' | 'network' | 'storage' | 'compose' | 'security' | 'diagnostics' | 'cleanup';
type DockerCategoryFilter = 'all' | DockerCategory;

interface DockerAction {
  id: string;
  titleKey: TranslationKey;
  descriptionKey: TranslationKey;
  command: string;
  category: DockerCategory;
  risk: DockerRisk;
  mode?: 'run' | 'insert';
}

const RISK_LABEL_KEYS: Record<DockerRisk, TranslationKey> = {
  safe: 'docker.risk.safe',
  change: 'docker.risk.change',
  danger: 'docker.risk.danger',
};

const RISK_COLORS: Record<DockerRisk, string> = {
  safe: 'green',
  change: 'orange',
  danger: 'red',
};

const CATEGORY_LABEL_KEYS: Record<DockerCategory, TranslationKey> = {
  overview: 'docker.category.overview',
  container: 'docker.category.container',
  image: 'docker.category.image',
  network: 'docker.category.network',
  storage: 'docker.category.storage',
  compose: 'docker.category.compose',
  security: 'docker.category.security',
  diagnostics: 'docker.category.diagnostics',
  cleanup: 'docker.category.cleanup',
};

const DOCKER_ACTIONS: DockerAction[] = [
  {
    id: 'system-overview',
    titleKey: 'docker.action.systemOverview.title',
    descriptionKey: 'docker.action.systemOverview.desc',
    command: 'docker version && echo && docker context ls && echo && docker system df && echo && docker info',
    category: 'overview',
    risk: 'safe',
  },
  {
    id: 'container-list',
    titleKey: 'docker.action.containerList.title',
    descriptionKey: 'docker.action.containerList.desc',
    command: 'docker ps -a --format "table {{.ID}}\\t{{.Names}}\\t{{.Image}}\\t{{.Status}}\\t{{.Ports}}"',
    category: 'container',
    risk: 'safe',
  },
  {
    id: 'container-stats',
    titleKey: 'docker.action.containerStats.title',
    descriptionKey: 'docker.action.containerStats.desc',
    command: 'docker stats --no-stream --all',
    category: 'container',
    risk: 'safe',
  },
  {
    id: 'container-logs',
    titleKey: 'docker.action.containerLogs.title',
    descriptionKey: 'docker.action.containerLogs.desc',
    command: 'docker logs --tail 200 -f <container_name_or_id>',
    category: 'container',
    risk: 'safe',
    mode: 'insert',
  },
  {
    id: 'container-inspect',
    titleKey: 'docker.action.containerInspect.title',
    descriptionKey: 'docker.action.containerInspect.desc',
    command: 'docker inspect <container_name_or_id>',
    category: 'container',
    risk: 'safe',
    mode: 'insert',
  },
  {
    id: 'container-processes',
    titleKey: 'docker.action.containerProcesses.title',
    descriptionKey: 'docker.action.containerProcesses.desc',
    command: 'docker top <container_name_or_id>',
    category: 'container',
    risk: 'safe',
    mode: 'insert',
  },
  {
    id: 'container-exec',
    titleKey: 'docker.action.containerExec.title',
    descriptionKey: 'docker.action.containerExec.desc',
    command: 'docker exec -it <container_name_or_id> /bin/sh',
    category: 'container',
    risk: 'change',
    mode: 'insert',
  },
  {
    id: 'container-restart',
    titleKey: 'docker.action.containerRestart.title',
    descriptionKey: 'docker.action.containerRestart.desc',
    command: 'docker restart <container_name_or_id>',
    category: 'container',
    risk: 'change',
    mode: 'insert',
  },
  {
    id: 'image-list',
    titleKey: 'docker.action.imageList.title',
    descriptionKey: 'docker.action.imageList.desc',
    command: 'docker images --digests',
    category: 'image',
    risk: 'safe',
  },
  {
    id: 'image-history',
    titleKey: 'docker.action.imageHistory.title',
    descriptionKey: 'docker.action.imageHistory.desc',
    command: 'docker history <image_name_or_id>',
    category: 'image',
    risk: 'safe',
    mode: 'insert',
  },
  {
    id: 'image-prune',
    titleKey: 'docker.action.imagePrune.title',
    descriptionKey: 'docker.action.imagePrune.desc',
    command: 'docker image prune',
    category: 'image',
    risk: 'danger',
    mode: 'insert',
  },
  {
    id: 'network-list',
    titleKey: 'docker.action.networkList.title',
    descriptionKey: 'docker.action.networkList.desc',
    command: 'docker network ls && echo && docker network inspect bridge',
    category: 'network',
    risk: 'safe',
  },
  {
    id: 'network-inspect',
    titleKey: 'docker.action.networkInspect.title',
    descriptionKey: 'docker.action.networkInspect.desc',
    command: 'docker network inspect <network_name_or_id>',
    category: 'network',
    risk: 'safe',
    mode: 'insert',
  },
  {
    id: 'volume-list',
    titleKey: 'docker.action.volumeList.title',
    descriptionKey: 'docker.action.volumeList.desc',
    command: 'docker volume ls && echo && docker system df -v',
    category: 'storage',
    risk: 'safe',
  },
  {
    id: 'volume-inspect',
    titleKey: 'docker.action.volumeInspect.title',
    descriptionKey: 'docker.action.volumeInspect.desc',
    command: 'docker volume inspect <volume_name>',
    category: 'storage',
    risk: 'safe',
    mode: 'insert',
  },
  {
    id: 'compose-status',
    titleKey: 'docker.action.composeStatus.title',
    descriptionKey: 'docker.action.composeStatus.desc',
    command: 'docker compose ps',
    category: 'compose',
    risk: 'safe',
    mode: 'insert',
  },
  {
    id: 'compose-logs',
    titleKey: 'docker.action.composeLogs.title',
    descriptionKey: 'docker.action.composeLogs.desc',
    command: 'docker compose logs --tail 200 -f',
    category: 'compose',
    risk: 'safe',
    mode: 'insert',
  },
  {
    id: 'compose-config',
    titleKey: 'docker.action.composeConfig.title',
    descriptionKey: 'docker.action.composeConfig.desc',
    command: 'docker compose config',
    category: 'compose',
    risk: 'safe',
    mode: 'insert',
  },
  {
    id: 'compose-pull',
    titleKey: 'docker.action.composePull.title',
    descriptionKey: 'docker.action.composePull.desc',
    command: 'docker compose pull',
    category: 'compose',
    risk: 'change',
    mode: 'insert',
  },
  {
    id: 'compose-restart',
    titleKey: 'docker.action.composeRestart.title',
    descriptionKey: 'docker.action.composeRestart.desc',
    command: 'docker compose restart',
    category: 'compose',
    risk: 'change',
    mode: 'insert',
  },
  {
    id: 'security-audit',
    titleKey: 'docker.action.securityAudit.title',
    descriptionKey: 'docker.action.securityAudit.desc',
    command: `docker ps -a --format '{{.ID}} {{.Names}}' | while read id name; do echo "===== $name ($id) ====="; docker inspect "$id" --format 'Privileged={{.HostConfig.Privileged}} Restart={{.HostConfig.RestartPolicy.Name}} Network={{.HostConfig.NetworkMode}} Mounts={{range .Mounts}}{{.Source}}:{{.Destination}} {{end}} CapAdd={{.HostConfig.CapAdd}} Ports={{json .NetworkSettings.Ports}}'; done`,
    category: 'security',
    risk: 'safe',
  },
  {
    id: 'docker-events',
    titleKey: 'docker.action.events.title',
    descriptionKey: 'docker.action.events.desc',
    command: 'docker events --since 30m',
    category: 'diagnostics',
    risk: 'safe',
    mode: 'insert',
  },
  {
    id: 'daemon-logs',
    titleKey: 'docker.action.daemonLogs.title',
    descriptionKey: 'docker.action.daemonLogs.desc',
    command: 'journalctl -u docker --since "1 hour ago" --no-pager | tail -n 200',
    category: 'diagnostics',
    risk: 'safe',
  },
  {
    id: 'healthcheck',
    titleKey: 'docker.action.healthcheck.title',
    descriptionKey: 'docker.action.healthcheck.desc',
    command: `docker ps -a --format '{{.ID}}\t{{.Names}}\t{{.Status}}' | awk 'BEGIN{print "ID\tNAME\tSTATUS"} /unhealthy|Exited|Restarting|Dead/ {print}'`,
    category: 'diagnostics',
    risk: 'safe',
  },
  {
    id: 'prune-dry-run',
    titleKey: 'docker.action.pruneDryRun.title',
    descriptionKey: 'docker.action.pruneDryRun.desc',
    command: 'docker system df -v',
    category: 'cleanup',
    risk: 'safe',
  },
  {
    id: 'prune-system',
    titleKey: 'docker.action.pruneSystem.title',
    descriptionKey: 'docker.action.pruneSystem.desc',
    command: 'docker system prune',
    category: 'cleanup',
    risk: 'danger',
    mode: 'insert',
  },
];

function getCategories(actions: DockerAction[]): DockerCategoryFilter[] {
  return ['all', ...Array.from(new Set(actions.map((action) => action.category)))];
}

function isRunAction(action: DockerAction) {
  return action.mode !== 'insert' && action.risk === 'safe' && !action.command.includes('<');
}

function DockerInlinePanel({ executeCommand, insertCommand }: DockerInlinePanelProps) {
  const { t, tText } = useTranslation();
  const [category, setCategory] = useState<DockerCategoryFilter>('all');
  const [runningId, setRunningId] = useState<string | null>(null);
  const categories = useMemo(() => getCategories(DOCKER_ACTIONS), []);

  const filteredActions = useMemo(() => {
    return DOCKER_ACTIONS.filter((action) => category === 'all' || action.category === category);
  }, [category]);

  const handleAction = async (action: DockerAction) => {
    if (isRunAction(action) && executeCommand) {
      setRunningId(action.id);
      try {
        await executeCommand(action.command, { timeoutMs: 30000 });
      } catch (error) {
        message.error(tText('docker.executeFailed', { error: error instanceof Error ? error.message : String(error) }));
      } finally {
        setRunningId(null);
      }
      return;
    }

    if (!insertCommand) {
      message.warning(tText('docker.terminalNotReady'));
      return;
    }
    await insertCommand(action.command);
    message.success(tText('docker.commandInserted'));
  };

  const renderAction = (action: DockerAction) => {
    const runAction = isRunAction(action);

    return (
      <div key={action.id} className="inline-panel-item docker-panel-inline-item">
        <div className="inline-panel-item-header">
          <span className="inline-panel-item-name">{t(action.titleKey)}</span>
          <div className="inline-panel-item-actions">
            <Tag color="blue">{t(CATEGORY_LABEL_KEYS[action.category])}</Tag>
            <Tag color={RISK_COLORS[action.risk]}>{t(RISK_LABEL_KEYS[action.risk])}</Tag>
            <Tooltip title={runAction ? tText('docker.tooltip.execute') : tText('docker.tooltip.insert')}>
              <button
                className={`inline-panel-icon-btn inline-panel-icon-btn-run ${action.risk === 'danger' ? 'docker-panel-danger-action' : ''}`}
                onClick={() => void handleAction(action)}
                disabled={runningId === action.id}
              >
                <PlayCircleOutlined spin={runningId === action.id} />
              </button>
            </Tooltip>
          </div>
        </div>
        <div className="inline-panel-item-desc">{t(action.descriptionKey)}</div>
        <code className="inline-panel-item-code">{action.command}</code>
      </div>
    );
  };

  return (
    <div className="inline-panel-shell">
      <div className="inline-panel-toolbar docker-panel-toolbar-compact">
        <div className="inline-panel-categories">
          {categories.map((item) => (
            <button
              key={item}
              type="button"
              className={`inline-panel-cat-btn ${category === item ? 'inline-panel-cat-btn-active' : ''}`}
              onClick={() => setCategory(item)}
            >
              {item === 'all' ? t('inlinePanel.all') : t(CATEGORY_LABEL_KEYS[item])}
            </button>
          ))}
        </div>
      </div>

      <div className="inline-panel-list">
        {filteredActions.length === 0 ? (
          <Empty description={t('docker.noItems')} image={Empty.PRESENTED_IMAGE_SIMPLE} />
        ) : filteredActions.map((action) => renderAction(action))}
      </div>
    </div>
  );
}

export default memo(DockerInlinePanel);
