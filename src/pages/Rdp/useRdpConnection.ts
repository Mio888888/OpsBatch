import { useCallback, useEffect, useRef, useState } from 'react';
import type { RefObject } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import {
  createRdpSessionId,
  disconnectRdpSession,
  getDesktopRequestSize,
  getErrorMessage,
  type OpenHostRequest,
  type RdpConnectResponse,
  type RdpConnectionState,
  type RdpFramePayload,
  type RdpInputEvent,
  type RdpStatusPayload,
} from './rdpProtocol';

interface UseRdpConnectionOptions {
  hostRequest?: OpenHostRequest;
  stageRef: RefObject<HTMLDivElement | null>;
  canvasRef: RefObject<HTMLCanvasElement | null>;
  connectNonce: number;
  invalidFrameMessage: string;
}

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
  const activeSessionIdRef = useRef<string | null>(null);

  const drawFrame = useCallback((payload: RdpFramePayload) => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    if (canvas.width !== payload.width) canvas.width = payload.width;
    if (canvas.height !== payload.height) canvas.height = payload.height;

    const expectedLength = payload.regionWidth * payload.regionHeight * 4;
    if (payload.rgba.length !== expectedLength) {
      setStatusMessage(invalidFrameMessage);
      return;
    }

    const context = canvas.getContext('2d');
    if (!context) return;

    const imageData = new ImageData(
      new Uint8ClampedArray(payload.rgba),
      payload.regionWidth,
      payload.regionHeight,
    );
    context.putImageData(imageData, payload.x, payload.y);
    setHasFrame(true);
  }, [canvasRef, invalidFrameMessage]);

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

    const canvas = canvasRef.current;
    if (canvas) {
      canvas.width = width;
      canvas.height = height;
      canvas.getContext('2d')?.clearRect(0, 0, width, height);
    }

    const start = async () => {
      const statusUnlisten = await listen<RdpStatusPayload>(`rdp-status-${sessionId}`, (event) => {
        if (disposed) return;
        setConnectionState(event.payload.state);
        setStatusMessage(event.payload.message ?? undefined);
      });
      unlistenFns.push(statusUnlisten);

      const frameUnlisten = await listen<RdpFramePayload>(`rdp-frame-${sessionId}`, (event) => {
        if (!disposed) drawFrame(event.payload);
      });
      unlistenFns.push(frameUnlisten);

      if (disposed) return;

      const response = await invoke<RdpConnectResponse>('rdp_connect', {
        request: {
          hostId: hostRequest.hostId,
          sessionId,
          width,
          height,
        },
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
      activeSessionIdRef.current = null;
      void disconnectRdpSession(sessionId);
    };
  }, [canvasRef, connectNonce, drawFrame, hostRequest, stageRef]);

  return {
    connection,
    connectionState,
    statusMessage,
    hasFrame,
    sendInput,
    disconnectActive,
  };
}
