import type { Host } from '../types';

export const MIN_RDP_DESKTOP_WIDTH = 640;
export const MIN_RDP_DESKTOP_HEIGHT = 480;
export const MAX_RDP_DESKTOP_WIDTH = 3840;
export const MAX_RDP_DESKTOP_HEIGHT = 2160;
export const DEFAULT_VNC_PORT = 5900;

export interface HostRdpSettingsFormValues {
  rdpDomain?: string;
  rdpDesktopWidth?: number;
  rdpDesktopHeight?: number;
  rdpEnableClipboard?: boolean;
  rdpEnableAudio?: boolean;
  rdpMapDisk?: boolean;
  rdpDiskPath?: string;
  vncPort?: number;
  vncUsername?: string;
  vncPassword?: string;
  vncViewOnly?: boolean;
  vncShared?: boolean;
}

export function normalizeRdpDesktopValue(value: unknown, min: number, max: number) {
  if (typeof value !== 'number' || !Number.isFinite(value)) return undefined;
  return Math.min(max, Math.max(min, Math.round(value)));
}

export function buildRdpSettings(
  values: HostRdpSettingsFormValues,
  os: Host['os'],
): Host['rdpSettings'] {
  if (os === 'vnc') {
    const settings: NonNullable<Host['rdpSettings']> = {
      protocol: 'vnc',
      vncPort: normalizeVncPort(values.vncPort),
    };
    if (values.vncUsername?.trim()) {
      settings.vncUsername = values.vncUsername.trim();
    }
    if (values.vncPassword?.trim()) {
      settings.vncPassword = values.vncPassword.trim();
    }
    settings.vncViewOnly = values.vncViewOnly ?? false;
    settings.vncShared = values.vncShared ?? true;
    return settings;
  }

  if (os !== 'windows') return undefined;

  const domain = values.rdpDomain?.trim();
  const desktopWidth = normalizeRdpDesktopValue(
    values.rdpDesktopWidth,
    MIN_RDP_DESKTOP_WIDTH,
    MAX_RDP_DESKTOP_WIDTH,
  );
  const desktopHeight = normalizeRdpDesktopValue(
    values.rdpDesktopHeight,
    MIN_RDP_DESKTOP_HEIGHT,
    MAX_RDP_DESKTOP_HEIGHT,
  );
  const settings: NonNullable<Host['rdpSettings']> = {};

  settings.protocol = 'rdp';
  if (domain) settings.domain = domain;
  if (desktopWidth) settings.desktopWidth = desktopWidth;
  if (desktopHeight) settings.desktopHeight = desktopHeight;
  settings.enableClipboard = values.rdpEnableClipboard ?? true;
  settings.enableAudio = values.rdpEnableAudio ?? true;
  settings.mapDisk = values.rdpMapDisk ?? false;
  if (settings.mapDisk && values.rdpDiskPath?.trim()) {
    settings.diskPath = values.rdpDiskPath.trim();
  }

  return Object.keys(settings).length > 0 ? settings : undefined;
}

export function normalizeVncPort(value: unknown) {
  if (typeof value !== 'number' || !Number.isFinite(value)) return DEFAULT_VNC_PORT;
  return Math.min(65535, Math.max(1, Math.round(value)));
}

export function isVncRemoteDesktopHost(host: Pick<Host, 'rdpSettings'>) {
  return host.rdpSettings?.protocol === 'vnc';
}
