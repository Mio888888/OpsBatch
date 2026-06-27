/* @refresh reset */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import RFB from '@novnc/novnc';
import { Button, Empty, Spin } from '../../components/ui';
import { CloseOutlined, ReloadOutlined } from '../../components/ui/icons';
import WindowControls from '../../components/WindowControls';
import { useAssetsStore } from '../../stores/assets';
import { useTranslation } from '../../i18n';
import {
  createVncSessionId,
  type VncConnectionState,
  type VncSessionStatus,
  vncDefaultResolution,
} from './vncProtocol';
import { createVncClipboardBridge } from './vncClipboard';
import '../../styles/pages/rdp.css';
import '../../styles/terminal/terminal.css';

interface VncConnectResponse {
  sessionId: string;
  hostId: string;
  websocketUrl: string;
  username?: string | null;
  password?: string | null;
  authMethod?: 'vnc' | 'ard';
  shared: boolean;
  viewOnly: boolean;
}

type RfbEvent<TDetail = unknown> = Event & { detail?: TDetail };
type VncClipboardBridgeHandle = ReturnType<typeof createVncClipboardBridge>;

const VNC_INTERACTIVE_QUALITY_LEVEL = 2;
const VNC_INTERACTIVE_COMPRESSION_LEVEL = 2;
const VNC_CONNECT_WATCHDOG_MS = 15000;

const VNC_ENCODING_COPY_RECT = 1;
const VNC_ENCODING_RRE = 2;
const VNC_ENCODING_HEXTILE = 5;
const VNC_ENCODING_ZLIB = 6;
const VNC_ENCODING_TIGHT = 7;
const VNC_ENCODING_ZRLE = 16;
const VNC_ENCODING_JPEG = 21;
const VNC_ENCODING_RAW = 0;
const VNC_ENCODING_TIGHT_PNG = -260;

type NoVncMessages = {
  clientEncodings?: (sock: unknown, encodings: number[]) => void;
};

type RfbDebugState = {
  _display?: {
    pending?: () => boolean;
  };
  _fbHeight?: number;
  _fbWidth?: number;
  _FBU?: {
    encoding?: number | null;
  };
  _framebufferUpdate?: (...args: unknown[]) => boolean;
  _handleRect?: (...args: unknown[]) => boolean;
  _sendMouse?: (x: number, y: number, mask: number) => void;
  _rfbAuthScheme?: number;
  _rfbVersion?: number;
  _enabledContinuousUpdates?: boolean;
  _supportsSetDesktopSize?: boolean;
};

type RfbEncodingState = {
  messages?: NoVncMessages;
};

function writeVncDiagnosticLog(message: string) {
  const context = `href=${window.location.href} search=${window.location.search} hash=${window.location.hash}`;
  void invoke('write_diagnostic_log', {
    source: 'vnc-frontend',
    message: `${message} ${context}`.slice(0, 4000),
  }).catch(() => undefined);
}

function isVncDebugEnabled(locationSearch: string) {
  const params = new URLSearchParams(locationSearch);
  if (params.get('vncDebug') === '1') return true;
  try {
    return window.localStorage.getItem('opsbatch.vncDebug') === '1';
  } catch {
    return false;
  }
}

function clearElement(element: HTMLElement | null) {
  if (!element) return;
  element.replaceChildren();
}

function hasRequiredVncCredentials(credentials: Record<string, string>, types: string[]) {
  return types.length > 0 && types.every((type) => Boolean(credentials[type]));
}

function missingVncCredentials(credentials: Record<string, string>, types: string[]) {
  return types.filter((type) => !credentials[type]);
}

function describeRfbAuthScheme(rfb: RFB) {
  const debugState = rfb as unknown as RfbDebugState;
  return `rfbVersion=${debugState._rfbVersion ?? 'unknown'} authScheme=${debugState._rfbAuthScheme ?? 'unknown'}`;
}

function describeRfbRuntime(rfb: RFB) {
  const debugState = rfb as unknown as RfbDebugState;
  return [
    `rfbVersion=${debugState._rfbVersion ?? 'unknown'}`,
    `authScheme=${debugState._rfbAuthScheme ?? 'unknown'}`,
    `framebuffer=${debugState._fbWidth ?? 'unknown'}x${debugState._fbHeight ?? 'unknown'}`,
    `setDesktopSize=${debugState._supportsSetDesktopSize === true}`,
    `continuousUpdates=${debugState._enabledContinuousUpdates === true}`,
  ].join(' ');
}

function vncEncodingName(encoding: number | null | undefined) {
  const names: Record<number, string> = {
    0: 'raw',
    1: 'copyRect',
    2: 'rre',
    5: 'hextile',
    6: 'zlib',
    7: 'tight',
    16: 'zrle',
    21: 'jpeg',
    50: 'h264',
    [-223]: 'desktopSize',
    [-239]: 'cursor',
    [-308]: 'extendedDesktopSize',
  };
  return names[encoding ?? Number.NaN] ?? String(encoding ?? 'unknown');
}

function installVncEncodingPreferences() {
  const messages = (RFB as unknown as RfbEncodingState).messages;
  const originalClientEncodings = messages?.clientEncodings;
  if (!messages || !originalClientEncodings) return undefined;

  messages.clientEncodings = function preferredClientEncodings(sock: unknown, requestedEncodings: number[]) {
    const preferred = [
      VNC_ENCODING_COPY_RECT,
      VNC_ENCODING_RRE,
      VNC_ENCODING_HEXTILE,
      VNC_ENCODING_ZRLE,
      VNC_ENCODING_TIGHT,
      VNC_ENCODING_TIGHT_PNG,
      VNC_ENCODING_JPEG,
      VNC_ENCODING_ZLIB,
      VNC_ENCODING_RAW,
    ];
    const reordered = [
      ...preferred.filter((encoding) => requestedEncodings.includes(encoding)),
      ...requestedEncodings.filter((encoding) => !preferred.includes(encoding)),
    ];

    return originalClientEncodings.call(this, sock, reordered);
  };

  return () => {
    if (messages.clientEncodings !== originalClientEncodings) {
      messages.clientEncodings = originalClientEncodings;
    }
  };
}

function installVncPerformanceDiagnostics(
  rfb: RFB,
  hostId: string,
  sessionId: string,
) {
  const debugState = rfb as unknown as RfbDebugState;
  const originalFramebufferUpdate = debugState._framebufferUpdate;
  const originalHandleRect = debugState._handleRect;
  const stats = {
    framebufferUpdates: 0,
    rects: 0,
    totalFramebufferMs: 0,
    maxFramebufferMs: 0,
    encodings: new Map<string, number>(),
  };

  if (originalFramebufferUpdate) {
    debugState._framebufferUpdate = function patchedFramebufferUpdate(...args: unknown[]) {
      const startedAt = performance.now();
      const result = originalFramebufferUpdate.apply(this, args);
      const elapsed = performance.now() - startedAt;
      stats.framebufferUpdates += 1;
      stats.totalFramebufferMs += elapsed;
      stats.maxFramebufferMs = Math.max(stats.maxFramebufferMs, elapsed);
      return result;
    };
  }

  if (originalHandleRect) {
    debugState._handleRect = function patchedHandleRect(...args: unknown[]) {
      const encoding = vncEncodingName((rfb as unknown as RfbDebugState)._FBU?.encoding);
      stats.rects += 1;
      stats.encodings.set(encoding, (stats.encodings.get(encoding) ?? 0) + 1);
      return originalHandleRect.apply(this, args);
    };
  }

  const interval = window.setInterval(() => {
    if (stats.framebufferUpdates === 0 && stats.rects === 0) return;
    const avgFramebufferMs = stats.framebufferUpdates > 0
      ? stats.totalFramebufferMs / stats.framebufferUpdates
      : 0;
    const encodingSummary = Array.from(stats.encodings.entries())
      .map(([encoding, count]) => `${encoding}:${count}`)
      .join(',');
    const pending = debugState._display?.pending?.() === true;
    writeVncDiagnosticLog(
      `novnc perf hostId=${hostId} sessionId=${sessionId} fbu=${stats.framebufferUpdates} rects=${stats.rects} avgFbuMs=${avgFramebufferMs.toFixed(2)} maxFbuMs=${stats.maxFramebufferMs.toFixed(2)} encodings=${encodingSummary || 'none'} pending=${pending} ${describeRfbRuntime(rfb)}`,
    );
    stats.framebufferUpdates = 0;
    stats.rects = 0;
    stats.totalFramebufferMs = 0;
    stats.maxFramebufferMs = 0;
    stats.encodings.clear();
  }, 2000);

  return () => {
    window.clearInterval(interval);
    if (debugState._framebufferUpdate !== originalFramebufferUpdate) {
      debugState._framebufferUpdate = originalFramebufferUpdate;
    }
    if (debugState._handleRect !== originalHandleRect) {
      debugState._handleRect = originalHandleRect;
    }
  };
}

function installVncInputDiagnostics(
  rfb: RFB,
  target: HTMLElement,
  hostId: string,
  sessionId: string,
) {
  const debugState = rfb as unknown as RfbDebugState;
  const originalSendMouse = debugState._sendMouse;
  const originalSendKey = rfb.sendKey;
  const stats = {
    domMouse: 0,
    domKey: 0,
    sentMouse: 0,
    sentKey: 0,
    lastMouseMask: 0,
    lastKey: '',
  };

  if (originalSendMouse) {
    debugState._sendMouse = function patchedSendMouse(x: number, y: number, mask: number) {
      stats.sentMouse += 1;
      stats.lastMouseMask = mask;
      return originalSendMouse.call(this, x, y, mask);
    };
  }

  if (originalSendKey) {
    rfb.sendKey = function patchedSendKey(keysym: number, code: string, down?: boolean) {
      stats.sentKey += 1;
      stats.lastKey = `${code}:${down === false ? 'up' : 'down'}`;
      return originalSendKey.call(this, keysym, code, down);
    };
  }

  const focusRfb = () => {
    rfb.focus({ preventScroll: true });
  };
  const recordMouse = (event: MouseEvent) => {
    stats.domMouse += 1;
    if (event.type === 'mousedown' || event.type === 'mouseup') {
      focusRfb();
      writeVncDiagnosticLog(
        `novnc input domMouse hostId=${hostId} sessionId=${sessionId} type=${event.type} button=${event.button} buttons=${event.buttons} viewOnly=${rfb.viewOnly} target=${(event.target as HTMLElement | null)?.tagName ?? 'unknown'}`,
      );
    }
  };
  const recordKey = (event: KeyboardEvent) => {
    stats.domKey += 1;
    if (event.type === 'keydown') {
      writeVncDiagnosticLog(
        `novnc input domKey hostId=${hostId} sessionId=${sessionId} key=${event.key} code=${event.code} viewOnly=${rfb.viewOnly} target=${(event.target as HTMLElement | null)?.tagName ?? 'unknown'}`,
      );
    }
  };

  target.addEventListener('mousedown', recordMouse, true);
  target.addEventListener('mouseup', recordMouse, true);
  target.addEventListener('mousemove', recordMouse, true);
  target.addEventListener('keydown', recordKey, true);
  target.addEventListener('keyup', recordKey, true);

  const interval = window.setInterval(() => {
    if (stats.domMouse === 0 && stats.domKey === 0 && stats.sentMouse === 0 && stats.sentKey === 0) return;
    writeVncDiagnosticLog(
      `novnc input summary hostId=${hostId} sessionId=${sessionId} domMouse=${stats.domMouse} sentMouse=${stats.sentMouse} lastMouseMask=${stats.lastMouseMask} domKey=${stats.domKey} sentKey=${stats.sentKey} lastKey=${stats.lastKey || 'none'} viewOnly=${rfb.viewOnly} ${describeRfbRuntime(rfb)}`,
    );
    stats.domMouse = 0;
    stats.domKey = 0;
    stats.sentMouse = 0;
    stats.sentKey = 0;
    stats.lastMouseMask = 0;
    stats.lastKey = '';
  }, 2000);

  return () => {
    window.clearInterval(interval);
    target.removeEventListener('mousedown', recordMouse, true);
    target.removeEventListener('mouseup', recordMouse, true);
    target.removeEventListener('mousemove', recordMouse, true);
    target.removeEventListener('keydown', recordKey, true);
    target.removeEventListener('keyup', recordKey, true);
    if (debugState._sendMouse !== originalSendMouse) {
      debugState._sendMouse = originalSendMouse;
    }
    if (rfb.sendKey !== originalSendKey) {
      rfb.sendKey = originalSendKey;
    }
  };
}

export default function VncPage() {
  const { t, tText } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const hosts = useAssetsStore((s) => s.hosts);
  const hostsLoading = useAssetsStore((s) => s.loading);
  const loadHosts = useAssetsStore((s) => s.loadHosts);
  const queryHostId = useMemo(() => new URLSearchParams(location.search).get('hostId')?.trim() ?? '', [location.search]);
  const vncDebugEnabled = useMemo(() => isVncDebugEnabled(location.search), [location.search]);
  const host = hosts.find((item) => item.id === queryHostId);
  const activeHostId = host?.id ?? '';
  const [sessionId, setSessionId] = useState('');
  const [status, setStatus] = useState<VncConnectionState>('idle');
  const [statusMessage, setStatusMessage] = useState('');
  const [connectNonce, setConnectNonce] = useState(0);
  const screenRef = useRef<HTMLDivElement>(null);
  const rfbRef = useRef<RFB | null>(null);
  const sessionIdRef = useRef('');
  const statusRef = useRef<VncConnectionState>('idle');
  const tTextRef = useRef(tText);
  tTextRef.current = tText;

  const updateStatus = useCallback((nextStatus: VncConnectionState) => {
    statusRef.current = nextStatus;
    setStatus(nextStatus);
  }, []);

  const updateSessionId = useCallback((nextSessionId: string) => {
    sessionIdRef.current = nextSessionId;
    setSessionId(nextSessionId);
  }, []);

  const closeVncSession = useCallback((targetSessionId: string, markUi = true) => {
    if (!targetSessionId) return;
    if (targetSessionId === sessionIdRef.current) {
      rfbRef.current?.disconnect();
      rfbRef.current = null;
      clearElement(screenRef.current);
    }
    if (markUi && targetSessionId === sessionIdRef.current && statusRef.current !== 'idle') {
      updateStatus('disconnected');
      setStatusMessage(tTextRef.current('vnc.state.disconnected'));
    }
    void invoke('close_vnc_session', {
      request: { sessionId: targetSessionId },
    }).catch((error: unknown) => {
      if (targetSessionId === sessionIdRef.current) setStatusMessage(String(error));
    });
  }, [updateStatus]);

  const reconnectVncSession = useCallback(() => {
    writeVncDiagnosticLog(`manual reconnect requested hostId=${activeHostId} sessionId=${sessionIdRef.current}`);
    rfbRef.current?.disconnect();
    rfbRef.current = null;
    clearElement(screenRef.current);
    updateStatus('connecting');
    setStatusMessage(tTextRef.current('vnc.state.connecting'));
    setConnectNonce((value) => value + 1);
  }, [activeHostId, updateStatus]);

  useEffect(() => {
    if (queryHostId && hosts.length === 0) void loadHosts();
  }, [hosts.length, loadHosts, queryHostId]);

  useEffect(() => {
    if (!activeHostId) {
      writeVncDiagnosticLog(`effect skipped missing hostId activeHostId=${activeHostId}`);
      return undefined;
    }
    const target = screenRef.current;
    if (!target) {
      writeVncDiagnosticLog(`effect skipped missing screen hostId=${activeHostId}`);
      return undefined;
    }

    const nextSessionId = createVncSessionId(activeHostId);
    let disposed = false;
    let disposePerformanceDiagnostics: (() => void) | undefined;
    let disposeInputDiagnostics: (() => void) | undefined;
    let disposeEncodingPreferences: (() => void) | undefined;
    let clipboardBridge: VncClipboardBridgeHandle | undefined;
    const connectWatchdog = window.setTimeout(() => {
      if (disposed || sessionIdRef.current !== nextSessionId || statusRef.current !== 'connecting') return;
      disposed = true;
      writeVncDiagnosticLog(`novnc connect timeout hostId=${activeHostId} sessionId=${nextSessionId}`);
      closeVncSession(nextSessionId, false);
      updateStatus('error');
      setStatusMessage(tTextRef.current('vnc.connectTimeout'));
    }, VNC_CONNECT_WATCHDOG_MS);
    writeVncDiagnosticLog(
      `effect start hostId=${activeHostId} sessionId=${nextSessionId} connectNonce=${connectNonce}`,
    );
    updateSessionId(nextSessionId);
    updateStatus('connecting');
    setStatusMessage(tTextRef.current('vnc.state.connecting'));
    rfbRef.current?.disconnect();
    rfbRef.current = null;
    clearElement(target);

    const start = async () => {
      writeVncDiagnosticLog(`invoke vnc_connect start hostId=${activeHostId} sessionId=${nextSessionId}`);
      const response = await invoke<VncConnectResponse>('vnc_connect', {
        hostId: activeHostId,
        sessionId: nextSessionId,
      });
      writeVncDiagnosticLog(
        `invoke vnc_connect completed hostId=${activeHostId} sessionId=${nextSessionId} websocketUrl=${response.websocketUrl} usernameSet=${Boolean(response.username)} passwordSet=${Boolean(response.password)} authMethod=${response.authMethod ?? 'vnc'} shared=${response.shared} viewOnly=${response.viewOnly}`,
      );
      if (disposed) {
        closeVncSession(nextSessionId, false);
        return;
      }

      const credentials: Record<string, string> = {};
      if (response.username) credentials.username = response.username;
      if (response.password) credentials.password = response.password;
      disposeEncodingPreferences = installVncEncodingPreferences();
      const rfb = new RFB(target, response.websocketUrl, {
        credentials,
        shared: response.shared,
      });
      rfbRef.current = rfb;
      rfb.viewOnly = response.viewOnly;
      rfb.scaleViewport = true;
      rfb.resizeSession = true;
      rfb.focusOnClick = true;
      rfb.qualityLevel = VNC_INTERACTIVE_QUALITY_LEVEL;
      rfb.compressionLevel = VNC_INTERACTIVE_COMPRESSION_LEVEL;
      rfb.background = '#101417';
      rfb.showDotCursor = true;
      const readLocalClipboardText = () => invoke<string | null>('read_local_clipboard_text');
      const writeLocalClipboardText = (text: string) => invoke<void>('write_local_clipboard_text', { text });
      clipboardBridge = createVncClipboardBridge(rfb, {
        hostId: activeHostId,
        sessionId: nextSessionId,
        readLocalClipboardText,
        writeLocalClipboardText,
        canSendLocalClipboard: () => statusRef.current === 'connected',
        writeDiagnosticLog: writeVncDiagnosticLog,
      });
      writeVncDiagnosticLog(
        `novnc preferences hostId=${activeHostId} sessionId=${nextSessionId} cursor=local-dot encodingPreference=copyrect,rre,hextile,zrle,tight,tightpng,jpeg,zlib,raw`,
      );
      if (vncDebugEnabled) {
        disposePerformanceDiagnostics = installVncPerformanceDiagnostics(rfb, activeHostId, nextSessionId);
        disposeInputDiagnostics = installVncInputDiagnostics(rfb, target, activeHostId, nextSessionId);
      }

      rfb.addEventListener('connect', () => {
        if (disposed || sessionIdRef.current !== nextSessionId) return;
        window.clearTimeout(connectWatchdog);
        writeVncDiagnosticLog(`novnc connect hostId=${activeHostId} sessionId=${nextSessionId} viewOnly=${rfb.viewOnly} focusOnClick=${rfb.focusOnClick} ${describeRfbRuntime(rfb)}`);
        updateStatus('connected');
        setStatusMessage(tTextRef.current('vnc.state.connected'));
        rfb.focus({ preventScroll: true });
        void clipboardBridge?.syncLocalToRemote();
      });
      rfb.addEventListener('disconnect', (event: Event) => {
        const clean = (event as RfbEvent<{ clean?: boolean }>).detail?.clean;
        writeVncDiagnosticLog(
          `novnc disconnect hostId=${activeHostId} sessionId=${nextSessionId} clean=${clean}`,
        );
        if (disposed || sessionIdRef.current !== nextSessionId) return;
        window.clearTimeout(connectWatchdog);
        updateStatus(clean ? 'disconnected' : 'error');
        setStatusMessage(clean ? tTextRef.current('vnc.state.disconnected') : 'VNC connection closed unexpectedly');
      });
      rfb.addEventListener('credentialsrequired', (event: Event) => {
        const types = (event as RfbEvent<{ types?: string[] }>).detail?.types ?? [];
        const missing = missingVncCredentials(credentials, types);
        writeVncDiagnosticLog(
          `novnc credentialsrequired hostId=${activeHostId} sessionId=${nextSessionId} types=${types.join(',')} missing=${missing.join(',')} ${describeRfbAuthScheme(rfb)}`,
        );
        if (hasRequiredVncCredentials(credentials, types)) {
          rfb.sendCredentials(credentials);
          return;
        }
        window.clearTimeout(connectWatchdog);
        updateStatus('error');
        setStatusMessage(`VNC server requires credentials: ${missing.join(', ') || types.join(', ') || 'unknown'}`);
      });
      rfb.addEventListener('securityfailure', (event: Event) => {
        const detail = (event as RfbEvent<{ status?: number; reason?: string }>).detail;
        const reason = detail?.reason || `security status ${detail?.status ?? 'unknown'}`;
        writeVncDiagnosticLog(
          `novnc securityfailure hostId=${activeHostId} sessionId=${nextSessionId} reason=${reason} ${describeRfbAuthScheme(rfb)}`,
        );
        window.clearTimeout(connectWatchdog);
        updateStatus('error');
        setStatusMessage(reason);
      });
      rfb.addEventListener('desktopname', (event: Event) => {
        const name = (event as RfbEvent<{ name?: string }>).detail?.name ?? '';
        writeVncDiagnosticLog(`novnc desktopname hostId=${activeHostId} sessionId=${nextSessionId} name=${name} ${describeRfbRuntime(rfb)}`);
        if (name) setStatusMessage(name);
      });
    };

    void start().catch((error: unknown) => {
      if (!disposed) {
        window.clearTimeout(connectWatchdog);
        writeVncDiagnosticLog(`invoke vnc_connect failed hostId=${activeHostId} sessionId=${nextSessionId} error=${String(error)}`);
        updateStatus('error');
        setStatusMessage(String(error));
      }
    });

    const statusInterval = window.setInterval(() => {
      void invoke<VncSessionStatus>('get_vnc_session_status', {
        request: { sessionId: nextSessionId },
      }).then((payload) => {
        if (
          !disposed
          && payload.sessionId === nextSessionId
          && !payload.connected
          && statusRef.current === 'connected'
        ) {
          writeVncDiagnosticLog(`status poll disconnected hostId=${activeHostId} sessionId=${nextSessionId}`);
          updateStatus('disconnected');
        }
      }).catch(() => undefined);
    }, 1000);

    return () => {
      writeVncDiagnosticLog(`effect cleanup hostId=${activeHostId} sessionId=${nextSessionId} status=${statusRef.current}`);
      disposed = true;
      clipboardBridge?.dispose();
      disposePerformanceDiagnostics?.();
      disposeInputDiagnostics?.();
      disposeEncodingPreferences?.();
      window.clearTimeout(connectWatchdog);
      window.clearInterval(statusInterval);
      if (sessionIdRef.current === nextSessionId) {
        rfbRef.current?.disconnect();
        rfbRef.current = null;
        clearElement(screenRef.current);
      }
      closeVncSession(nextSessionId, false);
    };
  }, [activeHostId, connectNonce, closeVncSession, updateSessionId, updateStatus, vncDebugEnabled]);

  if (!host) {
    return (
      <section className="rdp-page rdp-page-empty">
        <header
          className="rdp-toolbar rdp-toolbar-empty"
          onMouseDown={(event) => {
            if ((event.target as HTMLElement).closest('button')) return;
            void getCurrentWindow().startDragging();
          }}
        >
          <WindowControls className="rdp-window-controls" />
          <div className="rdp-target">
            <span className="rdp-target-name">{t('vnc.windowTitle')}</span>
            <span className="rdp-target-meta">{queryHostId || t('vnc.state.idle')}</span>
          </div>
        </header>
        <Empty
          image={Empty.PRESENTED_IMAGE_SIMPLE}
          description={(
            <div className="rdp-empty-copy">
              <strong>{hostsLoading ? t('vnc.loadingHost') : t('vnc.emptyTitle')}</strong>
              <span>{hostsLoading ? t('rdp.loadingHostSubtitle') : t('vnc.emptySubtitle')}</span>
              <Button type="primary" onClick={() => navigate('/terminal?assets=1')}>{t('vnc.openAssets')}</Button>
            </div>
          )}
        />
      </section>
    );
  }

  const statusLabel = t(`vnc.state.${status}` as Parameters<typeof t>[0]);
  const showConnecting = status === 'connecting';
  const showError = status === 'error';
  const defaultResolution = vncDefaultResolution();
  const targetLabel = host.ip || host.name;

  return (
    <section className="rdp-page">
      <header
        className="rdp-toolbar"
        onMouseDown={(event) => {
          if ((event.target as HTMLElement).closest('button')) return;
          void getCurrentWindow().startDragging();
        }}
      >
        <WindowControls className="rdp-window-controls" />
        <div className="rdp-target">
          <span className="rdp-target-name">{host.name}</span>
          <span className="rdp-target-meta">{host.ip}:{host.rdpSettings?.vncPort ?? 5900}</span>
        </div>
        <span className={`rdp-status-pill rdp-status-pill-${status === 'error' ? 'error' : status}`}>{statusLabel}</span>
        <div className="rdp-toolbar-actions">
          <Button size="small" icon={<ReloadOutlined />} onClick={reconnectVncSession}>{t('vnc.reconnect')}</Button>
          <Button
            size="small"
            onClick={() => rfbRef.current?.sendCtrlAltDel()}
            disabled={!sessionId || status !== 'connected' || host.rdpSettings?.vncViewOnly === true}
          >
            Ctrl Alt Del
          </Button>
          <Button size="small" icon={<CloseOutlined />} onClick={() => closeVncSession(sessionId)}>{t('vnc.disconnect')}</Button>
        </div>
      </header>
      <div
        className={`rdp-stage${showConnecting || showError ? ' rdp-stage-state' : ''}`}
        tabIndex={0}
        role="application"
        aria-label={tText('vnc.canvasAria', { name: host.name })}
        onContextMenu={(event) => event.preventDefault()}
      >
        <div
          ref={screenRef}
          className={`rdp-canvas rdp-vnc-screen${showConnecting || showError ? ' rdp-render-hidden' : ''}`}
          style={{
            width: `${defaultResolution.width}px`,
            height: `${defaultResolution.height}px`,
          }}
        />
        {showConnecting ? (
          <div className="terminal-page-state-shell">
            <section className="terminal-state-card" aria-live="polite">
              <div className="terminal-state-card-header">
                <span className="terminal-status-light terminal-status-light-connecting" aria-hidden="true" />
                <span>{t('vnc.connectingTitle')}</span>
              </div>
              <div className="terminal-state-card-body">
                <Spin size="small" />
                <div>
                  <div className="terminal-state-title">{t('vnc.connectTo', { target: targetLabel })}</div>
                  <div className="terminal-state-subtitle">{t('vnc.connectingSubtitle')}</div>
                </div>
              </div>
              <div className="terminal-state-command" aria-hidden="true">
                <span>$</span>
                <span className="terminal-state-command-text">opsbatch vnc connect --target {targetLabel}</span>
              </div>
            </section>
          </div>
        ) : null}
        {showError ? (
          <div className="terminal-page-state-shell">
            <section className="terminal-state-card terminal-state-card-error" role="alert">
              <div className="terminal-state-card-header">
                <span className="terminal-status-light terminal-status-light-error" aria-hidden="true" />
                <span>{t('vnc.connectFailed')}</span>
              </div>
              <div className="terminal-state-title">{t('vnc.sessionStartFailed')}</div>
              <p className="terminal-state-subtitle">{statusMessage || t('vnc.errorFallback')}</p>
              <div className="terminal-state-actions">
                <Button type="primary" onClick={reconnectVncSession}>{t('vnc.reconnect')}</Button>
                <Button onClick={() => navigate('/terminal?assets=1')}>{t('vnc.openAssets')}</Button>
              </div>
            </section>
          </div>
        ) : null}
      </div>
    </section>
  );
}
