import { useCallback, useEffect, useRef, useState } from 'react';
import type { RefObject } from 'react';
import { Channel, invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import {
  createRdpSessionId,
  closeRdpWebRtcSession,
  decodeRdpFramePayload,
  disconnectRdpSession,
  drainRdpFrameBatch,
  getDesktopRequestSize,
  getErrorMessage,
  getRdpHandshakeStatusMessage,
  getRdpStartupStatusMessage,
  resolveH264UnavailableFallback,
  resolveRdpRenderMode,
  shouldAttachRdpVideoTrack,
  withRdpTimeout,
  type OpenHostRequest,
  type RdpConnectResponse,
  type RdpConnectionState,
  type RdpFramePayload,
  type RdpInputEvent,
  type RdpMetricsPayload,
  type RdpStatusPayload,
  type RdpTransportMode,
  type RdpWebRtcOffer,
} from './rdpProtocol';

interface UseRdpConnectionOptions {
  hostRequest?: OpenHostRequest;
  stageRef: RefObject<HTMLDivElement | null>;
  canvasRef: RefObject<HTMLCanvasElement | null>;
  videoRef: RefObject<HTMLVideoElement | null>;
  connectNonce: number;
  invalidFrameMessage: string;
  transportMode?: RdpTransportMode;
}

const RDP_CANVAS_CONTEXT_OPTIONS: CanvasRenderingContext2DSettings = {
  alpha: false,
  desynchronized: true,
};
const RDP_MAX_FRAMES_PER_PAINT = 8;
const RDP_LISTENER_SETUP_TIMEOUT_MS = 5_000;
const WEBRTC_SIGNALING_TIMEOUT_MS = 8_000;
const RDP_CONNECT_TIMEOUT_MS = 35_000;

export function useRdpConnection({
  hostRequest,
  stageRef,
  canvasRef,
  videoRef,
  connectNonce,
  invalidFrameMessage,
  transportMode = 'h264Direct',
}: UseRdpConnectionOptions) {
  const [connection, setConnection] = useState<RdpConnectResponse | null>(null);
  const [connectionState, setConnectionState] = useState<RdpConnectionState>('idle');
  const [statusMessage, setStatusMessage] = useState<string | undefined>();
  const [hasFrame, setHasFrame] = useState(false);
  const [presentedFps, setPresentedFps] = useState<number | null>(null);
  const [metrics, setMetrics] = useState<RdpMetricsPayload | null>(null);
  const [renderMode, setRenderMode] = useState<RdpTransportMode>('legacyBitmap');
  const connectionAttemptKey = `${hostRequest?.requestId ?? ''}:${hostRequest?.hostId ?? ''}:${connectNonce}:${transportMode}`;
  const [transportOverride, setTransportOverride] = useState<{
    attemptKey: string;
    transportMode: RdpTransportMode;
  } | null>(null);
  const effectiveTransportMode = transportOverride?.attemptKey === connectionAttemptKey
    ? transportOverride.transportMode
    : transportMode;
  const activeSessionIdRef = useRef<string | null>(null);
  const peerConnectionRef = useRef<RTCPeerConnection | null>(null);
  const hasFrameRef = useRef(false);
  const bitmapFallbackActiveRef = useRef(false);
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
    const batch = drainRdpFrameBatch(frames, RDP_MAX_FRAMES_PER_PAINT);

    let drewFrameCount = 0;
    for (const frame of batch) {
      if (drawFrame(frame)) {
        drewFrameCount += 1;
      }
    }

    if (frames.length > 0) {
      drawFrameRequestRef.current = window.requestAnimationFrame(flushFrames);
    }

    if (drewFrameCount > 0 && !hasFrameRef.current) {
      hasFrameRef.current = true;
      setHasFrame(true);
    }
    if (
      drewFrameCount > 0
      && effectiveTransportMode === 'h264Direct'
      && !bitmapFallbackActiveRef.current
    ) {
      bitmapFallbackActiveRef.current = true;
      setRenderMode('legacyBitmap');
    }
    if (drewFrameCount > 0) {
      const now = performance.now();
      const stats = frameStatsRef.current;
      stats.frames += drewFrameCount;
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
  }, [drawFrame, effectiveTransportMode]);

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
    if (sessionId) {
      void disconnectRdpSession(sessionId);
      void closeRdpWebRtcSession(sessionId);
    }
    peerConnectionRef.current?.close();
    peerConnectionRef.current = null;
    if (videoRef.current) videoRef.current.srcObject = null;
    setConnectionState('disconnected');
    setStatusMessage(undefined);
  }, [videoRef]);

  useEffect(() => {
    if (!hostRequest) {
      activeSessionIdRef.current = null;
      setConnection(null);
      setConnectionState('idle');
      setStatusMessage(undefined);
      setHasFrame(false);
      setPresentedFps(null);
      setMetrics(null);
      setRenderMode('legacyBitmap');
      hasFrameRef.current = false;
      bitmapFallbackActiveRef.current = false;
      pendingFramesRef.current = [];
      frameStatsRef.current = { frames: 0, lastUpdate: 0 };
      canvasContextRef.current = null;
      return undefined;
    }

    let disposed = false;
    const unlistenFns: UnlistenFn[] = [];
    const sessionId = createRdpSessionId(hostRequest.hostId);
    const { width, height } = getDesktopRequestSize(stageRef.current);
    let peerConnection: RTCPeerConnection | null = null;
    let activeTransportMode: RdpTransportMode = effectiveTransportMode;
    let frameChannel: Channel<ArrayBuffer | Uint8Array> | null = null;
    let h264UnavailableFallbackStarted = false;

    activeSessionIdRef.current = sessionId;
    setConnection({ sessionId, hostId: hostRequest.hostId, width, height });
    setConnectionState('connecting');
    setStatusMessage(getRdpStartupStatusMessage(effectiveTransportMode));
    setHasFrame(false);
    setPresentedFps(null);
    setMetrics(null);
    setRenderMode('legacyBitmap');
    hasFrameRef.current = false;
    bitmapFallbackActiveRef.current = false;
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
    if (videoRef.current) {
      videoRef.current.srcObject = null;
    }

    const markVideoFrameVisible = () => {
      bitmapFallbackActiveRef.current = false;
      setRenderMode('h264Direct');
      if (hasFrameRef.current) return;
      hasFrameRef.current = true;
      setHasFrame(true);
    };

    const waitForIceGatheringComplete = (pc: RTCPeerConnection) => new Promise<void>((resolve, reject) => {
      if (pc.iceGatheringState === 'complete') {
        resolve();
        return;
      }

      const timeout = window.setTimeout(() => {
        pc.removeEventListener('icegatheringstatechange', handleStateChange);
        reject(new Error('WebRTC ICE gathering timed out'));
      }, 5000);

      function handleStateChange() {
        if (pc.iceGatheringState !== 'complete') return;
        window.clearTimeout(timeout);
        pc.removeEventListener('icegatheringstatechange', handleStateChange);
        resolve();
      }

      pc.addEventListener('icegatheringstatechange', handleStateChange);
    });

    const createWebRtcPeer = async () => {
      setStatusMessage('正在创建 WebRTC H.264/音频通道');
      const pc = new RTCPeerConnection();
      peerConnection = pc;
      peerConnectionRef.current = pc;
      pc.ontrack = (event) => {
        if (!shouldAttachRdpVideoTrack(event.track.kind)) return;
        const video = videoRef.current;
        if (!video) return;
        const [stream] = event.streams;
        video.srcObject = stream ?? new MediaStream([event.track]);
        video.onloadeddata = markVideoFrameVisible;
        video.onplaying = markVideoFrameVisible;
        void video.play().catch(() => {
          setStatusMessage(getErrorMessage('WebRTC video autoplay was blocked'));
        });
      };

      setStatusMessage('正在等待后端 WebRTC offer');
      const offer = await invoke<RdpWebRtcOffer>('rdp_webrtc_create_offer', { sessionId });
      setStatusMessage('正在生成浏览器 WebRTC answer');
      await pc.setRemoteDescription({ type: offer.sdpType, sdp: offer.sdp });
      const answer = await pc.createAnswer();
      await pc.setLocalDescription(answer);
      setStatusMessage('正在收集浏览器 ICE candidate');
      await waitForIceGatheringComplete(pc);
      const localDescription = pc.localDescription;
      if (!localDescription?.sdp) {
        throw new Error('WebRTC local answer is empty');
      }
      setStatusMessage('正在提交 WebRTC answer');
      await invoke('rdp_webrtc_set_answer', {
        sessionId,
        answerSdp: localDescription.sdp,
      });
    };

    const start = async () => {
      const statusEventName = `rdp-status-${sessionId}`;
      const metricsEventName = `rdp-metrics-${sessionId}`;
      const connectRdpTransport = async (nextTransportMode: RdpTransportMode) => {
        if (!frameChannel) {
          throw new Error('RDP frame channel is not ready');
        }
        const handshakeMessage = getRdpHandshakeStatusMessage(nextTransportMode);
        setStatusMessage(handshakeMessage);
        const response = await withRdpTimeout(
          invoke<RdpConnectResponse>('rdp_connect', {
            request: {
              hostId: hostRequest.hostId,
              sessionId,
              width,
              height,
              transportMode: nextTransportMode,
            },
            frameChannel,
          }),
          RDP_CONNECT_TIMEOUT_MS,
          'RDP 连接超时',
        );

        return response;
      };
      const statusUnlisten = await withRdpTimeout(
        listen<RdpStatusPayload>(statusEventName, (event) => {
          if (disposed) return;
          const h264Fallback = resolveH264UnavailableFallback(event.payload, activeTransportMode);
          if (h264Fallback) {
            h264UnavailableFallbackStarted = true;
            activeTransportMode = h264Fallback.nextTransportMode;
            bitmapFallbackActiveRef.current = true;
            hasFrameRef.current = false;
            setHasFrame(false);
            setRenderMode(h264Fallback.nextTransportMode);
            setStatusMessage(h264Fallback.statusMessage);
            peerConnection?.close();
            peerConnection = null;
            peerConnectionRef.current = null;
            if (videoRef.current) videoRef.current.srcObject = null;
            void closeRdpWebRtcSession(sessionId);
            void disconnectRdpSession(sessionId);
            setTransportOverride({
              attemptKey: connectionAttemptKey,
              transportMode: h264Fallback.nextTransportMode,
            });
            return;
          }
          setConnectionState(event.payload.state);
          setStatusMessage(event.payload.message ?? undefined);
        }),
        RDP_LISTENER_SETUP_TIMEOUT_MS,
        `RDP 状态监听注册超时: ${statusEventName}`,
      );
      unlistenFns.push(statusUnlisten);

      const metricsUnlisten = await withRdpTimeout(
        listen<RdpMetricsPayload>(metricsEventName, (event) => {
          if (disposed) return;
          setMetrics(event.payload);
        }),
        RDP_LISTENER_SETUP_TIMEOUT_MS,
        `RDP 指标监听注册超时: ${metricsEventName}`,
      );
      unlistenFns.push(metricsUnlisten);

      frameChannel = new Channel<ArrayBuffer | Uint8Array>((message) => {
        if (disposed) return;
        try {
          const payload = decodeRdpFramePayload(message);
          enqueueFrame(payload);
        } catch {
          setStatusMessage(invalidFrameMessage);
        }
      });

      if (disposed) return;
      if (effectiveTransportMode === 'h264Direct') {
        try {
          await withRdpTimeout(
            createWebRtcPeer(),
            WEBRTC_SIGNALING_TIMEOUT_MS,
            'WebRTC H.264 信令超时，已回退 legacy bitmap',
          );
          const nextMode = resolveRdpRenderMode({
            requested: effectiveTransportMode,
            webRtcReady: true,
          });
          setRenderMode(nextMode.renderMode);
        } catch (error) {
          peerConnection?.close();
          peerConnection = null;
          peerConnectionRef.current = null;
          await closeRdpWebRtcSession(sessionId);
          activeTransportMode = 'legacyBitmap';
          setRenderMode('legacyBitmap');
          setStatusMessage(getErrorMessage(error));
        }
      }

      const response = await connectRdpTransport(activeTransportMode);

      if (disposed) {
        await disconnectRdpSession(response.sessionId);
        await closeRdpWebRtcSession(response.sessionId);
        return;
      }
      if (h264UnavailableFallbackStarted) {
        return;
      }

      setConnection(response);
      setConnectionState('connected');
      setStatusMessage(undefined);
      const nextCanvas = canvasRef.current;
      if (nextCanvas && !hasFrameRef.current) {
        nextCanvas.width = response.width;
        nextCanvas.height = response.height;
      }
    };

    void start().catch((error: unknown) => {
      if (disposed) return;
      peerConnection?.close();
      peerConnection = null;
      peerConnectionRef.current = null;
      void closeRdpWebRtcSession(sessionId);
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
      peerConnection?.close();
      peerConnectionRef.current = null;
      if (videoRef.current) videoRef.current.srcObject = null;
      void closeRdpWebRtcSession(sessionId);
      void disconnectRdpSession(sessionId);
    };
  }, [
    canvasRef,
    connectionAttemptKey,
    effectiveTransportMode,
    enqueueFrame,
    hostRequest,
    invalidFrameMessage,
    stageRef,
    videoRef,
  ]);

  return {
    connection,
    connectionState,
    statusMessage,
    hasFrame,
    presentedFps,
    metrics,
    renderMode,
    sendInput,
    disconnectActive,
  };
}
