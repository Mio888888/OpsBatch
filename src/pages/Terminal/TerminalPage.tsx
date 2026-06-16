import { useState, useEffect, useCallback, useRef, memo, useMemo, startTransition, useDeferredValue, type MouseEvent, type PointerEvent as ReactPointerEvent } from 'react';
import { useSearchParams, useNavigate, useLocation } from 'react-router-dom';
import { Tabs, Empty, Button, Spin } from '../../components/ui';
import { PlayCircleOutlined, PlusOutlined } from '../../components/ui/icons';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import TerminalView, { type TerminalController } from '../../components/TerminalView';
import { useAssetsStore } from '../../stores/assets';
import { useAiChatStore } from '../../stores/aiChat';
import { useTranslation } from '../../i18n';
import type { HostMonitorNetwork, HostMonitorSnapshot } from '../../types';
import { isHostMonitorIdle, shouldPollHostMonitor } from '../../utils/hostMonitor';
import { getTerminalTabCloseTargets, type TerminalTabCloseMode } from '../../utils/terminalTabs';
import { logHandledError } from '../../utils/globalLogger';
import BottomPanel from './BottomPanel';
import {
  SplitContextMenu,
  TabContextMenu,
  type TerminalSplitContextMenu,
  type TerminalTabContextMenu,
} from './TerminalContextMenus';

interface TerminalTab {
  key: string;
  sessionId?: string;
  splitSessionId?: string;
  hostId?: string;
  hostName: string;
  hostIp: string;
  kind: 'local' | 'remote';
  state: 'connecting' | 'connected' | 'error';
  splitState?: 'connecting' | 'connected' | 'error';
  splitErrorMessage?: string;
  errorMessage?: string;
  connectionId?: string;
  splitConnectionId?: string;
  auxiliaryReady?: boolean;
}

interface OpenHostRequest {
  requestId: string;
  hostId: string;
  name: string;
  ip: string;
}

interface TerminalBufferApi extends TerminalController {}

interface RemoteHostConnection {  hostId: string;
  name: string;
  ip: string;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object';
}

function isOpenHostRequest(value: unknown): value is OpenHostRequest {
  if (!isRecord(value)) return false;

  return (
    typeof value.requestId === 'string'
    && typeof value.hostId === 'string'
    && typeof value.name === 'string'
    && typeof value.ip === 'string'
  );
}

function getOpenHostRequest(state: unknown): OpenHostRequest | undefined {
  if (!isRecord(state)) return undefined;

  return isOpenHostRequest(state.openHost) ? state.openHost : undefined;
}

function getQueryHostConnection(searchParams: URLSearchParams, defaultName: string): RemoteHostConnection | undefined {
  const hostId = searchParams.get('hostId');
  if (!hostId) return undefined;

  return {
    hostId,
    name: searchParams.get('name') || defaultName,
    ip: searchParams.get('ip') || '',
  };
}

function createPendingTabKey(kind: TerminalTab['kind'], id = 'local') {
  return `${kind}-${id}-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

function createConnectionId(tabKey: string) {
  return `${tabKey}-attempt-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

function getErrorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function getTabStateLabel(state: TerminalTab['state'], labels: Record<TerminalTab['state'], string>) {
  return labels[state];
}

async function disconnectTerminalSession(sessionId: string, hostId?: string) {
  try {
    await invoke('terminal_disconnect', { sessionId, hostId });
  } catch {
    // 忽略断开连接时的错误
  }
}

function formatPercent(value?: number) {
  return typeof value === 'number' && Number.isFinite(value) ? Math.max(0, Math.min(100, value)) : undefined;
}

function formatBytesPerSecond(value: number) {
  if (value >= 1024 * 1024) return `${(value / 1024 / 1024).toFixed(1)}M`;
  if (value >= 1024) return `${Math.round(value / 1024)}K`;
  return `${value}B`;
}

function formatMemory(used?: number, total?: number) {
  if (typeof used !== 'number' || typeof total !== 'number') return '--';
  return `${(used / 1024).toFixed(1)}G/${(total / 1024).toFixed(1)}G`;
}

function getSnapshotNetworks(snapshot?: HostMonitorSnapshot | null) {
  const networks = snapshot?.networks?.length ? snapshot.networks : (snapshot?.network ? [snapshot.network] : []);
  const seen = new Set<string>();

  return networks.filter((network) => {
    if (!network.interface || seen.has(network.interface)) return false;
    seen.add(network.interface);
    return true;
  });
}

function getNetworkByInterface(snapshot: HostMonitorSnapshot | undefined, interfaceName: string) {
  if (!interfaceName) return undefined;
  return getSnapshotNetworks(snapshot).find((network) => network.interface === interfaceName);
}

function getSelectedNetwork(snapshot: HostMonitorSnapshot | undefined, selectedInterface: string) {
  const networks = getSnapshotNetworks(snapshot);
  if (networks.length === 0) return undefined;
  return networks.find((network) => network.interface === selectedInterface) ?? networks[0];
}

function getNetworkRate(current?: HostMonitorNetwork, previous?: HostMonitorNetwork, elapsedSeconds = 0) {
  if (!current || !previous || current.interface !== previous.interface || elapsedSeconds <= 0) return { rx: 0, tx: 0 };

  return {
    rx: Math.max(0, Math.round((current.rxBytes - previous.rxBytes) / elapsedSeconds)),
    tx: Math.max(0, Math.round((current.txBytes - previous.txBytes) / elapsedSeconds)),
  };
}

const DEFAULT_MONITOR_PANEL_WIDTH = 280;
const MAX_MONITOR_PANEL_WIDTH = DEFAULT_MONITOR_PANEL_WIDTH;
const MIN_MONITOR_PANEL_WIDTH = 8;
const MIN_TERMINAL_MAIN_WIDTH = 420;

function clampMonitorPanelWidth(nextWidth: number, containerWidth?: number) {
  const layoutMaxWidth = containerWidth
    ? Math.max(MIN_MONITOR_PANEL_WIDTH, containerWidth - MIN_TERMINAL_MAIN_WIDTH)
    : MAX_MONITOR_PANEL_WIDTH;
  const maxWidth = Math.min(MAX_MONITOR_PANEL_WIDTH, layoutMaxWidth);

  return Math.max(MIN_MONITOR_PANEL_WIDTH, Math.min(maxWidth, nextWidth));
}

interface HostMonitorPanelProps {
  hostId: string;
  hostIp: string;
  width: number;
  onResizePointerDown: (event: ReactPointerEvent<HTMLDivElement>) => void;
}

const HostMonitorPanel = memo(function HostMonitorPanel({ hostId, hostIp, width, onResizePointerDown }: HostMonitorPanelProps) {
  const { t, tText } = useTranslation();
  const getHostMonitorSnapshot = useAssetsStore((state) => state.getHostMonitorSnapshot);
  const [snapshot, setSnapshot] = useState<HostMonitorSnapshot | null>(null);
  const [history, setHistory] = useState<HostMonitorSnapshot[]>([]);
  const [selectedInterface, setSelectedInterface] = useState('');
  const [error, setError] = useState('');
  const [realtimeEnabled, setRealtimeEnabled] = useState(false);

  useEffect(() => {
    setSnapshot(null);
    setHistory([]);
    setSelectedInterface('');
    setError('');
    setRealtimeEnabled(false);
  }, [hostId]);

  useEffect(() => {
    let disposed = false;
    let requestSequence = 0;
    let inFlight = false;
    let timer: number | undefined;

    const loadSnapshot = async () => {
      if (inFlight) return;
      inFlight = true;
      const requestId = ++requestSequence;
      try {
        const next = await getHostMonitorSnapshot(hostId);
        if (disposed || requestId !== requestSequence) return;
        setSnapshot(next);
        setHistory((prev) => [...prev, next].slice(-28));
        setError('');
      } catch (e: unknown) {
        if (disposed || requestId !== requestSequence) return;
        setError(getErrorMessage(e));
      } finally {
        if (requestId === requestSequence) {
          inFlight = false;
        }
      }
    };

    if (shouldPollHostMonitor({ realtimeEnabled, hasSnapshot: false })) {
      void loadSnapshot();
      timer = window.setInterval(() => {
        void loadSnapshot();
      }, 3000);
    }

    return () => {
      disposed = true;
      requestSequence += 1;
      if (timer !== undefined) window.clearInterval(timer);
    };
  }, [getHostMonitorSnapshot, hostId, realtimeEnabled]);

  useEffect(() => {
    const nextInterface = getSelectedNetwork(snapshot ?? undefined, selectedInterface)?.interface || '';
    if (nextInterface !== selectedInterface) {
      setSelectedInterface(nextInterface);
    }
  }, [snapshot, selectedInterface]);

  const previousSnapshot = history.length > 1 ? history[history.length - 2] : undefined;
  const isLoadingSnapshot = !snapshot && !error;
  const networkOptions = getSnapshotNetworks(snapshot);
  const isIdle = isHostMonitorIdle({ realtimeEnabled, hasSnapshot: Boolean(snapshot) });
  const selectedNetwork = getSelectedNetwork(snapshot ?? undefined, selectedInterface);
  const activeInterface = selectedNetwork?.interface || '';
  const previousNetwork = getNetworkByInterface(previousSnapshot, activeInterface);
  const elapsedSeconds = snapshot && previousSnapshot
    ? Math.max(1, (snapshot.timestamp - previousSnapshot.timestamp) / 1000)
    : 0;
  const { rx: rxRate, tx: txRate } = getNetworkRate(selectedNetwork, previousNetwork, elapsedSeconds);
  const cpuPercent = (() => {
    // 使用相邻两次 snapshot 的 /proc/stat 累积值计算 CPU 使用率
    if (snapshot?.cpuPercent != null) return formatPercent(snapshot.cpuPercent);
    const curUsed = snapshot?.cpuTimeUsed;
    const curTotal = snapshot?.cpuTimeTotal;
    const prevUsed = previousSnapshot?.cpuTimeUsed;
    const prevTotal = previousSnapshot?.cpuTimeTotal;
    if (curUsed != null && curTotal != null && prevUsed != null && prevTotal != null) {
      const dUsed = curUsed - prevUsed;
      const dTotal = curTotal - prevTotal;
      if (dTotal > 0) return formatPercent((dUsed / dTotal) * 100);
    }
    return undefined;
  })();
  const memoryPercent = snapshot?.memoryUsedMb && snapshot.memoryTotalMb
    ? formatPercent((snapshot.memoryUsedMb / snapshot.memoryTotalMb) * 100)
    : undefined;
 const swapPercent = snapshot?.swapUsedMb && snapshot.swapTotalMb
   ? formatPercent((snapshot.swapUsedMb / snapshot.swapTotalMb) * 100)
   : undefined;
  const networkMax = useMemo(() => {
    // reduce 取代 Math.max(...spread),避免大 history 时调用栈溢出;useMemo 避免每次渲染重算
    return history.reduce((max, item, index) => {
      const prev = history[index - 1];
      const itemNetwork = getNetworkByInterface(item, activeInterface);
      const prevNetwork = getNetworkByInterface(prev, activeInterface);
      const seconds = prev ? Math.max(1, (item.timestamp - prev.timestamp) / 1000) : 0;
      const rate = getNetworkRate(itemNetwork, prevNetwork, seconds);
      return Math.max(max, rate.rx, rate.tx);
    }, 1);
  }, [history, activeInterface]);

  return (
    <aside className="terminal-monitor-panel" style={{ width, flexBasis: width, maxWidth: MAX_MONITOR_PANEL_WIDTH }}>
      <div
        className="terminal-monitor-resize-handle"
        role="separator"
        aria-orientation="vertical"
        aria-label={tText('terminal.monitorResizeAria')}
        onPointerDown={onResizePointerDown}
      />
      <div className="terminal-monitor-content">
        <div className="terminal-monitor-host">
          <span className="terminal-monitor-ip">IP {hostIp || '--'}</span>
          <button type="button" onClick={() => navigator.clipboard.writeText(hostIp)} disabled={!hostIp}>{t('terminal.copy')}</button>
        </div>

        <section className="terminal-monitor-section">
        <div className="terminal-monitor-title">{'─'} {t('terminal.systemInfo')} {'─'}</div>
        {isIdle ? (
          <div className="terminal-monitor-idle">
            <button
              type="button"
              className="terminal-monitor-start-button"
              onClick={() => setRealtimeEnabled(true)}
              title={tText('terminal.startRealtimeMonitor')}
            >
              <PlayCircleOutlined />
              <span>{t('terminal.startRealtimeMonitor')}</span>
            </button>
            <div className="terminal-monitor-idle-text">{t('terminal.monitorIdleHint')}</div>
          </div>
        ) : null}
        {isIdle ? null : isLoadingSnapshot ? (
          <div className="terminal-monitor-skeleton-block" aria-label={tText('terminal.loadingSystemInfo')}>
            <span className="terminal-skeleton-line terminal-skeleton-line-wide" />
            <span className="terminal-skeleton-line terminal-skeleton-line-medium" />
            <span className="terminal-skeleton-line terminal-skeleton-line-full" />
            <span className="terminal-skeleton-line terminal-skeleton-line-narrow" />
          </div>
        ) : !isIdle ? (
          <>
            <div className="terminal-monitor-line">{t('terminal.uptime', { value: snapshot?.uptime || '--' })}</div>
            <div className="terminal-monitor-line">{t('terminal.loadAverage', { value: snapshot?.loadAverage || '--' })}</div>
            <div className="terminal-monitor-muted">{snapshot?.os || t('common.noData')}</div>
            <div className="terminal-monitor-muted">{t('terminal.kernel', { value: snapshot?.kernel || '--' })}</div>
          </>
        ) : null}
        {error ? <div className="terminal-monitor-error">{error}</div> : null}
      </section>

      <section className="terminal-monitor-section terminal-monitor-bars">
        {isIdle ? (
          <div className="terminal-monitor-empty">{t('terminal.monitorNotStarted')}</div>
        ) : isLoadingSnapshot ? (
          <div className="terminal-monitor-resource-skeleton" aria-label={tText('terminal.loadingResources')}>
            <span />
            <i />
            <span />
            <i />
            <span />
            <i />
          </div>
        ) : (
          <>
            <ResourceBar label="CPU" value={cpuPercent} detail={cpuPercent === undefined ? '--' : `${cpuPercent.toFixed(0)}%`} />
            <ResourceBar label={tText('terminal.memory')} value={memoryPercent} detail={formatMemory(snapshot?.memoryUsedMb, snapshot?.memoryTotalMb)} />
            <ResourceBar label={tText('terminal.swap')} value={swapPercent} detail={formatMemory(snapshot?.swapUsedMb, snapshot?.swapTotalMb)} />
          </>
        )}
      </section>

      <section className="terminal-monitor-section">
        <div className="terminal-monitor-process-header">
          <span>{t('terminal.memory')}</span><span>CPU</span><span>{t('terminal.processCommand')}</span>
        </div>
        {isIdle ? (
          <div className="terminal-monitor-empty">{t('terminal.monitorNotStarted')}</div>
        ) : isLoadingSnapshot ? (
          <div className="terminal-monitor-table-skeleton" aria-label={tText('terminal.loadingProcesses')}>
            <span /><span /><span />
            <span /><span /><span />
            <span /><span /><span />
          </div>
        ) : (
          <>
            {(snapshot?.processes.length ? snapshot.processes : []).map((process, index) => (
              <div className="terminal-monitor-process-row" key={`${process.command}-${index}`}>
                <span>{process.memory}</span><span>{process.cpu}</span><span title={process.command}>{process.command}</span>
              </div>
            ))}
            {snapshot && snapshot.processes.length === 0 ? <div className="terminal-monitor-empty">{t('common.noData')}</div> : null}
          </>
        )}
      </section>

      <section className="terminal-monitor-section">
        <div className="terminal-monitor-network-head">
          <span>↑{formatBytesPerSecond(txRate)}</span>
          <span>↓{formatBytesPerSecond(rxRate)}</span>
          {networkOptions.length > 0 ? (
            <select
              className="terminal-monitor-network-select"
              aria-label={tText('terminal.switchNetworkCard')}
              value={activeInterface}
              onChange={(event) => setSelectedInterface(event.target.value)}
              disabled={networkOptions.length === 1}
            >
              {networkOptions.map((network) => (
                <option key={network.interface} value={network.interface}>{network.interface}</option>
              ))}
            </select>
          ) : (
            <span>--</span>
          )}
        </div>
        {isIdle ? (
          <div className="terminal-monitor-chart-empty">{t('terminal.monitorNotStarted')}</div>
        ) : isLoadingSnapshot ? (
          <div className="terminal-monitor-chart-skeleton" aria-label={tText('terminal.loadingNetworkData')}>
            {Array.from({ length: 18 }, (_, index) => <span key={index} style={{ height: `${18 + (index % 7) * 9}%` }} />)}
          </div>
        ) : history.length > 1 ? (
          <div className="terminal-monitor-chart">
            {history.map((item, index) => {
              const prev = history[index - 1];
              const itemNetwork = getNetworkByInterface(item, activeInterface);
              const prevNetwork = getNetworkByInterface(prev, activeInterface);
              const seconds = prev ? Math.max(1, (item.timestamp - prev.timestamp) / 1000) : 0;
              const rate = getNetworkRate(itemNetwork, prevNetwork, seconds);
              const height = networkMax > 0 ? Math.max(6, Math.min(100, rate.rx / networkMax * 100)) : 6;
              return <span key={item.timestamp} style={{ height: `${height}%` }} />;
            })}
          </div>
        ) : <div className="terminal-monitor-chart-empty">{t('terminal.collecting')}</div>}
      </section>

      <section className="terminal-monitor-section">
        <div className="terminal-monitor-chart-label">
          PING {snapshot?.pingMs !== undefined ? `${snapshot.pingMs.toFixed(1)}ms` : '--'}
        </div>
        {isIdle ? (
          <div className="terminal-monitor-chart-empty">{t('terminal.monitorNotStarted')}</div>
        ) : isLoadingSnapshot ? (
          <div className="terminal-monitor-chart-skeleton terminal-monitor-chart-skeleton-blue" aria-label={tText('terminal.loadingLatencyData')}>
            {Array.from({ length: 18 }, (_, index) => <span key={index} style={{ height: `${14 + (index % 5) * 10}%` }} />)}
          </div>
        ) : history.length > 0 ? (
          <div className="terminal-monitor-grid-chart">
            {history.map((item) => {
              const ping = item.pingMs;
              const height = ping !== undefined ? Math.max(3, Math.min(100, ping * 3)) : 3;
              return <span key={item.timestamp} style={{ height: `${height}%` }} />;
            })}
          </div>
        ) : <div className="terminal-monitor-chart-empty">{t('terminal.collecting')}</div>}
      </section>

        <section className="terminal-monitor-section terminal-monitor-filesystems">
          <div className="terminal-monitor-filesystem-header"><span>{t('terminal.path')}</span><span>{t('terminal.availableSize')}</span></div>
          {isIdle ? (
            <div className="terminal-monitor-empty">{t('terminal.monitorNotStarted')}</div>
          ) : isLoadingSnapshot ? (
            <div className="terminal-monitor-filesystem-skeleton" aria-label={tText('terminal.loadingDiskInfo')}>
              <span /><span />
              <span /><span />
              <span /><span />
            </div>
          ) : (
            <>
              {(snapshot?.filesystems.length ? snapshot.filesystems : []).map((filesystem) => (
                <div className="terminal-monitor-filesystem-row" key={filesystem.path}>
                  <span title={filesystem.path}>{filesystem.path}</span>
                  <span>{filesystem.available}/{filesystem.total}</span>
                </div>
              ))}
              {snapshot && snapshot.filesystems.length === 0 ? <div className="terminal-monitor-empty">{t('common.noData')}</div> : null}
            </>
          )}
        </section>
      </div>
    </aside>
  );
});
HostMonitorPanel.displayName = 'HostMonitorPanel';

const ResourceBar = memo(function ResourceBar({ label, value, detail }: { label: string; value?: number; detail: string }) {
  return (
    <div className="terminal-monitor-resource">
      <span>{label}</span>
      <div className="terminal-monitor-resource-bar">
        <i style={{ width: `${value ?? 0}%` }} />
        <strong>{detail}</strong>
      </div>
    </div>
  );
});
ResourceBar.displayName = 'ResourceBar';

interface TerminalTabContentProps {
  tab: TerminalTab;
  isActive: boolean;
  setTerminalBuffer: (key: string, api: TerminalBufferApi) => void;
  onContextMenu: (event: MouseEvent<HTMLDivElement>, tab: TerminalTab, terminalKey: string) => void;
  onCloseSplit: (tab: TerminalTab) => void;
}

const TerminalTabContent = memo(function TerminalTabContent({
  tab,
  isActive,
  setTerminalBuffer,
  onContextMenu,
  onCloseSplit,
}: TerminalTabContentProps) {
  const { t, tText } = useTranslation();
  const sessionId = tab.sessionId || '';
  const handleTerminalReady = useCallback((api: TerminalBufferApi) => {
    setTerminalBuffer(tab.key, api);
  }, [setTerminalBuffer, tab.key]);
  const handleSplitTerminalReady = useCallback((api: TerminalBufferApi) => {
    setTerminalBuffer(`${tab.key}:split`, api);
  }, [setTerminalBuffer, tab.key]);

  if (!tab.sessionId) return null;

  if (tab.splitSessionId || tab.splitState) {
    return (
      <div className="terminal-split-layout">
        <div className="terminal-split-pane" onContextMenu={(event) => onContextMenu(event, tab, tab.key)}>
          <TerminalView
            sessionId={sessionId}
            active={isActive}
            onTerminalReady={handleTerminalReady}
          />
        </div>
        <div className="terminal-split-divider" aria-hidden="true" />
        <div className="terminal-split-pane terminal-split-pane-secondary" onContextMenu={(event) => onContextMenu(event, tab, `${tab.key}:split`)}>
          {tab.splitSessionId && tab.splitState === 'connected' ? (
            <TerminalView
              sessionId={tab.splitSessionId}
              active={isActive}
              onTerminalReady={handleSplitTerminalReady}
            />
          ) : (
            <div className="terminal-split-state" role={tab.splitState === 'error' ? 'alert' : 'status'}>
              <div className={`terminal-status-light terminal-status-light-${tab.splitState === 'error' ? 'error' : 'connecting'}`} aria-hidden="true" />
              <span>{tab.splitState === 'error' ? (tab.splitErrorMessage || t('terminal.splitConnectFailed')) : t('terminal.openingSplit')}</span>
              {tab.splitState === 'error' ? <Button size="small" onClick={() => onCloseSplit(tab)}>{t('terminal.closeSplit')}</Button> : null}
            </div>
          )}
          {tab.splitSessionId ? (
            <button type="button" className="terminal-split-close" onClick={() => onCloseSplit(tab)} title={tText('terminal.closeSplit')}>
              ×
            </button>
          ) : null}
        </div>
      </div>
    );
  }

  return (
    <div className="terminal-single-pane" onContextMenu={(event) => onContextMenu(event, tab, tab.key)}>
      <TerminalView
        sessionId={sessionId}
        active={isActive}
        onTerminalReady={handleTerminalReady}
      />
    </div>
  );
});

interface TerminalPageProps {
  visible: boolean;
}

export default function TerminalPage({ visible }: TerminalPageProps) {
  const [searchParams] = useSearchParams();
  const navigate = useNavigate();
  const location = useLocation();
  const { t, tText } = useTranslation();
  const [tabs, setTabs] = useState<TerminalTab[]>([]);
  const [activeKey, setActiveKey] = useState<string>('');
  const [splitContextMenu, setSplitContextMenu] = useState<TerminalSplitContextMenu | null>(null);
  const [tabContextMenu, setTabContextMenu] = useState<TerminalTabContextMenu | null>(null);
  const [monitorWidth, setMonitorWidth] = useState(DEFAULT_MONITOR_PANEL_WIDTH);
  const terminalPageShellRef = useRef<HTMLDivElement>(null);
  const monitorResizeCleanupRef = useRef<(() => void) | null>(null);
  const autoConnectedHostIdRef = useRef<string | null>(null);
  const consumedOpenHostRequestIdsRef = useRef<Set<string>>(new Set());
  const lastRemoteConnectionRef = useRef<RemoteHostConnection | null>(null);
  const autoConnectedLocalRef = useRef(false);
  const connectDisposedRef = useRef(false);
  const tabsRef = useRef<TerminalTab[]>([]);
  const activeKeyRef = useRef<string>('');
  const activeConnectionIdsRef = useRef<Map<string, string>>(new Map());
  const setInlineVisible = useAiChatStore((s) => s.setInlineVisible);
  const terminalBufferRefs = useRef<Map<string, TerminalBufferApi>>(new Map());
  const setTerminalBuffer = useCallback((key: string, api: TerminalBufferApi) => {
    terminalBufferRefs.current.set(key, api);
  }, []);
  const getTerminalBuffer = useCallback((key: string) => terminalBufferRefs.current.get(key)?.getBuffer() || '', []);
  const executeTerminalCommand = useCallback((key: string, command: string, options?: Parameters<TerminalController['executeCommand']>[1]) => terminalBufferRefs.current.get(key)?.executeCommand(command, options), []);
  const insertTerminalCommand = useCallback((key: string, command: string) => terminalBufferRefs.current.get(key)?.insertCommand(command), []);
  const pasteTerminalText = useCallback((key: string, text: string) => terminalBufferRefs.current.get(key)?.pasteText(text), []);

  // ⌘I global shortcut for inline AI panel
  useEffect(() => {
    if (!visible) return;
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'i') {
        e.preventDefault();
        setInlineVisible(true);
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [visible, setInlineVisible]);

  useEffect(() => {
    tabsRef.current = tabs;
  }, [tabs]);

  useEffect(() => () => {
    monitorResizeCleanupRef.current?.();
  }, []);

  useEffect(() => {
    activeKeyRef.current = activeKey;
  }, [activeKey]);

  useEffect(() => {
    if (!splitContextMenu) return;
    const close = () => setSplitContextMenu(null);
    window.addEventListener('click', close);
    window.addEventListener('blur', close);
    return () => {
      window.removeEventListener('click', close);
      window.removeEventListener('blur', close);
    };
  }, [splitContextMenu]);

  useEffect(() => {
    if (!tabContextMenu) return;
    const close = () => setTabContextMenu(null);
    window.addEventListener('click', close);
    window.addEventListener('blur', close);
    window.addEventListener('scroll', close, true);
    return () => {
      window.removeEventListener('click', close);
      window.removeEventListener('blur', close);
      window.removeEventListener('scroll', close, true);
    };
  }, [tabContextMenu]);

  const connectRemoteTab = useCallback(async (tabKey: string, hostId: string, hostName = tText('terminal.defaultName'), hostIp = '') => {
    const connectionId = createConnectionId(tabKey);
    const previousTab = tabsRef.current.find((tab) => tab.key === tabKey);

    if (previousTab?.sessionId) {
      void disconnectTerminalSession(previousTab.sessionId, previousTab.hostId);
    }

    activeConnectionIdsRef.current.set(tabKey, connectionId);
    setTabs((prev) => prev.map((tab) => (
      tab.key === tabKey
        ? {
          ...tab,
          sessionId: undefined,
          state: 'connecting' as const,
          errorMessage: undefined,
          connectionId,
          auxiliaryReady: false,
        }
        : tab
    )));
    setActiveKey(tabKey);

    try {
      const resultSessionId = await invoke<string>('terminal_connect', {
        hostId,
        cols: 80,
        rows: 24,
      });

      if (connectDisposedRef.current || activeConnectionIdsRef.current.get(tabKey) !== connectionId) {
        await disconnectTerminalSession(resultSessionId, hostId);
        return;
      }

      activeConnectionIdsRef.current.delete(tabKey);
      setTabs((prev) => prev.map((tab) => (
        tab.key === tabKey
          ? {
            ...tab,
            sessionId: resultSessionId,
            hostId,
            hostName,
            hostIp,
            kind: 'remote' as const,
            state: 'connected' as const,
            errorMessage: undefined,
            connectionId: undefined,
            auxiliaryReady: false,
          }
          : tab
      )));
      setActiveKey(tabKey);

      // SFTP warmup runs independently after terminal is interactive.
      // These are best-effort and must not gate terminal/AI/port-forward readiness.
      void invoke('sftp_warmup', { hostId }).catch((error) => {
        void logHandledError('terminal.sftpWarmup', error, 'warn');
      });
    } catch (e: unknown) {
      if (connectDisposedRef.current || activeConnectionIdsRef.current.get(tabKey) !== connectionId) {
        return;
      }

      activeConnectionIdsRef.current.delete(tabKey);
      setTabs((prev) => prev.map((tab) => (
        tab.key === tabKey
          ? {
            ...tab,
            sessionId: undefined,
            state: 'error' as const,
            errorMessage: getErrorMessage(e),
            connectionId: undefined,
            auxiliaryReady: false,
          }
          : tab
      )));
      setActiveKey(tabKey);
    }
  }, [tText]);

  const connectHost = useCallback((hostId: string, hostName = tText('terminal.defaultName'), hostIp = '') => {
    const existing = tabsRef.current.find(
      (t) => t.hostId === hostId && (t.state === 'connecting' || t.state === 'connected'),
    );
    if (existing) {
      setActiveKey(existing.key);
      return false;
    }

    const tabKey = createPendingTabKey('remote', hostId);
    const tab: TerminalTab = {
      key: tabKey,
      hostId,
      hostName,
      hostIp,
      kind: 'remote',
      state: 'connecting',
      auxiliaryReady: false,
    };

    setTabs((prev) => [...prev, tab]);
    setActiveKey(tabKey);

    window.setTimeout(() => {
      void connectRemoteTab(tabKey, hostId, hostName, hostIp);
    }, 0);
    return true;
  }, [connectRemoteTab, tText]);

  const connectLocalTab = useCallback(async (tabKey: string) => {
    const connectionId = createConnectionId(tabKey);
    const previousTab = tabsRef.current.find((tab) => tab.key === tabKey);

    if (previousTab?.sessionId) {
      void disconnectTerminalSession(previousTab.sessionId, previousTab.hostId);
    }

    activeConnectionIdsRef.current.set(tabKey, connectionId);
    setTabs((prev) => prev.map((tab) => (
      tab.key === tabKey
        ? {
          ...tab,
          sessionId: undefined,
          state: 'connecting',
          errorMessage: undefined,
          connectionId,
          auxiliaryReady: false,
        }
        : tab
    )));
    setActiveKey(tabKey);

    try {
      const sessionId = await invoke<string>('terminal_connect_local', {
        cols: 80,
        rows: 24,
      });

      if (connectDisposedRef.current || activeConnectionIdsRef.current.get(tabKey) !== connectionId) {
        await disconnectTerminalSession(sessionId, undefined);
        return;
      }

      activeConnectionIdsRef.current.delete(tabKey);
      setTabs((prev) => prev.map((tab) => (
        tab.key === tabKey
          ? {
            ...tab,
            sessionId,
            hostName: tText('terminal.localTerminal'),
            hostIp: '127.0.0.1',
            kind: 'local',
            state: 'connected',
            errorMessage: undefined,
            connectionId: undefined,
            auxiliaryReady: true,
          }
          : tab
      )));
      setActiveKey(tabKey);
    } catch (e: unknown) {
      if (connectDisposedRef.current || activeConnectionIdsRef.current.get(tabKey) !== connectionId) {
        return;
      }

      activeConnectionIdsRef.current.delete(tabKey);
      setTabs((prev) => prev.map((tab) => (
        tab.key === tabKey
          ? {
            ...tab,
            sessionId: undefined,
            state: 'error',
            errorMessage: getErrorMessage(e),
            connectionId: undefined,
            auxiliaryReady: false,
          }
          : tab
      )));
      setActiveKey(tabKey);
    }
  }, [tText]);

  const connectLocal = useCallback(() => {
    const tabKey = createPendingTabKey('local');
    const tab: TerminalTab = {
      key: tabKey,
      hostName: tText('terminal.localTerminal'),
      hostIp: '127.0.0.1',
      kind: 'local',
      state: 'connecting',
    };

    setTabs((prev) => [...prev, tab]);
    setActiveKey(tabKey);
    void connectLocalTab(tabKey);
  }, [connectLocalTab, tText]);

  const handleCloseSplit = useCallback((tab: TerminalTab) => {
    const splitSessionId = tab.splitSessionId;
    const splitHostId = tab.kind === 'remote' ? tab.hostId : undefined;
    activeConnectionIdsRef.current.delete(`${tab.key}:split`);
    terminalBufferRefs.current.delete(`${tab.key}:split`);
    setTabs((prev) => prev.map((item) => (
      item.key === tab.key
        ? {
          ...item,
          splitSessionId: undefined,
          splitState: undefined,
          splitErrorMessage: undefined,
          splitConnectionId: undefined,
        }
        : item
    )));
    if (splitSessionId) {
      window.setTimeout(() => {
        useAiChatStore.getState().clearSession(splitSessionId);
        void disconnectTerminalSession(splitSessionId, splitHostId);
      }, 0);
    }
  }, []);

  const handleOpenSplit = useCallback((tabKey: string) => {
    const tab = tabsRef.current.find((item) => item.key === tabKey);
    if (!tab || tab.state !== 'connected' || tab.splitState || tab.splitSessionId) return;

    const splitConnectionId = createConnectionId(`${tabKey}:split`);
    activeConnectionIdsRef.current.set(`${tabKey}:split`, splitConnectionId);
    setSplitContextMenu(null);
    setTabs((prev) => prev.map((item) => (
      item.key === tabKey
        ? {
          ...item,
          splitState: 'connecting' as const,
          splitErrorMessage: undefined,
          splitConnectionId,
        }
        : item
    )));

    const connect = tab.kind === 'remote' && tab.hostId
      ? invoke<string>('terminal_connect', { hostId: tab.hostId, cols: 80, rows: 24 })
      : invoke<string>('terminal_connect_local', { cols: 80, rows: 24 });

    void connect.then((splitSessionId) => {
      if (connectDisposedRef.current || activeConnectionIdsRef.current.get(`${tabKey}:split`) !== splitConnectionId) {
        void disconnectTerminalSession(splitSessionId, tab.kind === 'remote' ? tab.hostId : undefined);
        return;
      }

      activeConnectionIdsRef.current.delete(`${tabKey}:split`);
      setTabs((prev) => prev.map((item) => (
        item.key === tabKey
          ? {
            ...item,
            splitSessionId,
            splitState: 'connected' as const,
            splitErrorMessage: undefined,
            splitConnectionId: undefined,
          }
          : item
      )));
    }).catch((e: unknown) => {
      if (connectDisposedRef.current || activeConnectionIdsRef.current.get(`${tabKey}:split`) !== splitConnectionId) {
        return;
      }

      activeConnectionIdsRef.current.delete(`${tabKey}:split`);
      setTabs((prev) => prev.map((item) => (
        item.key === tabKey
          ? {
            ...item,
            splitSessionId: undefined,
            splitState: 'error' as const,
            splitErrorMessage: getErrorMessage(e),
            splitConnectionId: undefined,
          }
          : item
      )));
    });
  }, []);

  const handleCopySelection = useCallback((selectedText: string) => {
    void navigator.clipboard.writeText(selectedText).catch((error) => {
      void logHandledError('terminal.copySelection', error, 'warn');
    });
    setSplitContextMenu(null);
  }, []);

  const handlePasteClipboard = useCallback((terminalKey: string) => {
    setSplitContextMenu(null);
    void navigator.clipboard.readText()
      .then((text) => pasteTerminalText(terminalKey, text))
      .catch((error) => {
        void logHandledError('terminal.pasteClipboard', error, 'warn');
      });
  }, [pasteTerminalText]);

  const handleAiAnalyzeSelection = useCallback((tabKey: string, selectedText: string) => {
    const tab = tabsRef.current.find((t) => t.key === tabKey);
    if (!tab?.sessionId) return;

    const store = useAiChatStore.getState();
    store.activateSession(tab.sessionId, tab.kind === 'remote' && tab.hostId
      ? { scope: 'ssh_host', scopeId: tab.hostId }
      : { scope: 'terminal_session', scopeId: tab.sessionId });
    void store.initStreamListener();
    store.setBottomTab('ai');
    setSplitContextMenu(null);

    const context = `你是 OpsBatch 终端的 AI 运维助手。用户框选了终端中的一部分输出，请分析其中可能存在的错误原因，并给出诊断建议和修复方案。

## 分析要求
1. 识别错误类型和错误原因
2. 分析可能的根因
3. 给出具体的修复建议和命令

## 命令格式
当需要执行命令时，在回复末尾用以下格式提议：
[ACTION:命令描述]
实际命令内容
[/ACTION]
可以提议多个命令，每个都用 ACTION 块包裹。用户可以在右侧命令确认栏审批这些命令。批准后命令将通过 bracketed paste 注入终端。`;

    const hostInfo = tab.hostName && tab.hostIp ? `\n\n当前连接: ${tab.hostName} (${tab.hostIp})` : '';
    const truncated = selectedText.length > 4000 ? selectedText.slice(0, 4000) + '\n…(已截断)' : selectedText;

    window.setTimeout(() => {
      void useAiChatStore.getState().sendDirectMessage(
        `请分析以下终端输出中的错误原因：\n\n\`\`\`\n${truncated}\n\`\`\``,
        context + hostInfo,
      );
    }, 50);
  }, []);

  const handleTerminalContextMenu = useCallback((event: MouseEvent<HTMLDivElement>, tab: TerminalTab, terminalKey: string) => {
    if (tab.state !== 'connected') return;

    event.preventDefault();

    const bufferApi = terminalBufferRefs.current.get(terminalKey);
    const selectedText = bufferApi?.getSelection() || undefined;

    setSplitContextMenu({
      tabKey: tab.key,
      terminalKey,
      x: event.clientX,
      y: event.clientY,
      selectedText,
    });
  }, []);

  useEffect(() => {
    if (!visible) return;

    const openHostRequest = getOpenHostRequest(location.state);
    if (openHostRequest) {
      if (!consumedOpenHostRequestIdsRef.current.has(openHostRequest.requestId)) {
        consumedOpenHostRequestIdsRef.current.add(openHostRequest.requestId);
        const started = connectHost(
          openHostRequest.hostId,
          openHostRequest.name || tText('terminal.defaultName'),
          openHostRequest.ip || '',
        );
        if (started) {
          lastRemoteConnectionRef.current = {
            hostId: openHostRequest.hostId,
            name: openHostRequest.name || tText('terminal.defaultName'),
            ip: openHostRequest.ip || '',
          };
          autoConnectedLocalRef.current = true;
        }
        navigate('/terminal', { replace: true, state: null });
      }
      return;
    }

    const queryHostConnection = getQueryHostConnection(searchParams, tText('terminal.defaultName'));
    if (queryHostConnection) {
      if (autoConnectedHostIdRef.current !== queryHostConnection.hostId) {
        autoConnectedHostIdRef.current = queryHostConnection.hostId;
        lastRemoteConnectionRef.current = queryHostConnection;
        autoConnectedLocalRef.current = true;
        connectHost(queryHostConnection.hostId, queryHostConnection.name, queryHostConnection.ip);
      }
      return;
    }

    if (!autoConnectedLocalRef.current) {
      autoConnectedLocalRef.current = true;
      connectLocal();
    }
  }, [visible, connectHost, connectLocal, location.state, navigate, searchParams, tText]);

  const disconnectSession = useCallback((sessionId: string, hostId?: string) => {
    void disconnectTerminalSession(sessionId, hostId);
  }, []);

  const handleCloseTabs = useCallback(
    (targetKeys: readonly string[], preferredActiveKey?: string) => {
      if (targetKeys.length === 0) return;

      const currentTabs = tabsRef.current;
      const targetKeySet = new Set(targetKeys);
      const closingTabs = currentTabs.filter((tab) => targetKeySet.has(tab.key));
      if (closingTabs.length === 0) return;

      const firstClosingIndex = currentTabs.findIndex((tab) => targetKeySet.has(tab.key));
      const nextTabs = currentTabs.filter((tab) => !targetKeySet.has(tab.key));

      // Immediate: update refs so everything else uses the correct state
      closingTabs.forEach((tab) => {
        activeConnectionIdsRef.current.delete(tab.key);
        activeConnectionIdsRef.current.delete(`${tab.key}:split`);
        terminalBufferRefs.current.delete(tab.key);
        terminalBufferRefs.current.delete(`${tab.key}:split`);
      });
      tabsRef.current = nextTabs;

      const preferredTabExists = preferredActiveKey && nextTabs.some((tab) => tab.key === preferredActiveKey);
      const activeTabWasClosed = targetKeySet.has(activeKeyRef.current);

      // Immediate: switch active tab away from closed tabs
      if (activeTabWasClosed) {
        const nextActiveKey = (
          preferredTabExists
            ? preferredActiveKey
            : nextTabs[firstClosingIndex]?.key ?? nextTabs[firstClosingIndex - 1]?.key ?? ''
        );
        activeKeyRef.current = nextActiveKey;
        setActiveKey(nextActiveKey);
      }

      // Deferred: remove tab from list (re-render is interruptible)
      startTransition(() => {
        setTabs(nextTabs);
      });

      // Async: destroy frontend & backend resources after UI has settled
      closingTabs.forEach((tab) => {
        if (!tab.sessionId) return;
        const { sessionId, hostId, splitSessionId, kind } = tab;
        window.setTimeout(() => {
          useAiChatStore.getState().clearSession(sessionId);
          disconnectSession(sessionId, hostId);
          if (splitSessionId) {
            useAiChatStore.getState().clearSession(splitSessionId);
            disconnectSession(splitSessionId, kind === 'remote' ? hostId : undefined);
          }
        }, 0);
      });
    },
    [disconnectSession],
  );

  const handleCloseTab = useCallback((targetKey: string) => {
    handleCloseTabs([targetKey]);
  }, [handleCloseTabs]);

  const handleCloseTabsByMode = useCallback((targetKey: string, mode: TerminalTabCloseMode) => {
    const targetKeys = getTerminalTabCloseTargets(tabsRef.current.map((tab) => tab.key), targetKey, mode);
    const preferredActiveKey = mode === 'all' ? undefined : targetKey;
    setTabContextMenu(null);
    handleCloseTabs(targetKeys, preferredActiveKey);
  }, [handleCloseTabs]);

  const handleTabContextMenu = useCallback((event: MouseEvent<HTMLDivElement>, tabKey: string) => {
    event.preventDefault();
    setSplitContextMenu(null);
    setTabContextMenu({
      kind: 'tab',
      tabKey,
      x: event.clientX,
      y: event.clientY,
    });
  }, []);

  const handleTabListContextMenu = useCallback((event: MouseEvent<HTMLDivElement>) => {
    event.preventDefault();
    setSplitContextMenu(null);
    setTabContextMenu({
      kind: 'list',
      x: event.clientX,
      y: event.clientY,
    });
  }, []);

  const handleCloseAllTabs = useCallback(() => {
    setTabContextMenu(null);
    handleCloseTabs(tabsRef.current.map((tab) => tab.key));
  }, [handleCloseTabs]);

  const handleRetryTab = useCallback((tab: TerminalTab) => {
    if (tab.kind === 'remote' && tab.hostId) {
      lastRemoteConnectionRef.current = {
        hostId: tab.hostId,
        name: tab.hostName,
        ip: tab.hostIp,
      };
      void connectRemoteTab(tab.key, tab.hostId, tab.hostName, tab.hostIp);
      return;
    }

    void connectLocalTab(tab.key);
  }, [connectLocalTab, connectRemoteTab]);

  const handleOpenLocalTerminal = useCallback(() => {
    autoConnectedLocalRef.current = true;
    connectLocal();
  }, [connectLocal]);

  const handleBackToAssets = useCallback(() => {
    navigate('/terminal?assets=1');
  }, [navigate]);

  const handleNewConnectionFromContext = useCallback(() => {
    setTabContextMenu(null);
    handleBackToAssets();
  }, [handleBackToAssets]);

  const renderTabLabel = useCallback((tab: TerminalTab) => {
    const stateLabel = getTabStateLabel(tab.state, {
      connecting: tText('terminal.state.connecting'),
      connected: tText('terminal.state.connected'),
      error: tText('terminal.state.error'),
    });
    const title = `${tab.hostName}${tab.hostIp ? ` (${tab.hostIp})` : ''} · ${stateLabel}`;

    return (
      <span className={`terminal-tab-label terminal-tab-label-${tab.state}`} title={title}>
        <span className="terminal-tab-status-dot" aria-hidden="true" />
        <span className="terminal-tab-text">
          <span className="terminal-tab-name">{tab.hostName}</span>
          {tab.hostIp ? <span className="terminal-tab-meta">{tab.hostIp}</span> : null}
        </span>
        {tab.state !== 'connected' ? (
          <span className={`terminal-tab-status-text terminal-tab-status-text-${tab.state}`}>{stateLabel}</span>
        ) : null}
        <span className="terminal-tab-sr-only">{t('terminal.statusSr', { state: stateLabel })}</span>
      </span>
    );
  }, [t, tText]);

  const renderTabContent = useCallback((tab: TerminalTab, isActive: boolean) => {
    if (tab.sessionId && tab.state === 'connected') {
      return (
        <TerminalTabContent
          tab={tab}
          isActive={isActive}
          setTerminalBuffer={setTerminalBuffer}
          onContextMenu={handleTerminalContextMenu}
          onCloseSplit={handleCloseSplit}
        />
      );
    }

    if (tab.state === 'connecting') {
      const target = tab.hostIp || tab.hostName;
      return (
        <div className="terminal-page-state-shell">
          <section className="terminal-state-card" aria-live="polite">
            <div className="terminal-state-card-header">
              <span className="terminal-status-light terminal-status-light-connecting" aria-hidden="true" />
              <span>{t('terminal.connectingTitle')}</span>
            </div>
            <div className="terminal-state-card-body">
              <Spin size="small" />
              <div>
                <div className="terminal-state-title">{t('terminal.connectTo', { target })}</div>
                <div className="terminal-state-subtitle">{t('terminal.connectingSubtitle')}</div>
              </div>
            </div>
            <div className="terminal-state-command" aria-hidden="true">
              <span>$</span>
              <span className="terminal-state-command-text">opsbatch terminal connect --target {target}</span>
            </div>
          </section>
        </div>
      );
    }

    if (tab.state === 'error') {
      const isRemote = tab.kind === 'remote';
      return (
        <div className="terminal-page-state-shell">
          <section className="terminal-state-card terminal-state-card-error" role="alert">
            <div className="terminal-state-card-header">
              <span className="terminal-status-light terminal-status-light-error" aria-hidden="true" />
              <span>{isRemote ? t('terminal.remoteConnectFailed') : t('terminal.localStartFailed')}</span>
            </div>
            <div className="terminal-state-title">{t('terminal.sessionStartFailed')}</div>
            <p className="terminal-state-subtitle">{tab.errorMessage || t('terminal.errorFallback')}</p>
            <div className="terminal-state-actions">
              <Button type="primary" onClick={() => handleRetryTab(tab)}>{t('terminal.retry')}</Button>
              {isRemote ? <Button onClick={handleBackToAssets}>{t('terminal.openAssetsSidebar')}</Button> : null}
            </div>
          </section>
        </div>
      );
    }

    return (
      <div className="terminal-page-state-shell">
        <Empty description={t('terminal.sessionNotReady')} />
      </div>
    );
  }, [handleBackToAssets, handleRetryTab, setTerminalBuffer, handleTerminalContextMenu, handleCloseSplit, t]);

  // 监听连接状态变化（空闲断开、级联故障等）
  useEffect(() => {
    if (!visible) return;
    const unlistenPromise = listen<{ hostId: string; status: string }>('connection_status_changed', (event) => {
      const { hostId, status } = event.payload;
      if (status === 'idle_disconnected' || status === 'link_down') {
        setTabs((prev) => prev.map((tab) => {
          if (tab.hostId === hostId && tab.state === 'connected') {
            return {
              ...tab,
              state: 'error' as const,
              errorMessage: status === 'idle_disconnected'
                ? tText('terminal.idleDisconnected')
                : tText('terminal.linkDown'),
            };
          }
          return tab;
        }));
      }
    });
    return () => {
      unlistenPromise.then((fn) => fn());
    };
  }, [visible, tText]);

  // 页面卸载时关闭所有连接
  useEffect(() => {
    connectDisposedRef.current = false;
    return () => {
      connectDisposedRef.current = true;
      tabsRef.current.forEach((tab) => {
        if (tab.sessionId) {
          void disconnectTerminalSession(tab.sessionId, tab.hostId);
        }
        if (tab.splitSessionId) {
          void disconnectTerminalSession(tab.splitSessionId, tab.kind === 'remote' ? tab.hostId : undefined);
        }
      });
    };
  }, []);

  const activeTab = useMemo(() => tabs.find((t) => t.key === activeKey), [tabs, activeKey]);
  const deferredActiveTab = useDeferredValue(activeTab);
  const [tabSwitching, setTabSwitching] = useState(false);

  useEffect(() => {
    setTabSwitching(true);
    const timer = window.setTimeout(() => setTabSwitching(false), 120);
    return () => window.clearTimeout(timer);
  }, [activeKey]);
  const activeRemoteHostId = deferredActiveTab?.kind === 'remote' ? deferredActiveTab.hostId : undefined;
  const isTerminalConnected = !!activeTab?.sessionId && activeTab.state === 'connected';
  const shouldShowFixedPanels = isTerminalConnected;
  const hasMonitor = shouldShowFixedPanels && !!activeRemoteHostId;

  useEffect(() => {
    if (!hasMonitor) return undefined;

    const shell = terminalPageShellRef.current;
    if (!shell) return undefined;

    const clampToShell = (shellWidth: number) => {
      if (shellWidth <= 0) return;
      setMonitorWidth((currentWidth) => clampMonitorPanelWidth(currentWidth, shellWidth));
    };

    clampToShell(shell.clientWidth);

    const resizeObserver = new ResizeObserver((entries) => {
      const entry = entries[0];
      clampToShell(entry?.contentRect.width ?? shell.clientWidth);
    });
    resizeObserver.observe(shell);

    return () => resizeObserver.disconnect();
  }, [hasMonitor]);

  const handleMonitorResizePointerDown = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    monitorResizeCleanupRef.current?.();

    const startX = event.clientX;
    const startWidth = monitorWidth;
    const containerWidth = terminalPageShellRef.current?.clientWidth;

    const cleanup = () => {
      window.removeEventListener('pointermove', handlePointerMove);
      window.removeEventListener('pointerup', handlePointerUp);
      window.removeEventListener('pointercancel', handlePointerUp);
      document.body.classList.remove('terminal-monitor-resizing');
      monitorResizeCleanupRef.current = null;
    };

    const handlePointerMove = (moveEvent: PointerEvent) => {
      const deltaX = startX - moveEvent.clientX;
      setMonitorWidth(clampMonitorPanelWidth(startWidth + deltaX, containerWidth));
    };

    const handlePointerUp = () => {
      cleanup();
    };

    document.body.classList.add('terminal-monitor-resizing');
    monitorResizeCleanupRef.current = cleanup;
    window.addEventListener('pointermove', handlePointerMove);
    window.addEventListener('pointerup', handlePointerUp);
    window.addEventListener('pointercancel', handlePointerUp);
  }, [monitorWidth]);

  const bottomPanelGetTerminalBuffer = useCallback(() => {
    const key = activeKeyRef.current;
    return key ? getTerminalBuffer(key) : '';
  }, [getTerminalBuffer]);
  const bottomPanelExecuteTerminalCommand = useCallback((
    command: string,
    options?: Parameters<TerminalController['executeCommand']>[1],
  ) => {
    const key = activeKeyRef.current;
    return key ? executeTerminalCommand(key, command, options) : undefined;
  }, [executeTerminalCommand]);
  const bottomPanelInsertTerminalCommand = useCallback((command: string) => {
    const key = activeKeyRef.current;
    return key ? insertTerminalCommand(key, command) : undefined;
  }, [insertTerminalCommand]);
  const tabItems = useMemo(() => tabs.map((tab) => ({
    key: tab.key,
    label: renderTabLabel(tab),
    closable: true,
    closeLabel: tText('terminal.closeTabLabel', { name: tab.hostName }),
    children: (isActive: boolean) => renderTabContent(tab, isActive),
  })), [tabs, renderTabLabel, renderTabContent, tText]);

  if (tabs.length === 0) {
    return (
      <div className="terminal-page-state-shell">
        <Empty
          description={t('terminal.noSessions')}
          styles={{ footer: { marginTop: 16 } }}
        >
          <Button type="primary" onClick={handleOpenLocalTerminal}>{t('terminal.reopenLocal')}</Button>
        </Empty>
      </div>
    );
  }

  return (
    <div ref={terminalPageShellRef} className={`terminal-page-shell terminal-page-shell-active ${hasMonitor ? 'terminal-layout-with-monitor' : ''}`}>
      <div className={`terminal-layout-left${tabSwitching ? ' terminal-tab-switching' : ''}`}>
        <Tabs
          type="editable-card"
          activeKey={activeKey}
          onChange={setActiveKey}
          hideAdd
          destroyInactiveTabPane
          className="terminal-tabs"
          items={tabItems}
          onTabContextMenu={handleTabContextMenu}
          onTabListContextMenu={handleTabListContextMenu}
          tabBarExtraContent={(
            <button
              type="button"
              className="terminal-tab-add"
              title={tText('terminal.connectHost')}
              onClick={handleBackToAssets}
            >
              <PlusOutlined />
            </button>
          )}
          onEdit={(targetKey, action) => {
            if (action === 'remove' && typeof targetKey === 'string') {
              handleCloseTab(targetKey);
            }
          }}
        />

        {shouldShowFixedPanels && deferredActiveTab?.sessionId && (
          <BottomPanel
            hostId={deferredActiveTab.hostId}
            hostName={deferredActiveTab.hostName}
            hostIp={deferredActiveTab.hostIp}
            sessionId={deferredActiveTab.sessionId}
            isRemote={!!activeRemoteHostId}
            getTerminalBuffer={bottomPanelGetTerminalBuffer}
            executeTerminalCommand={bottomPanelExecuteTerminalCommand}
            insertTerminalCommand={bottomPanelInsertTerminalCommand}
          />
        )}

        <SplitContextMenu
          menu={splitContextMenu}
          copyLabel={tText('terminal.copy')}
          pasteLabel={tText('terminal.paste')}
          aiAnalyzeLabel={tText('terminal.aiAnalyzeError')}
          openSplitLabel={tText('terminal.openSplit')}
          onCopySelection={handleCopySelection}
          onPasteClipboard={handlePasteClipboard}
          onAiAnalyzeSelection={handleAiAnalyzeSelection}
          onOpenSplit={handleOpenSplit}
        />

        <TabContextMenu
          menu={tabContextMenu}
          labels={{
            newConnection: tText('terminal.context.newConnection'),
            closeAll: tText('terminal.context.closeAll'),
            closeOthers: tText('terminal.context.closeOthers'),
            closeLeft: tText('terminal.context.closeLeft'),
            closeRight: tText('terminal.context.closeRight'),
          }}
          onNewConnection={handleNewConnectionFromContext}
          onCloseAll={handleCloseAllTabs}
          onCloseTabsByMode={handleCloseTabsByMode}
        />
      </div>

      {hasMonitor && deferredActiveTab && activeRemoteHostId && (
        <HostMonitorPanel
          hostId={activeRemoteHostId}
          hostIp={deferredActiveTab.hostIp}
          width={monitorWidth}
          onResizePointerDown={handleMonitorResizePointerDown}
        />
      )}
    </div>
  );
}
