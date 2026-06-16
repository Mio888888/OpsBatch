import type { ReactNode } from 'react';
import type { TranslationKey } from '../../i18n';
import type { Host, ProxySettings } from '../../types';
import type { DataNode } from '../ui';
import { isVncRemoteDesktopHost } from '../../utils/rdpSettings';
import {
  ApartmentOutlined,
  CodeOutlined,
  GithubOutlined,
} from '../ui/icons';

export const DEFAULT_GROUP_ID = '__default__';
export const DEFAULT_SSH_PORT = 22;
export const DEFAULT_RDP_PORT = 3389;
export const GROUP_DROP_ID_PREFIX = 'asset-group:';
export const HOST_DRAG_ID_PREFIX = 'asset-host:';
export const SECRET_PLACEHOLDER = '***keychain***';

export const workModes: Array<{ key: string; icon: ReactNode; labelKey: TranslationKey }> = [
  { key: '/terminal', icon: <CodeOutlined />, labelKey: 'nav.terminal' },
  { key: '/workflow', icon: <ApartmentOutlined />, labelKey: 'nav.workflow' },
  { key: '/github', icon: <GithubOutlined />, labelKey: 'nav.github' },
];

export interface HostTreeNode extends DataNode {
  children?: HostTreeNode[];
  nodeType: 'group' | 'host';
  groupId?: string;
  host?: Host;
  depth?: number;
}

export interface HostFormValues {
  name: string;
  ip: string;
  port: number;
  authType: Host['authType'];
  username: string;
  password?: string;
  privateKey?: string;
  os: Host['os'];
  tags?: string[];
  groupId?: string;
  remark?: string;
  jumpChain?: string[];
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
  vncAuthMethod?: 'vnc' | 'ard';
  vncViewOnly?: boolean;
  vncShared?: boolean;
  proxyEnabled?: boolean;
  proxyType?: ProxySettings['type'];
  proxyHost?: string;
  proxyPort?: number;
  proxyUsername?: string;
  proxyPassword?: string;
}

export interface AppUpdateInfo {
  hasUpdate: boolean;
  currentVersion: string;
  latestVersion?: string | null;
  releaseTitle?: string | null;
  releaseNotes?: string | null;
  publishedAt?: string | null;
  releaseUrl: string;
}

export type UpdateInstallPhase = 'idle' | 'checking' | 'downloading' | 'installing' | 'ready' | 'error';

export interface UpdateInstallState {
  phase: UpdateInstallPhase;
  downloaded: number;
  total?: number;
  error?: string;
}

export function formatUpdateBytes(value?: number): string {
  if (!value || value <= 0) return '-';
  const units = ['B', 'KB', 'MB', 'GB'];
  let size = value;
  let unitIndex = 0;
  while (size >= 1024 && unitIndex < units.length - 1) {
    size /= 1024;
    unitIndex += 1;
  }
  const precision = size >= 100 || unitIndex === 0 ? 0 : 1;
  return `${size.toFixed(precision)} ${units[unitIndex]}`;
}

export function editableSecret(value?: string): string | undefined {
  return value && value !== SECRET_PLACEHOLDER ? value : undefined;
}

export function submittedSecret(value?: string): string | undefined {
  const trimmed = value?.trim();
  return trimmed && trimmed !== SECRET_PLACEHOLDER ? value : undefined;
}

export function secretDebugState(value?: string) {
  return {
    present: Boolean(value),
    placeholder: value === SECRET_PLACEHOLDER,
    length: value?.length ?? 0,
  };
}

export function hostUsesStoredSecret(host: Pick<Host, 'authType' | 'password' | 'privateKey' | 'jumpChain'>) {
  if (host.authType === 'password' && host.password === SECRET_PLACEHOLDER) return true;
  if (host.authType === 'key' && host.privateKey === SECRET_PLACEHOLDER) return true;
  return host.jumpChain.length > 0;
}

export function hostFormStoresSecret(host: Pick<Host, 'password' | 'privateKey' | 'rdpSettings' | 'proxySettings'>) {
  return Boolean(
    host.password
    || host.privateKey
    || (host.rdpSettings?.vncPassword && host.rdpSettings.vncPassword !== SECRET_PLACEHOLDER)
    || (host.proxySettings?.password && host.proxySettings.password !== SECRET_PLACEHOLDER),
  );
}

export function isWindowsHost(host: Pick<Host, 'os'>) {
  return host.os === 'windows';
}

export function isVncHost(host: Pick<Host, 'os' | 'rdpSettings'>) {
  return host.os === 'vnc' || isVncRemoteDesktopHost(host);
}

export function getLinuxHostIds(selectedIds: string[], hosts: Host[]) {
  const selectedIdSet = new Set(selectedIds);
  return hosts
    .filter((host) => selectedIdSet.has(host.id) && host.os === 'linux')
    .map((host) => host.id);
}

export function getGroupDropId(groupId: string) {
  return `${GROUP_DROP_ID_PREFIX}${groupId}`;
}

export function getHostDragId(hostId: string) {
  return `${HOST_DRAG_ID_PREFIX}${hostId}`;
}

export function getGroupIdFromDropId(id: string) {
  return id.startsWith(GROUP_DROP_ID_PREFIX) ? id.slice(GROUP_DROP_ID_PREFIX.length) : null;
}

export function getHostIdFromDragId(id: string) {
  return id.startsWith(HOST_DRAG_ID_PREFIX) ? id.slice(HOST_DRAG_ID_PREFIX.length) : null;
}

export function getActiveMode(pathname: string) {
  if (pathname === '/terminal') return '/terminal';
  if (pathname === '/rdp') return '/terminal';
  if (pathname === '/vnc') return '/terminal';
  if (pathname === '/workflow') return '/workflow';
  if (pathname === '/github') return '/github';
  if (pathname === '/assets' || pathname === '/commands' || pathname === '/quick-actions') return '';
  return '';
}
