import { useCallback, useEffect, useRef, useState } from 'react';
import type { RefObject } from 'react';
import { Channel, invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import {
  createRdpSessionId,
  decodeRdpFramePayload,
  disconnectRdpSession,
  getDesktopRequestSize,
  getErrorMessage,
  type OpenHostRequest,
  type RdpConnectResponse,
  type RdpConnectionState,
  type RdpFramePayload,
  type RdpInputEvent,
  type RdpMetricsPayload,
  type RdpStatusPayload,
} from './rdpProtocol';

interface UseRdpConnectionOptions {
  hostRequest?: OpenHostRequest;
  stageRef: RefObject<HTMLDivElement | null>;
  canvasRef: RefObject<HTMLCanvasElement | null>;
  connectNonce: number;
  invalidFrameMessage: string;
}

const RDP_CANVAS_CONTEXT_OPTIONS: CanvasRenderingContext2DSettings = {
  alpha: false,
  desynchronized: true,
};

export function useRdpConnection({
  hostRequest,
  stageRef,
  canvasRef,
  connectNonce,
  invalidFrameMessage,
}: UseRdpConnectionOptions) {
  const [connection, setConnection] = useState<RdpConnectResponse | null>(null);
  const [connectionState, setConnectionState] = useState<RdpConnectionState>('idle');
  const [statusMessage, setStatusMessage] = useState<string | undefined>();
  const [hasFrame, setHasFrame] = useState(false);
  const [presentedFps, setPresentedFps] = useState<number | null>(null);
  const [metrics, setMetrics] = useState<RdpMetricsPayload | null>(null);
  const activeSessionIdRef = useRef<string | null>(null);
  const hasFrameRef = useRef(false);
  const pendingFramesRef = useRef<RdpFramePayload[]>([]);
  const drawFrameRequestRef = useRef<number | null>(null);
  const frameStatsRef = useRef({ frames: 0, lastUpdate: 0 });
  const canvasContextRef = useRef<CanvasRenderingContext2D | null>(null);

  const getCanvasContext = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return null;

    const cached = canvasContextRef.current;
    if (cached?.canvas === canvas) {
      return { canvas, context: cached };
    }

    const context = canvas.getContext('2d', RDP_CANVAS_CONTEXT_OPTIONS);
    canvasContextRef.current = context;
    return context ? { canvas, context } : null;
  }, [canvasRef]);

  const drawFrame = useCallback((payload: RdpFramePayload) => {
    const canvasContext = getCanvasContext();
    if (!canvasContext) return false;
    const { canvas, context } = canvasContext;

    if (canvas.width !== payload.width) canvas.width = payload.width;
    if (canvas.height !== payload.height) canvas.height = payload.height;

    const expectedLength = payload.regionWidth * payload.regionHeight * 4;
    if (payload.rgba.length !== expectedLength) {
      setStatusMessage(invalidFrameMessage);
      return false;
    }

    const imageData = new ImageData(payload.rgba, payload.regionWidth, payload.regionHeight);
    context.putImageData(imageData, payload.x, payload.y);
    return true;
  }, [getCanvasContext, invalidFrameMessage]);

  const flushFrames = useCallback(() => {
    drawFrameRequestRef.current = null;
    const frames = pendingFramesRef.current;
    pendingFramesRef.current = [];

    let drewFrame = false;
    for (const frame of frames) {
      drewFrame = drawFrame(frame) || drewFrame;
    }

    if (drewFrame && !hasFrameRef.current) {
      hasFrameRef.current = true;
      setHasFrame(true);
    }
    if (drewFrame) {
      const now = performance.now();
      const stats = frameStatsRef.current;
      stats.frames += 1;
      if (stats.lastUpdate === 0) {
        stats.lastUpdate = now;
      }
      const elapsed = now - stats.lastUpdate;
      if (elapsed >= 1000) {
        setPresentedFps(Math.round((stats.frames * 1000) / elapsed));
        stats.frames = 0;
        stats.lastUpdate = now;
      }
    }
  }, [drawFrame]);

  const enqueueFrame = useCallback((payload: RdpFramePayload) => {
    pendingFramesRef.current.push(payload);
    if (drawFrameRequestRef.current !== null) return;

    drawFrameRequestRef.current = window.requestAnimationFrame(flushFrames);
  }, [flushFrames]);

  const sendInput = useCallback((event: RdpInputEvent) => {
    const sessionId = activeSessionIdRef.current;
    if (!sessionId || connectionState !== 'connected') return;
    void invoke('rdp_send_input', { sessionId, event }).catch((error: unknown) => {
      setStatusMessage(getErrorMessage(error));
    });
  }, [connectionState]);

  const disconnectActive = useCallback(() => {
    const sessionId = activeSessionIdRef.current;
    if (sessionId) void disconnectRdpSession(sessionId);
    setConnectionState('disconnected');
    setStatusMessage(undefined);
  }, []);

  useEffect(() => {
    if (!hostRequest) {
      activeSessionIdRef.current = null;
      setConnection(null);
      setConnectionState('idle');
      setStatusMessage(undefined);
      setHasFrame(false);
      setPresentedFps(null);
      setMetrics(null);
      hasFrameRef.current = false;
      pendingFramesRef.current = [];
      frameStatsRef.current = { frames: 0, lastUpdate: 0 };
      canvasContextRef.current = null;
      return undefined;
    }

    let disposed = false;
    const unlistenFns: UnlistenFn[] = [];
    const sessionId = createRdpSessionId(hostRequest.hostId);
    const { width, height } = getDesktopRequestSize(stageRef.current);

    activeSessionIdRef.current = sessionId;
    setConnection({ sessionId, hostId: hostRequest.hostId, width, height });
    setConnectionState('connecting');
    setStatusMessage(undefined);
    setHasFrame(false);
    setPresentedFps(null);
    setMetrics(null);
    hasFrameRef.current = false;
    pendingFramesRef.current = [];
    frameStatsRef.current = { frames: 0, lastUpdate: 0 };

    const canvas = canvasRef.current;
    if (canvas) {
      canvas.width = width;
      canvas.height = height;
      const context = canvas.getContext('2d', RDP_CANVAS_CONTEXT_OPTIONS);
      canvasContextRef.current = context;
      context?.clearRect(0, 0, width, height);
    }

    const start = async () => {
      const statusUnlisten = await listen<RdpStatusPayload>(`rdp-status-${sessionId}`, (event) => {
        if (disposed) return;
        setConnectionState(event.payload.state);
        setStatusMessage(event.payload.message ?? undefined);
      });
      unlistenFns.push(statusUnlisten);

      const metricsUnlisten = await listen<RdpMetricsPayload>(`rdp-metrics-${sessionId}`, (event) => {
        if (disposed) return;
        setMetrics(event.payload);
      });
      unlistenFns.push(metricsUnlisten);

      const frameChannel = new Channel<ArrayBuffer | Uint8Array>((message) => {
        if (disposed) return;
        try {
          enqueueFrame(decodeRdpFramePayload(message));
        } catch {
          setStatusMessage(invalidFrameMessage);
        }
      });

      if (disposed) return;

      const response = await invoke<RdpConnectResponse>('rdp_connect', {
        request: {
          hostId: hostRequest.hostId,
          sessionId,
          width,
          height,
        },
        frameChannel,
      });

      if (disposed) {
        await disconnectRdpSession(response.sessionId);
        return;
      }

      setConnection(response);
      setConnectionState('connected');
      setStatusMessage(undefined);
      const nextCanvas = canvasRef.current;
      if (nextCanvas) {
        nextCanvas.width = response.width;
        nextCanvas.height = response.height;
      }
    };

    void start().catch((error: unknown) => {
      if (disposed) return;
      setConnectionState('error');
      setStatusMessage(getErrorMessage(error));
    });

    return () => {
      disposed = true;
      unlistenFns.forEach((unlisten) => unlisten());
      if (drawFrameRequestRef.current !== null) {
        window.cancelAnimationFrame(drawFrameRequestRef.current);
        drawFrameRequestRef.current = null;
      }
      pendingFramesRef.current = [];
      activeSessionIdRef.current = null;
      void disconnectRdpSession(sessionId);
    };
  }, [canvasRef, connectNonce, enqueueFrame, hostRequest, invalidFrameMessage, stageRef]);

  return {
    connection,
    connectionState,
    statusMessage,
    hasFrame,
    presentedFps,
    metrics,
    sendInput,
    disconnectActive,
  };
}
