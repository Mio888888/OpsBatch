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

export interface RdpStatusPayload {
  sessionId: string;
  state: RdpConnectionState;
  message?: string | null;
}

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

const MIN_DESKTOP_WIDTH = 640;
const MIN_DESKTOP_HEIGHT = 480;
const MAX_DESKTOP_WIDTH = 1920;
const MAX_DESKTOP_HEIGHT = 1080;
const RDP_FRAME_HEADER_BYTES = 16;

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

export function getScancodeForKey(key: string): { code: number; extended: boolean } | undefined {
  const map: Record<string, { code: number; extended: boolean }> = {
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
    Shift: { code: 0x2a, extended: false },
    Control: { code: 0x1d, extended: false },
    Alt: { code: 0x38, extended: false },
  };

  return map[key];
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
