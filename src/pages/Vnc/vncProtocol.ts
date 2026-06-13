export type VncConnectionState = 'idle' | 'connecting' | 'connected' | 'disconnected' | 'error';

export interface VncSessionStatus {
  sessionId: string;
  connected: boolean;
}

const VNC_DEFAULT_WIDTH = 1920;
const VNC_DEFAULT_HEIGHT = 1080;
const VNC_MAX_WIDTH = 1920;
const VNC_MAX_HEIGHT = 1080;

export function vncDefaultResolution() {
  return { width: VNC_DEFAULT_WIDTH, height: VNC_DEFAULT_HEIGHT };
}

export function vncResolutionLimit(width: number, height: number) {
  return {
    width: Math.min(VNC_MAX_WIDTH, Math.max(1, Math.round(width))),
    height: Math.min(VNC_MAX_HEIGHT, Math.max(1, Math.round(height))),
  };
}

export function vncPresentationSize(width: number, height: number) {
  const normalizedWidth = Math.max(1, Math.round(width));
  const normalizedHeight = Math.max(1, Math.round(height));
  const scale = Math.min(
    1,
    VNC_MAX_WIDTH / normalizedWidth,
    VNC_MAX_HEIGHT / normalizedHeight,
  );
  return {
    width: Math.max(1, Math.round(normalizedWidth * scale)),
    height: Math.max(1, Math.round(normalizedHeight * scale)),
  };
}

export function createVncSessionId(hostId: string) {
  return `vnc-${hostId}-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}
