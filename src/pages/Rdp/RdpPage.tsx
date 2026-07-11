import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent, MouseEvent as ReactMouseEvent, WheelEvent as ReactWheelEvent } from 'react';
import { useLocation } from 'react-router-dom';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { WebviewWindow } from '@tauri-apps/api/webviewWindow';
import { emit, listen, type UnlistenFn } from '@tauri-apps/api/event';
import { Button, Empty, Spin } from '../../components/ui';
import { CloseOutlined, ReloadOutlined, UploadOutlined } from '../../components/ui/icons';
import WindowControls from '../../components/WindowControls';
import { OPEN_ASSET_MANAGER_EVENT } from '../../utils/windowEvents';
import { useAssetsStore } from '../../stores/assets';
import { useTranslation } from '../../i18n';
import { clamp, getOpenHostRequest, getRdpKeyboardInputEvents, getRdpOverlayText, uploadFilesToRdp, type OpenHostRequest } from './rdpProtocol';
import { useRdpConnection } from './useRdpConnection';
import RdpAiPanel from '../../components/RdpAiPanel';
import { executeRdpOperations } from '../../utils/rdpAgentExecutor';
import type { RdpOperation } from '../../utils/aiActionParser';
import { BotOutlined } from '../../components/ui/icons';
import '../../styles/pages/rdp.css';

const RDP_AI_SCREENSHOT_MAX_WIDTH = 1280;
const RDP_AI_SCREENSHOT_MAX_HEIGHT = 720;
const RDP_AI_SCREENSHOT_QUALITY = 0.72;

export default function RdpPage() {
  const { t, tText } = useTranslation();
  const location = useLocation();
  const hosts = useAssetsStore((s) => s.hosts);
  const hostsLoading = useAssetsStore((s) => s.loading);
  const loadHosts = useAssetsStore((s) => s.loadHosts);
  const stateHostRequest = useMemo(() => getOpenHostRequest(location.state), [location.state]);
  const queryHostId = useMemo(() => new URLSearchParams(location.search).get('hostId')?.trim() ?? '', [location.search]);
  const hostRequest = useMemo<OpenHostRequest | undefined>(() => {
    if (stateHostRequest) return stateHostRequest;
    if (!queryHostId) return undefined;
    const host = hosts.find((item) => item.id === queryHostId);
    if (!host) return undefined;
    return {
      requestId: `rdp-window-${host.id}`,
      hostId: host.id,
      name: host.name,
      ip: host.ip,
    };
  }, [hosts, queryHostId, stateHostRequest]);
  const [connectNonce, setConnectNonce] = useState(0);
  const [aiPanelOpen, setAiPanelOpen] = useState(true);
  const [isDragOver, setIsDragOver] = useState(false);
  const [transferToast, setTransferToast] = useState<string | null>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const videoRef = useRef<HTMLVideoElement>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const pendingMoveRef = useRef<{ x: number; y: number } | null>(null);
  const moveFrameRef = useRef<number | null>(null);
  const {
    connection,
    connectionState,
    statusMessage,
    hasFrame,
    presentedFps,
    metrics,
    renderMode,
    sendInput,
    disconnectActive,
  } = useRdpConnection({
    hostRequest,
    stageRef,
    canvasRef,
    videoRef,
    connectNonce,
    invalidFrameMessage: tText('rdp.invalidFrame'),
    transportMode: 'h264Direct',
  });

  const rdpSessionId = connection?.sessionId ?? null;
  const desktopWidth = connection?.width ?? 1280;
  const desktopHeight = connection?.height ?? 720;

  const handleExecuteRdpOperations = useCallback(
    async (ops: RdpOperation[]) => {
      if (!rdpSessionId) {
        throw new Error('RDP 会话未连接，无法执行 AI 操作');
      }
      await executeRdpOperations(ops, { sessionId: rdpSessionId });
    },
    [rdpSessionId],
  );

  const captureRdpScreenshot = useCallback((): string | null => {
    const source = renderMode === 'h264Direct' ? videoRef.current : canvasRef.current;
    if (!source) return null;

    const sourceWidth = source instanceof HTMLVideoElement
      ? source.videoWidth
      : source.width;
    const sourceHeight = source instanceof HTMLVideoElement
      ? source.videoHeight
      : source.height;
    if (sourceWidth <= 0 || sourceHeight <= 0) return null;

    const scale = Math.min(
      1,
      RDP_AI_SCREENSHOT_MAX_WIDTH / sourceWidth,
      RDP_AI_SCREENSHOT_MAX_HEIGHT / sourceHeight,
    );
    const width = Math.max(1, Math.round(sourceWidth * scale));
    const height = Math.max(1, Math.round(sourceHeight * scale));
    const canvas = document.createElement('canvas');
    canvas.width = width;
    canvas.height = height;
    const context = canvas.getContext('2d');
    if (!context) return null;

    try {
      context.drawImage(source, 0, 0, width, height);
      return canvas.toDataURL('image/jpeg', RDP_AI_SCREENSHOT_QUALITY);
    } catch {
      return null;
    }
  }, [renderMode]);

  useEffect(() => {
    if (!stateHostRequest && queryHostId && hosts.length === 0) {
      void loadHosts();
    }
  }, [hosts.length, loadHosts, queryHostId, stateHostRequest]);

  // ===== 文件拖拽上传 =====
  useEffect(() => {
    if (!rdpSessionId) return;

    let dragUnlisten: UnlistenFn | undefined;
    let transferUnlisten: UnlistenFn | undefined;

    const setup = async () => {
      dragUnlisten = await getCurrentWebview().onDragDropEvent((event) => {
        const payload = event.payload as { type: string; paths?: string[]; position?: { x: number; y: number } | null };
        if (payload.type === 'enter' || payload.type === 'over') {
          if (payload.paths && payload.paths.length > 0) {
            setIsDragOver(true);
          }
        } else if (payload.type === 'leave') {
          setIsDragOver(false);
        } else if (payload.type === 'drop') {
          setIsDragOver(false);
          const paths = payload.paths ?? [];
          if (paths.length === 0 || !rdpSessionId) return;
          void handleFileDrop(paths, payload.position ?? null);
        }
      });

      transferUnlisten = await listen<{
        completed: string[];
        failed: string[];
        downloadDir: string;
      }>(`rdp-file-transfer-${rdpSessionId}`, (event) => {
        const { completed, failed, downloadDir } = event.payload;
        const parts: string[] = [];
        if (completed.length > 0) {
          parts.push(`已下载 ${completed.length} 个文件到 ${downloadDir}`);
        }
        if (failed.length > 0) {
          parts.push(`${failed.length} 个文件下载失败`);
        }
        if (parts.length > 0) {
          setTransferToast(parts.join('，'));
          setTimeout(() => setTransferToast(null), 5000);
        }
      });
    };

    void setup();

    return () => {
      dragUnlisten?.();
      transferUnlisten?.();
    };
  }, [rdpSessionId]);

  const handleFileDrop = useCallback(
    async (paths: string[], position: { x: number; y: number } | null) => {
      if (!rdpSessionId) return;
      setTransferToast(`正在上传 ${paths.length} 个文件...`);
      try {
        // 将 Tauri 物理像素坐标映射为远程桌面坐标
        let remotePos: { x: number; y: number } | null = null;
        const target = renderMode === 'h264Direct' ? videoRef.current : canvasRef.current;
        if (position && target) {
          const dpr = window.devicePixelRatio || 1;
          const cssX = position.x / dpr;
          const cssY = position.y / dpr;
          const rect = target.getBoundingClientRect();
          const remoteWidth = connection?.width ?? 0;
          const remoteHeight = connection?.height ?? 0;
          if (rect.width > 0 && rect.height > 0 && remoteWidth > 0 && remoteHeight > 0) {
            remotePos = {
              x: clamp(Math.floor((cssX - rect.left) * (remoteWidth / rect.width)), 0, remoteWidth - 1),
              y: clamp(Math.floor((cssY - rect.top) * (remoteHeight / rect.height)), 0, remoteHeight - 1),
            };
          }
        }
        // 传给后端：后端等 CLIPRDR 确认后自动点击定位 + Ctrl+V
        await uploadFilesToRdp(rdpSessionId, paths, remotePos);
        setTransferToast(`正在粘贴 ${paths.length} 个文件到远程桌面...`);
        setTimeout(() => setTransferToast(null), 4000);
      } catch (error) {
        setTransferToast(`上传失败: ${error instanceof Error ? error.message : String(error)}`);
        setTimeout(() => setTransferToast(null), 5000);
      }
    },
    [rdpSessionId, renderMode, connection?.width, connection?.height],
  );

  const getRemotePoint = useCallback((event: { clientX: number; clientY: number }) => {
    const target = renderMode === 'h264Direct' ? videoRef.current : canvasRef.current;
    if (!target) return null;

    const rect = target.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return null;
    const remoteWidth = connection?.width ?? (canvasRef.current?.width || 0);
    const remoteHeight = connection?.height ?? (canvasRef.current?.height || 0);
    if (remoteWidth === 0 || remoteHeight === 0) return null;

    return {
      x: clamp(Math.floor((event.clientX - rect.left) * (remoteWidth / rect.width)), 0, remoteWidth - 1),
      y: clamp(Math.floor((event.clientY - rect.top) * (remoteHeight / rect.height)), 0, remoteHeight - 1),
    };
  }, [connection?.height, connection?.width, renderMode]);

  const scheduleMouseMove = useCallback((point: { x: number; y: number }) => {
    pendingMoveRef.current = point;
    if (moveFrameRef.current !== null) return;

    moveFrameRef.current = window.requestAnimationFrame(() => {
      moveFrameRef.current = null;
      const next = pendingMoveRef.current;
      pendingMoveRef.current = null;
      if (next) sendInput({ type: 'mouse_move', x: next.x, y: next.y });
    });
  }, [sendInput]);

  useEffect(() => {
    return () => {
      if (moveFrameRef.current !== null) {
        window.cancelAnimationFrame(moveFrameRef.current);
        moveFrameRef.current = null;
      }
    };
  }, []);

  const handlePointerDown = useCallback((event: ReactMouseEvent<HTMLDivElement>) => {
    stageRef.current?.focus({ preventScroll: true });
    const point = getRemotePoint(event);
    if (!point) return;
    sendInput({ type: 'mouse_button', x: point.x, y: point.y, button: event.button, down: true });
  }, [getRemotePoint, sendInput]);

  const handlePointerUp = useCallback((event: ReactMouseEvent<HTMLDivElement>) => {
    const point = getRemotePoint(event);
    if (!point) return;
    sendInput({ type: 'mouse_button', x: point.x, y: point.y, button: event.button, down: false });
  }, [getRemotePoint, sendInput]);

  const handlePointerMove = useCallback((event: ReactMouseEvent<HTMLDivElement>) => {
    const point = getRemotePoint(event);
    if (point) scheduleMouseMove(point);
  }, [getRemotePoint, scheduleMouseMove]);

  const handleWheel = useCallback((event: ReactWheelEvent<HTMLDivElement>) => {
    const point = getRemotePoint(event);
    if (!point || event.deltaY === 0) return;
    event.preventDefault();
    sendInput({
      type: 'wheel',
      x: point.x,
      y: point.y,
      delta: -Math.sign(event.deltaY) * 120,
      vertical: true,
    });
  }, [getRemotePoint, sendInput]);

  const handleKey = useCallback((event: ReactKeyboardEvent<HTMLDivElement>, down: boolean) => {
    const inputEvents = getRdpKeyboardInputEvents(event.nativeEvent, down);
    if (inputEvents.length === 0) return;

    event.preventDefault();
    for (const inputEvent of inputEvents) {
      sendInput(inputEvent);
    }
  }, [sendInput]);

  const statusLabel = useMemo(() => {
    if (connectionState === 'connecting') return tText('rdp.state.connecting');
    if (connectionState === 'connected') return tText('rdp.state.connected');
    if (connectionState === 'disconnected') return tText('rdp.state.disconnected');
    if (connectionState === 'terminated') return tText('rdp.state.terminated');
    if (connectionState === 'error') return tText('rdp.state.error');
    return tText('rdp.state.idle');
  }, [connectionState, tText]);

  const disconnectAndCloseWindow = useCallback(() => {
    disconnectActive();
    void WebviewWindow.getCurrent().destroy().catch((e) => {
      console.warn('[rdp] destroy window failed:', e);
    });
  }, [disconnectActive]);

  // 关闭当前 RDP 窗口，并请求主窗口打开资产管理面板。
  const closeAndOpenAssets = useCallback(() => {
    disconnectActive();
    void emit(OPEN_ASSET_MANAGER_EVENT, {
      sourceWindowLabel: getCurrentWindow().label,
    }).catch((e) => {
      console.warn('[rdp] emit open-asset-manager failed:', e);
    });
    void WebviewWindow.getCurrent().destroy().catch((e) => {
      console.warn('[rdp] destroy window failed:', e);
    });
  }, [disconnectActive]);

  if (!hostRequest) {
    return (
      <section className="rdp-page rdp-page-empty">
        <header
          className="rdp-toolbar rdp-toolbar-empty"
          onMouseDown={(e) => {
            const target = e.target as HTMLElement;
            if (target.closest('button')) return;
            void getCurrentWindow().startDragging();
          }}
        >
          <WindowControls className="rdp-window-controls" />
          <div className="rdp-target">
            <span className="rdp-target-name">{t('rdp.windowTitle')}</span>
            <span className="rdp-target-meta">{queryHostId || t('rdp.state.idle')}</span>
          </div>
        </header>
        <Empty
          image={Empty.PRESENTED_IMAGE_SIMPLE}
          description={(
            <div className="rdp-empty-copy">
              <strong>{hostsLoading ? t('rdp.loadingHost') : t('rdp.emptyTitle')}</strong>
              <span>{hostsLoading ? t('rdp.loadingHostSubtitle') : t('rdp.emptySubtitle')}</span>
              <Button type="primary" onClick={closeAndOpenAssets}>
                {t('rdp.openAssets')}
              </Button>
            </div>
          )}
        />
      </section>
    );
  }

  const showConnecting = connectionState === 'connecting' || (connectionState === 'connected' && !hasFrame);
  const showError = connectionState === 'error' || connectionState === 'terminated';
  const overlayText = showConnecting
    ? getRdpOverlayText({ connectionState, hasFrame, statusMessage, renderMode })
    : null;

  return (
    <section className="rdp-page">
      <header
        className="rdp-toolbar"
        onMouseDown={(e) => {
          const target = e.target as HTMLElement;
          if (target.closest('button')) return;
          void getCurrentWindow().startDragging();
        }}
      >
        <WindowControls className="rdp-window-controls" />
        <div className="rdp-target">
          <span className="rdp-target-name">{hostRequest.name}</span>
          <span className="rdp-target-meta">
            {hostRequest.ip}
            {connection ? ` · ${connection.width}x${connection.height}` : null}
            {connection ? ` · ${renderMode === 'h264Direct' ? 'H.264' : 'bitmap'}` : null}
            {presentedFps !== null ? ` · ${presentedFps} FPS` : null}
            {metrics ? ` · srv ${metrics.serverUpdatesPerSecond}/s · tx ${metrics.sentFramesPerSecond}/s · ${metrics.sentMbytesPerSecond.toFixed(1)} MB/s` : null}
          </span>
        </div>
        <span className={`rdp-status-pill rdp-status-pill-${showError ? 'error' : connectionState}`}>
          {statusLabel}
        </span>
        <div className="rdp-toolbar-actions">
          <Button
            size="small"
            icon={<BotOutlined />}
            onClick={() => setAiPanelOpen((value) => !value)}
          >
            {t('rdp.aiToggle')}
          </Button>
          <Button
            size="small"
            icon={<ReloadOutlined />}
            onClick={() => setConnectNonce((value) => value + 1)}
          >
            {t('rdp.reconnect')}
          </Button>
          <Button
            size="small"
            icon={<CloseOutlined />}
            onClick={disconnectAndCloseWindow}
          >
            {t('rdp.disconnect')}
          </Button>
        </div>
      </header>

      <div className={`rdp-body ${aiPanelOpen ? 'rdp-body-with-ai' : ''}`}>
      <div
        ref={stageRef}
        className="rdp-stage"
        tabIndex={0}
        role="application"
        aria-label={tText('rdp.canvasAria', { name: hostRequest.name })}
        onMouseDown={handlePointerDown}
        onMouseUp={handlePointerUp}
        onMouseMove={handlePointerMove}
        onWheel={handleWheel}
        onKeyDown={(event) => handleKey(event, true)}
        onKeyUp={(event) => handleKey(event, false)}
        onContextMenu={(event) => event.preventDefault()}
      >
        <canvas
          ref={canvasRef}
          className={`rdp-canvas ${renderMode === 'h264Direct' ? 'rdp-render-hidden' : ''}`}
        />
        <video
          ref={videoRef}
          className={`rdp-video ${renderMode === 'h264Direct' ? '' : 'rdp-render-hidden'}`}
          autoPlay
          playsInline
          controls={false}
        />
        {showConnecting && (
          <div className="rdp-overlay">
            <Spin />
            <div>
              <strong>{overlayText?.title ?? t('rdp.connectingTitle')}</strong>
              <span>{overlayText?.subtitle ?? t('rdp.connectingSubtitle')}</span>
            </div>
          </div>
        )}
        {showError && (
          <div className="rdp-overlay rdp-overlay-error">
            <div>
              <strong>{statusMessage || t('rdp.connectFailed')}</strong>
              <span>{t('rdp.errorFallback')}</span>
            </div>
            <div className="rdp-overlay-actions">
              <Button type="primary" icon={<ReloadOutlined />} onClick={() => setConnectNonce((value) => value + 1)}>
                {t('rdp.reconnect')}
              </Button>
              <Button onClick={closeAndOpenAssets}>{t('rdp.openAssets')}</Button>
            </div>
          </div>
        )}
        {isDragOver && (
          <div className="rdp-drop-zone">
            <div className="rdp-drop-zone-inner">
              <UploadOutlined />
              <strong>松开以上传文件到远程桌面</strong>
              <span>文件将通过剪贴板粘贴到远程</span>
            </div>
          </div>
        )}
        {transferToast && (
          <div className="rdp-transfer-toast">
            {transferToast}
          </div>
        )}
      </div>
      {aiPanelOpen && hostRequest && (
        <aside className="rdp-ai-aside">
          <RdpAiPanel
            hostId={hostRequest.hostId}
            hostName={hostRequest.name}
            rdpSessionId={rdpSessionId}
            desktopWidth={desktopWidth}
            desktopHeight={desktopHeight}
            executeRdpOperations={handleExecuteRdpOperations}
            getRdpScreenshot={captureRdpScreenshot}
            onClose={() => setAiPanelOpen(false)}
          />
        </aside>
      )}
      </div>
    </section>
  );
}
