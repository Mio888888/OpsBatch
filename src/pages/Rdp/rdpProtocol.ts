import { invoke } from '@tauri-apps/api/core';

export interface OpenHostRequest {
  requestId: string;
  hostId: string;
  name: string;
  ip: string;
}

export interface RdpConnectResponse {
  sessionId: string;
  hostId: string;
  width: number;
  height: number;
}

export type RdpTransportMode = 'legacyBitmap' | 'h264Direct';

export interface RdpWebRtcOffer {
  sdp: string;
  sdpType: RTCSdpType;
}

export interface RdpRenderModeState {
  renderMode: RdpTransportMode;
  usesVideoElement: boolean;
}

export interface RdpOverlayText {
  title: string;
  subtitle: string;
}

export interface RdpStatusPayload {
  sessionId: string;
  state: RdpConnectionState;
  message?: string | null;
  detail?: RdpStatusDetail;
}

export type RdpStatusDetail =
  | { type: 'h264DirectUnavailable'; reason: string };

export interface RdpMetricsPayload {
  sessionId: string;
  serverUpdatesPerSecond: number;
  sentFramesPerSecond: number;
  coalescedUpdatesPerSecond: number;
  sentMbytesPerSecond: number;
}

export interface RdpFramePayload {
  width: number;
  height: number;
  x: number;
  y: number;
  regionWidth: number;
  regionHeight: number;
  rgba: Uint8ClampedArray;
}

export type RdpConnectionState = 'idle' | 'connecting' | 'connected' | 'disconnected' | 'terminated' | 'error' | string;

export type RdpInputEvent =
  | { type: 'mouse_move'; x: number; y: number }
  | { type: 'mouse_button'; x: number; y: number; button: number; down: boolean }
  | { type: 'wheel'; x: number; y: number; delta: number; vertical: boolean }
  | { type: 'key_scancode'; code: number; extended: boolean; down: boolean }
  | { type: 'unicode'; character: string; down: boolean };

export interface RdpKeyboardEventLike {
  key: string;
  code: string;
  ctrlKey?: boolean;
  metaKey?: boolean;
  altKey?: boolean;
  repeat?: boolean;
}

const MIN_DESKTOP_WIDTH = 640;
const MIN_DESKTOP_HEIGHT = 480;
const MAX_DESKTOP_WIDTH = 1920;
const MAX_DESKTOP_HEIGHT = 1080;
const RDP_FRAME_HEADER_BYTES = 16;
const RDP_BASE_FRAME_PAINT_BUDGET = 8;
const RDP_MAX_FRAME_PAINT_BUDGET = 24;
const RDP_LARGE_BACKLOG_FRAME_COUNT = 24;
const RDP_CONTROL_SCANCODE = { code: 0x1d, extended: false };

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

export function getOpenHostRequest(state: unknown): OpenHostRequest | undefined {
  if (!isRecord(state)) return undefined;
  return isOpenHostRequest(state.openHost) ? state.openHost : undefined;
}

export function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

export function createRdpSessionId(hostId: string) {
  return `rdp-${hostId}-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

export function getDesktopRequestSize(stage: HTMLElement | null) {
  const rect = stage?.getBoundingClientRect();
  const fallbackWidth = window.innerWidth || 1280;
  const fallbackHeight = Math.max(480, (window.innerHeight || 800) - 76);
  const width = Math.round(rect?.width || fallbackWidth);
  const height = Math.round(rect?.height || fallbackHeight);
  const scale = Math.min(1, MAX_DESKTOP_WIDTH / width, MAX_DESKTOP_HEIGHT / height);

  return {
    width: clamp(Math.round(width * scale), MIN_DESKTOP_WIDTH, MAX_DESKTOP_WIDTH),
    height: clamp(Math.round(height * scale), MIN_DESKTOP_HEIGHT, MAX_DESKTOP_HEIGHT),
  };
}

export function decodeRdpFramePayload(message: ArrayBuffer | Uint8Array): RdpFramePayload {
  const view = message instanceof Uint8Array
    ? new DataView(message.buffer, message.byteOffset, message.byteLength)
    : new DataView(message);
  if (view.byteLength < RDP_FRAME_HEADER_BYTES) {
    throw new Error('RDP frame header is incomplete');
  }

  const width = view.getUint16(0, true);
  const height = view.getUint16(2, true);
  const x = view.getUint16(4, true);
  const y = view.getUint16(6, true);
  const regionWidth = view.getUint16(8, true);
  const regionHeight = view.getUint16(10, true);
  const rgbaLength = view.getUint32(12, true);
  const expectedLength = regionWidth * regionHeight * 4;
  const bodyOffset = (message instanceof Uint8Array ? message.byteOffset : 0) + RDP_FRAME_HEADER_BYTES;
  const buffer = message instanceof Uint8Array ? message.buffer : message;

  if (rgbaLength !== expectedLength || view.byteLength !== RDP_FRAME_HEADER_BYTES + rgbaLength) {
    throw new Error('RDP frame payload size is invalid');
  }

  return {
    width,
    height,
    x,
    y,
    regionWidth,
    regionHeight,
    rgba: new Uint8ClampedArray(buffer, bodyOffset, rgbaLength),
  };
}

export function drainRdpFrameBatch<T>(queue: T[], budget: number): T[] {
  if (!Number.isFinite(budget) || budget <= 0 || queue.length === 0) {
    return [];
  }

  return queue.splice(0, Math.min(queue.length, Math.floor(budget)));
}

export function getRdpFramePaintBudget(queueLength: number) {
  if (!Number.isFinite(queueLength) || queueLength <= 0) {
    return 0;
  }
  if (queueLength <= RDP_BASE_FRAME_PAINT_BUDGET) {
    return Math.floor(queueLength);
  }
  if (queueLength <= RDP_LARGE_BACKLOG_FRAME_COUNT) {
    return RDP_BASE_FRAME_PAINT_BUDGET;
  }

  return Math.min(RDP_MAX_FRAME_PAINT_BUDGET, Math.ceil(queueLength / 2));
}

export function resolveRdpRenderMode({
  requested,
  webRtcReady,
  bitmapFallbackActive = false,
}: {
  requested: RdpTransportMode;
  webRtcReady: boolean;
  bitmapFallbackActive?: boolean;
}): RdpRenderModeState {
  if (bitmapFallbackActive) {
    return {
      renderMode: 'legacyBitmap',
      usesVideoElement: false,
    };
  }

  if (requested === 'h264Direct' && webRtcReady) {
    return {
      renderMode: 'h264Direct',
      usesVideoElement: true,
    };
  }

  return {
    renderMode: 'legacyBitmap',
    usesVideoElement: false,
  };
}

export function shouldAttachRdpMediaTrack(kind: string) {
  return kind === 'video' || kind === 'audio';
}

export function isH264DirectUnavailableStatus(
  payload: RdpStatusPayload,
): payload is RdpStatusPayload & { detail: { type: 'h264DirectUnavailable'; reason: string } } {
  return payload.detail?.type === 'h264DirectUnavailable';
}

export function resolveH264UnavailableFallback(
  payload: RdpStatusPayload,
  activeTransportMode: RdpTransportMode,
) {
  if (!isH264DirectUnavailableStatus(payload) || activeTransportMode !== 'h264Direct') {
    return null;
  }

  return {
    nextTransportMode: 'legacyBitmap' as const,
    requiresFreshSession: true,
    statusMessage: `${payload.detail.reason} 正在切换到 legacy bitmap。`,
  };
}

export function getRdpHandshakeStatusMessage(transportMode: RdpTransportMode) {
  return transportMode === 'h264Direct'
    ? '正在进行 RDP TCP、TLS、NLA 与 EGFX 握手'
    : '正在进行 RDP TCP、TLS 与 NLA 握手';
}

export function getRdpStartupStatusMessage(transportMode: RdpTransportMode) {
  return transportMode === 'h264Direct'
    ? '正在创建 WebRTC H.264/音频通道'
    : getRdpHandshakeStatusMessage(transportMode);
}

export function getRdpOverlayText({
  connectionState,
  hasFrame,
  statusMessage,
  renderMode = 'h264Direct',
}: {
  connectionState: RdpConnectionState;
  hasFrame: boolean;
  statusMessage?: string;
  renderMode?: RdpTransportMode;
}): RdpOverlayText {
  if (connectionState === 'connected' && !hasFrame) {
    return {
      title: '正在等待远程画面',
      subtitle: statusMessage || (
        renderMode === 'h264Direct'
          ? 'RDP 已连接，正在等待 RDPGFX H.264 首帧。'
          : 'RDP 已连接，正在等待 bitmap/ClearCodec 首帧。'
      ),
    };
  }

  return {
    title: '正在建立 RDP 连接',
    subtitle: statusMessage || '正在进行 TCP、TLS 与 NLA 握手，请稍候。',
  };
}

export async function withRdpTimeout<T>(
  promise: Promise<T>,
  timeoutMs: number,
  message: string,
): Promise<T> {
  let timeoutId: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<never>((_, reject) => {
    timeoutId = setTimeout(() => reject(new Error(message)), timeoutMs);
  });

  try {
    return await Promise.race([promise, timeout]);
  } finally {
    if (timeoutId !== undefined) {
      clearTimeout(timeoutId);
    }
  }
}

export function getScancodeForKey(
  key: string,
  code = '',
): { code: number; extended: boolean } | undefined {
  const codeMap: Record<string, { code: number; extended: boolean }> = {
    Backquote: { code: 0x29, extended: false },
    Digit1: { code: 0x02, extended: false },
    Digit2: { code: 0x03, extended: false },
    Digit3: { code: 0x04, extended: false },
    Digit4: { code: 0x05, extended: false },
    Digit5: { code: 0x06, extended: false },
    Digit6: { code: 0x07, extended: false },
    Digit7: { code: 0x08, extended: false },
    Digit8: { code: 0x09, extended: false },
    Digit9: { code: 0x0a, extended: false },
    Digit0: { code: 0x0b, extended: false },
    Minus: { code: 0x0c, extended: false },
    Equal: { code: 0x0d, extended: false },
    KeyQ: { code: 0x10, extended: false },
    KeyW: { code: 0x11, extended: false },
    KeyE: { code: 0x12, extended: false },
    KeyR: { code: 0x13, extended: false },
    KeyT: { code: 0x14, extended: false },
    KeyY: { code: 0x15, extended: false },
    KeyU: { code: 0x16, extended: false },
    KeyI: { code: 0x17, extended: false },
    KeyO: { code: 0x18, extended: false },
    KeyP: { code: 0x19, extended: false },
    BracketLeft: { code: 0x1a, extended: false },
    BracketRight: { code: 0x1b, extended: false },
    KeyA: { code: 0x1e, extended: false },
    KeyS: { code: 0x1f, extended: false },
    KeyD: { code: 0x20, extended: false },
    KeyF: { code: 0x21, extended: false },
    KeyG: { code: 0x22, extended: false },
    KeyH: { code: 0x23, extended: false },
    KeyJ: { code: 0x24, extended: false },
    KeyK: { code: 0x25, extended: false },
    KeyL: { code: 0x26, extended: false },
    Semicolon: { code: 0x27, extended: false },
    Quote: { code: 0x28, extended: false },
    Backslash: { code: 0x2b, extended: false },
    KeyZ: { code: 0x2c, extended: false },
    KeyX: { code: 0x2d, extended: false },
    KeyC: { code: 0x2e, extended: false },
    KeyV: { code: 0x2f, extended: false },
    KeyB: { code: 0x30, extended: false },
    KeyN: { code: 0x31, extended: false },
    KeyM: { code: 0x32, extended: false },
    Comma: { code: 0x33, extended: false },
    Period: { code: 0x34, extended: false },
    Slash: { code: 0x35, extended: false },
    Space: { code: 0x39, extended: false },
    NumpadEnter: { code: 0x1c, extended: true },
    ControlLeft: { code: 0x1d, extended: false },
    ControlRight: { code: 0x1d, extended: true },
    ShiftLeft: { code: 0x2a, extended: false },
    ShiftRight: { code: 0x36, extended: false },
    AltLeft: { code: 0x38, extended: false },
    AltRight: { code: 0x38, extended: true },
    MetaLeft: { code: 0x5b, extended: true },
    MetaRight: { code: 0x5c, extended: true },
  };
  if (codeMap[code]) {
    return codeMap[code];
  }

  const keyMap: Record<string, { code: number; extended: boolean }> = {
    Backspace: { code: 0x0e, extended: false },
    Tab: { code: 0x0f, extended: false },
    Enter: { code: 0x1c, extended: false },
    Escape: { code: 0x01, extended: false },
    Delete: { code: 0x53, extended: true },
    Insert: { code: 0x52, extended: true },
    Home: { code: 0x47, extended: true },
    End: { code: 0x4f, extended: true },
    PageUp: { code: 0x49, extended: true },
    PageDown: { code: 0x51, extended: true },
    ArrowUp: { code: 0x48, extended: true },
    ArrowDown: { code: 0x50, extended: true },
    ArrowLeft: { code: 0x4b, extended: true },
    ArrowRight: { code: 0x4d, extended: true },
    F1: { code: 0x3b, extended: false },
    F2: { code: 0x3c, extended: false },
    F3: { code: 0x3d, extended: false },
    F4: { code: 0x3e, extended: false },
    F5: { code: 0x3f, extended: false },
    F6: { code: 0x40, extended: false },
    F7: { code: 0x41, extended: false },
    F8: { code: 0x42, extended: false },
    F9: { code: 0x43, extended: false },
    F10: { code: 0x44, extended: false },
    F11: { code: 0x57, extended: false },
    F12: { code: 0x58, extended: false },
    Shift: { code: 0x2a, extended: false },
    Control: { code: 0x1d, extended: false },
    Alt: { code: 0x38, extended: false },
    ' ': { code: 0x39, extended: false },
  };

  return keyMap[key];
}

export function getRdpKeyboardInputEvents(
  event: RdpKeyboardEventLike,
  down: boolean,
): RdpInputEvent[] {
  if (event.key === 'Meta') {
    return [];
  }

  const special = getScancodeForKey(event.key, event.code);
  const hasModifier = Boolean(event.ctrlKey || event.metaKey || event.altKey);
  const keyScancode = event.key.length === 1
    ? getScancodeForKey(event.key, event.code)
    : special;

  if (event.metaKey && keyScancode && event.key.length === 1) {
    const keyEvent: RdpInputEvent = { type: 'key_scancode', ...keyScancode, down };
    if (event.repeat && down) {
      return [keyEvent];
    }

    return down
      ? [
        { type: 'key_scancode', ...RDP_CONTROL_SCANCODE, down: true },
        keyEvent,
      ]
      : [
        keyEvent,
        { type: 'key_scancode', ...RDP_CONTROL_SCANCODE, down: false },
      ];
  }

  if (hasModifier && keyScancode) {
    return [{ type: 'key_scancode', ...keyScancode, down }];
  }

  if (special) {
    return [{ type: 'key_scancode', ...special, down }];
  }

  if (event.key.length === 1) {
    return [{ type: 'unicode', character: event.key, down }];
  }

  return [];
}

export function getErrorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

export async function disconnectRdpSession(sessionId: string) {
  try {
    await invoke('rdp_disconnect', { sessionId });
  } catch {
    // 会话可能尚未完成握手或已由后端关闭，清理阶段忽略即可。
  }
}

export async function uploadFilesToRdp(
  sessionId: string,
  paths: string[],
  position?: { x: number; y: number } | null,
): Promise<void> {
  // 后端 Rust 元组 (u16, u16) 需要 JSON 数组 [x, y]，不是对象
  const posTuple = position ? [position.x, position.y] : null;
  await invoke('rdp_upload_files', { sessionId, paths, position: posTuple });
}

export async function closeRdpWebRtcSession(sessionId: string) {
  try {
    await invoke('rdp_webrtc_close', { sessionId });
  } catch {
    // WebRTC 会话可能没有创建成功，清理阶段忽略即可。
  }
}
