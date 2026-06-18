import {
  memo,
  useCallback,
  useEffect,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from 'react';
import SftpPanel from '../../components/SftpPanel';
import AiChatPanel from '../../components/AiChatPanel';
import AiInlinePanel from '../../components/AiInlinePanel';
import PortForwardPanel from '../../components/PortForwardPanel';
import CommandsInlinePanel from '../../components/CommandsInlinePanel';
import ScriptsInlinePanel from '../../components/ScriptsInlinePanel';
import DockerInlinePanel from '../../components/DockerInlinePanel';
import { useAiChatStore } from '../../stores/aiChat';
import { useTranslation } from '../../i18n';
import type { TerminalController } from '../../components/TerminalView';
import '../../styles/panels/bottom-panel.css';

interface BottomPanelProps {
  hostId?: string;
  hostName?: string;
  hostIp?: string;
  sessionId?: string;
  isRemote: boolean;
  dockerAvailable?: boolean;
  getTerminalBuffer: () => string;
  executeTerminalCommand: (command: string, options?: Parameters<TerminalController['executeCommand']>[1]) => ReturnType<TerminalController['executeCommand']> | undefined;
  insertTerminalCommand: (command: string) => ReturnType<TerminalController['insertCommand']> | undefined;
}

const DEFAULT_BOTTOM_PANEL_HEIGHT = 292;
const MIN_BOTTOM_PANEL_HEIGHT = 36;
const MIN_TERMINAL_BODY_HEIGHT = 180;

function clampBottomPanelHeight(nextHeight: number, containerHeight?: number) {
  const maxHeight = containerHeight
    ? Math.max(MIN_BOTTOM_PANEL_HEIGHT, containerHeight - MIN_TERMINAL_BODY_HEIGHT)
    : 520;

  return Math.max(MIN_BOTTOM_PANEL_HEIGHT, Math.min(maxHeight, nextHeight));
}

const BottomPanel = memo(function BottomPanel({
  hostId,
  hostName,
  hostIp,
  sessionId,
  isRemote,
  dockerAvailable,
  getTerminalBuffer,
  executeTerminalCommand,
  insertTerminalCommand,
}: BottomPanelProps) {
  const { t, tText } = useTranslation();
  const bottomTab = useAiChatStore((s) => s.bottomTab);
  const setBottomTab = useAiChatStore((s) => s.setBottomTab);
  const inlineVisible = useAiChatStore((s) => s.inlineVisible);
  const setInlineVisible = useAiChatStore((s) => s.setInlineVisible);
  const [panelHeight, setPanelHeight] = useState(DEFAULT_BOTTOM_PANEL_HEIGHT);
  const shellRef = useRef<HTMLDivElement>(null);
  const resizeCleanupRef = useRef<(() => void) | null>(null);

  const effectiveTab = (!isRemote && bottomTab !== 'ai' && bottomTab !== 'commands' && bottomTab !== 'scripts')
    || (bottomTab === 'docker' && !dockerAvailable)
    ? 'ai'
    : bottomTab;

  useEffect(() => () => {
    resizeCleanupRef.current?.();
  }, []);

  useEffect(() => {
    const parent = shellRef.current?.parentElement;
    if (!parent) return undefined;

    const clampToContainer = (containerHeight: number) => {
      if (containerHeight <= 0) return;
      setPanelHeight((currentHeight) => clampBottomPanelHeight(currentHeight, containerHeight));
    };

    clampToContainer(parent.clientHeight);

    const resizeObserver = new ResizeObserver((entries) => {
      const entry = entries[0];
      clampToContainer(entry?.contentRect.height ?? parent.clientHeight);
    });
    resizeObserver.observe(parent);

    return () => resizeObserver.disconnect();
  }, []);

  const handleResizePointerDown = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    resizeCleanupRef.current?.();

    const startY = event.clientY;
    const startHeight = panelHeight;
    const containerHeight = shellRef.current?.parentElement?.clientHeight;

    const cleanup = () => {
      window.removeEventListener('pointermove', handlePointerMove);
      window.removeEventListener('pointerup', handlePointerUp);
      window.removeEventListener('pointercancel', handlePointerUp);
      document.body.classList.remove('bottom-panel-resizing');
      resizeCleanupRef.current = null;
    };

    const handlePointerMove = (moveEvent: PointerEvent) => {
      const deltaY = startY - moveEvent.clientY;
      setPanelHeight(clampBottomPanelHeight(startHeight + deltaY, containerHeight));
    };

    const handlePointerUp = () => {
      cleanup();
    };

    document.body.classList.add('bottom-panel-resizing');
    resizeCleanupRef.current = cleanup;
    window.addEventListener('pointermove', handlePointerMove);
    window.addEventListener('pointerup', handlePointerUp);
    window.addEventListener('pointercancel', handlePointerUp);
  }, [panelHeight]);

  return (
    <>
      <div ref={shellRef} className="bottom-panel-shell" style={{ height: panelHeight }}>
        <div
          className="bottom-panel-resize-handle"
          role="separator"
          aria-orientation="horizontal"
          aria-label={tText('terminal.bottomPanelResizeAria')}
          onPointerDown={handleResizePointerDown}
        />
        <div className="bottom-panel-tab-bar">
          {isRemote && (
            <>
              <button
                className={`sftp-tab-btn ${effectiveTab === 'sftp' ? 'sftp-tab-btn-active' : ''}`}
                onClick={() => setBottomTab('sftp')}
              >
                SFTP
              </button>
              <button
                className={`sftp-tab-btn ${effectiveTab === 'forward' ? 'sftp-tab-btn-active' : ''}`}
                onClick={() => setBottomTab('forward')}
              >
                {t('terminal.tab.portForward')}
              </button>
            </>
          )}
          <button
            className={`sftp-tab-btn ${effectiveTab === 'ai' ? 'sftp-tab-btn-active' : ''}`}
            onClick={() => setBottomTab('ai')}
          >
            AI
          </button>
          <button
            className={`sftp-tab-btn ${effectiveTab === 'commands' ? 'sftp-tab-btn-active' : ''}`}
            onClick={() => setBottomTab('commands')}
          >
            {t('terminal.tab.commands')}
          </button>
          <button
            className={`sftp-tab-btn ${effectiveTab === 'scripts' ? 'sftp-tab-btn-active' : ''}`}
            onClick={() => setBottomTab('scripts')}
          >
            {t('terminal.tab.scripts')}
          </button>
          {isRemote && dockerAvailable && (
            <button
              className={`sftp-tab-btn ${effectiveTab === 'docker' ? 'sftp-tab-btn-active' : ''}`}
              onClick={() => setBottomTab('docker')}
            >
              Docker
            </button>
          )}
          <button
            className="ai-inline-trigger"
            onClick={() => setInlineVisible(!inlineVisible)}
            title={tText('terminal.quickCommandTitle')}
          >
            ⌘I
          </button>
        </div>

        <div className="bottom-panel-content">
          {effectiveTab === 'ai' ? (
            <div className="bottom-panel-pane bottom-panel-pane-active">
              <AiChatPanel
                hostId={hostId}
                hostName={hostName}
                hostIp={hostIp}
                sessionId={sessionId}
                getTerminalBuffer={getTerminalBuffer}
                executeTerminalCommand={executeTerminalCommand}
              />
            </div>
          ) : null}
          {isRemote && hostId && effectiveTab === 'sftp' ? (
            <div className="bottom-panel-pane bottom-panel-pane-active">
              <SftpPanel hostId={hostId} hideTabBar forceTab="sftp" />
            </div>
          ) : null}
          {isRemote && hostId && effectiveTab === 'forward' ? (
            <div className="bottom-panel-pane bottom-panel-pane-active">
              <PortForwardPanel hostId={hostId} />
            </div>
          ) : null}
          {effectiveTab === 'commands' ? (
            <div className="bottom-panel-pane bottom-panel-pane-active">
              <CommandsInlinePanel
                insertCommand={sessionId ? (command) => insertTerminalCommand(command) : undefined}
              />
            </div>
          ) : null}
          {effectiveTab === 'scripts' ? (
            <div className="bottom-panel-pane bottom-panel-pane-active">
              <ScriptsInlinePanel
                insertCommand={sessionId ? (command) => insertTerminalCommand(command) : undefined}
              />
            </div>
          ) : null}
          {effectiveTab === 'docker' ? (
            <div className="bottom-panel-pane bottom-panel-pane-active">
              <DockerInlinePanel
                executeCommand={sessionId ? (command, options) => executeTerminalCommand(command, options) : undefined}
                insertCommand={sessionId ? (command) => insertTerminalCommand(command) : undefined}
              />
            </div>
          ) : null}
        </div>
      </div>

      {inlineVisible && (
        <AiInlinePanel
          sessionId={sessionId}
          hostId={hostId}
          visible={inlineVisible}
          onClose={() => setInlineVisible(false)}
          terminalBuffer={getTerminalBuffer()}
        />
      )}
    </>
  );
});

export default BottomPanel;
