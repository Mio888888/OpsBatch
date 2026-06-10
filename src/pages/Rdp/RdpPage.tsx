import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent, MouseEvent as ReactMouseEvent, WheelEvent as ReactWheelEvent } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import { Button, Empty, Spin } from '../../components/ui';
import { CloseOutlined, ReloadOutlined } from '../../components/ui/icons';
import { useTranslation } from '../../i18n';
import { clamp, getOpenHostRequest, getScancodeForKey } from './rdpProtocol';
import { useRdpConnection } from './useRdpConnection';

export default function RdpPage() {
  const { t, tText } = useTranslation();
  const location = useLocation();
  const navigate = useNavigate();
  const hostRequest = useMemo(() => getOpenHostRequest(location.state), [location.state]);
  const [connectNonce, setConnectNonce] = useState(0);
  const canvasRef = useRef<HTMLCanvasElement>(null);
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
    sendInput,
    disconnectActive,
  } = useRdpConnection({
    hostRequest,
    stageRef,
    canvasRef,
    connectNonce,
    invalidFrameMessage: tText('rdp.invalidFrame'),
  });

  const getRemotePoint = useCallback((event: { clientX: number; clientY: number }) => {
    const canvas = canvasRef.current;
    if (!canvas || canvas.width === 0 || canvas.height === 0) return null;

    const rect = canvas.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return null;

    return {
      x: clamp(Math.floor((event.clientX - rect.left) * (canvas.width / rect.width)), 0, canvas.width - 1),
      y: clamp(Math.floor((event.clientY - rect.top) * (canvas.height / rect.height)), 0, canvas.height - 1),
    };
  }, []);

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
    const special = getScancodeForKey(event.key);
    if (special) {
      event.preventDefault();
      sendInput({ type: 'key_scancode', ...special, down });
      return;
    }

    if (event.key.length === 1 && !event.metaKey) {
      event.preventDefault();
      sendInput({ type: 'unicode', character: event.key, down });
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

  if (!hostRequest) {
    return (
      <section className="rdp-page rdp-page-empty">
        <Empty
          image={Empty.PRESENTED_IMAGE_SIMPLE}
          description={(
            <div className="rdp-empty-copy">
              <strong>{t('rdp.emptyTitle')}</strong>
              <span>{t('rdp.emptySubtitle')}</span>
              <Button type="primary" onClick={() => navigate('/terminal?assets=1')}>
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

  return (
    <section className="rdp-page">
      <header className="rdp-toolbar">
        <div className="rdp-target">
          <span className="rdp-target-name">{hostRequest.name}</span>
          <span className="rdp-target-meta">
            {hostRequest.ip}
            {connection ? ` · ${connection.width}x${connection.height}` : null}
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
            icon={<ReloadOutlined />}
            onClick={() => setConnectNonce((value) => value + 1)}
          >
            {t('rdp.reconnect')}
          </Button>
          <Button
            size="small"
            icon={<CloseOutlined />}
            onClick={disconnectActive}
          >
            {t('rdp.disconnect')}
          </Button>
        </div>
      </header>

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
        <canvas ref={canvasRef} className="rdp-canvas" />
        {showConnecting && (
          <div className="rdp-overlay">
            <Spin />
            <div>
              <strong>{t('rdp.connectingTitle')}</strong>
              <span>{statusMessage || t('rdp.connectingSubtitle')}</span>
            </div>
          </div>
        )}
        {showError && (
          <div className="rdp-overlay rdp-overlay-error">
            <div>
              <strong>{statusMessage || t('rdp.connectFailed')}</strong>
              <span>{t('rdp.errorFallback')}</span>
            </div>
            <Button type="primary" icon={<ReloadOutlined />} onClick={() => setConnectNonce((value) => value + 1)}>
              {t('rdp.reconnect')}
            </Button>
          </div>
        )}
      </div>
    </section>
  );
}
