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

export interface RdpFramePayload {
  sessionId: string;
  width: number;
  height: number;
  x: number;
  y: number;
  regionWidth: number;
  regionHeight: number;
  rgba: number[];
}

export type RdpConnectionState = 'idle' | 'connecting' | 'connected' | 'disconnected' | 'terminated' | 'error' | string;

export type RdpInputEvent =
  | { type: 'mouse_move'; x: number; y: number }
  | { type: 'mouse_button'; x: number; y: number; button: number; down: boolean }
  | { type: 'wheel'; x: number; y: number; delta: number; vertical: boolean }
  | { type: 'key_scancode'; code: number; extended: boolean; down: boolean }
  | { type: 'unicode'; character: string; down: boolean };

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

  return {
    width: clamp(width, 640, 3840),
    height: clamp(height, 480, 2160),
  };
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
