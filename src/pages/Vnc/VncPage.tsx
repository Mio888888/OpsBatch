import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent, MouseEvent as ReactMouseEvent } from 'react';
import { useNavigate, useLocation } from 'react-router-dom';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Button, Empty } from '../../components/ui';
import { CloseOutlined, ReloadOutlined } from '../../components/ui/icons';
import WindowControls from '../../components/WindowControls';
import { useAssetsStore } from '../../stores/assets';
import { useTranslation } from '../../i18n';

interface VncStatusPayload {
  session_id: string;
  state: 'idle' | 'connecting' | 'connected' | 'disconnected' | 'error' | string;
  message: string;
}

type VncFramePayload =
  | { type: 'Resize'; session_id: string; width: number; height: number }
  | { type: 'Raw'; session_id: string; x: number; y: number; width: number; height: number; data: number[] | Uint8Array }
  | { type: 'Copy'; session_id: string; dst_x: number; dst_y: number; src_x: number; src_y: number; width: number; height: number };

function createVncSessionId(hostId: string) {
  return `vnc-${hostId}-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

function getKeyboardKeycode(event: ReactKeyboardEvent<HTMLDivElement>) {
  if (event.key.length === 1) return event.key.charCodeAt(0);
  const special: Record<string, number> = {
    Backspace: 0xff08,
    Tab: 0xff09,
    Enter: 0xff0d,
    Escape: 0xff1b,
    Delete: 0xffff,
    ArrowLeft: 0xff51,
    ArrowUp: 0xff52,
    ArrowRight: 0xff53,
    ArrowDown: 0xff54,
  };
  return special[event.key] ?? 0;
}

function getMouseMask(event: ReactMouseEvent<HTMLDivElement>, down: boolean) {
  if (event.type === 'mousemove') return event.buttons;
  if (!down) return event.buttons;
  if (event.buttons > 0) return event.buttons;
  if (event.button === 0) return 1;
  if (event.button === 1) return 2;
  if (event.button === 2) return 4;
  return 0;
}

export default function VncPage() {
  const { t, tText } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const hosts = useAssetsStore((s) => s.hosts);
  const hostsLoading = useAssetsStore((s) => s.loading);
  const loadHosts = useAssetsStore((s) => s.loadHosts);
  const queryHostId = useMemo(() => new URLSearchParams(location.search).get('hostId')?.trim() ?? '', [location.search]);
  const host = hosts.find((item) => item.id === queryHostId);
  const [sessionId, setSessionId] = useState('');
  const [status, setStatus] = useState<VncStatusPayload['state']>('idle');
  const [statusMessage, setStatusMessage] = useState('');
  const [connectNonce, setConnectNonce] = useState(0);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const viewOnly = host?.rdpSettings?.vncViewOnly === true;

  useEffect(() => {
    if (queryHostId && hosts.length === 0) void loadHosts();
  }, [hosts.length, loadHosts, queryHostId]);

  useEffect(() => {
    if (!host) return;
    const nextSessionId = createVncSessionId(host.id);
    let disposed = false;
    let unlistenFns: UnlistenFn[] = [];
    setSessionId(nextSessionId);
    setStatus('connecting');
    setStatusMessage(tText('vnc.state.connecting'));

    const drawFrame = (payload: VncFramePayload) => {
      const canvas = canvasRef.current;
      const context = canvas?.getContext('2d');
      if (!canvas || !context) return;

      if (payload.type === 'Resize') {
        canvas.width = payload.width;
        canvas.height = payload.height;
        return;
      }

      if (payload.type === 'Raw') {
        const image = new ImageData(
          new Uint8ClampedArray(payload.data),
          payload.width,
          payload.height,
        );
        context.putImageData(image, payload.x, payload.y);
        return;
      }

      const copy = context.getImageData(payload.src_x, payload.src_y, payload.width, payload.height);
      context.putImageData(copy, payload.dst_x, payload.dst_y);
    };

    const start = async () => {
      const statusUnlisten = await listen<VncStatusPayload>(`vnc-status-${nextSessionId}`, (event) => {
        if (disposed) return;
        setStatus(event.payload.state);
        setStatusMessage(event.payload.message);
      });
      const frameUnlisten = await listen<VncFramePayload>(`vnc-frame-${nextSessionId}`, (event) => {
        if (!disposed) drawFrame(event.payload);
      });
      unlistenFns = [statusUnlisten, frameUnlisten];
      await invoke('vnc_connect', { hostId: host.id, sessionId: nextSessionId });
    };

    void start().catch((error: unknown) => {
      if (!disposed) {
        setStatus('error');
        setStatusMessage(String(error));
      }
    });

    return () => {
      disposed = true;
      unlistenFns.forEach((unlisten) => unlisten());
      void invoke('vnc_disconnect', { sessionId: nextSessionId });
    };
  }, [connectNonce, host, tText]);

  const sendMouse = useCallback((event: ReactMouseEvent<HTMLDivElement>, down: boolean) => {
    if (!sessionId || viewOnly) return;
    const canvas = canvasRef.current;
    if (!canvas) return;
    const rect = canvas.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return;
    const x = Math.max(0, Math.min(canvas.width - 1, Math.floor((event.clientX - rect.left) * (canvas.width / rect.width))));
    const y = Math.max(0, Math.min(canvas.height - 1, Math.floor((event.clientY - rect.top) * (canvas.height / rect.height))));
    void invoke('vnc_send_input', {
      sessionId,
      event: { type: 'mouse', x, y, buttons: getMouseMask(event, down) },
    });
  }, [sessionId, viewOnly]);

  const sendKey = useCallback((event: ReactKeyboardEvent<HTMLDivElement>, down: boolean) => {
    const keycode = getKeyboardKeycode(event);
    if (!sessionId || viewOnly || keycode === 0) return;
    event.preventDefault();
    void invoke('vnc_send_input', { sessionId, event: { type: 'key', keycode, down } });
  }, [sessionId, viewOnly]);

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
  const showOverlay = status !== 'connected';

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
          <Button size="small" icon={<ReloadOutlined />} onClick={() => setConnectNonce((value) => value + 1)}>{t('vnc.reconnect')}</Button>
          <Button size="small" icon={<CloseOutlined />} onClick={() => void invoke('vnc_disconnect', { sessionId })}>{t('vnc.disconnect')}</Button>
        </div>
      </header>
      <div
        className="rdp-stage"
        tabIndex={0}
        role="application"
        aria-label={tText('vnc.canvasAria', { name: host.name })}
        onMouseDown={(event) => sendMouse(event, true)}
        onMouseUp={(event) => sendMouse(event, false)}
        onMouseMove={(event) => sendMouse(event, true)}
        onKeyDown={(event) => sendKey(event, true)}
        onKeyUp={(event) => sendKey(event, false)}
        onContextMenu={(event) => event.preventDefault()}
      >
        <canvas ref={canvasRef} className="rdp-canvas" />
        {showOverlay ? (
          <div className="rdp-overlay">
            <div className="rdp-overlay-card">
              <strong>{statusLabel}</strong>
              <span>{statusMessage}</span>
            </div>
          </div>
        ) : null}
      </div>
    </section>
  );
}
