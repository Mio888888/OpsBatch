import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import type { Host, AssetGroup, HostMonitorNetwork, HostMonitorSnapshot, RdpSettings, Tag } from '../types';
import { parseProxySettings, serializeProxySettings } from '../utils/proxySettings';

interface BackendHost {
  id: string;
  name: string;
  ip: string;
  port: number;
  auth_type: string;
  username: string;
  password?: string;
  private_key?: string;
  os: string;
  tags: string;
  group_id?: string;
  remark: string;
  status: string;
  jump_chain: string;
  rdp_settings?: string | null;
  proxy_settings?: string | null;
  created_at: string;
  updated_at: string;
}

interface BackendHostMonitorNetwork {
  interface: string;
  rx_bytes: number;
  tx_bytes: number;
}

interface BackendHostMonitorSnapshot {
  timestamp: number;
  uptime: string | null;
  load_average: string | null;
  cpu_percent: number | null;
  cpu_time_used: number | null;
  cpu_time_total: number | null;
  memory_used_mb: number | null;
  memory_total_mb: number | null;
  swap_used_mb: number | null;
  swap_total_mb: number | null;
  os: string | null;
  kernel: string | null;
  processes: HostMonitorSnapshot['processes'];
  network: BackendHostMonitorNetwork | null;
  networks?: BackendHostMonitorNetwork[] | null;
  ping_ms: number | null;
  filesystems: HostMonitorSnapshot['filesystems'];
}

interface BackendAssetGroup {
  id: string;
  name: string;
  parent_id?: string;
  sort_order: number;
}

interface BackendTag {
  id: string;
  name: string;
  color: string;
  icon?: string;
}

export interface ImportHostsCsvResult {
  total: number;
  imported: number;
  skipped: number;
  errors: string[];
}

function getErrorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function parseJsonArray(value: string | undefined, fallback: string[] = []) {
  if (!value) return fallback;
  try {
    const parsed = JSON.parse(value);
    return Array.isArray(parsed) ? parsed : fallback;
  } catch {
    return fallback;
  }
}

function secretDebugState(value?: string) {
  return {
    present: Boolean(value),
    placeholder: value === '***keychain***',
    length: value?.length ?? 0,
  };
}

function mapHostMonitorNetwork(network: BackendHostMonitorNetwork): HostMonitorNetwork {
  return {
    interface: network.interface,
    rxBytes: network.rx_bytes,
    txBytes: network.tx_bytes,
  };
}

function parseRdpSettings(value?: string | null): RdpSettings | undefined {
  if (!value) return undefined;
  try {
    const parsed = JSON.parse(value) as Partial<RdpSettings>;
    const settings: RdpSettings = {};
    if (parsed.protocol === 'rdp' || parsed.protocol === 'vnc') {
      settings.protocol = parsed.protocol;
    }
    if (typeof parsed.domain === 'string' && parsed.domain.trim()) {
      settings.domain = parsed.domain.trim();
    }
    if (typeof parsed.desktopWidth === 'number' && Number.isFinite(parsed.desktopWidth)) {
      settings.desktopWidth = parsed.desktopWidth;
    }
    if (typeof parsed.desktopHeight === 'number' && Number.isFinite(parsed.desktopHeight)) {
      settings.desktopHeight = parsed.desktopHeight;
    }
    if (typeof parsed.enableClipboard === 'boolean') {
      settings.enableClipboard = parsed.enableClipboard;
    }
    if (typeof parsed.enableAudio === 'boolean') {
      settings.enableAudio = parsed.enableAudio;
    }
    if (typeof parsed.mapDisk === 'boolean') {
      settings.mapDisk = parsed.mapDisk;
    }
    if (typeof parsed.diskPath === 'string' && parsed.diskPath.trim()) {
      settings.diskPath = parsed.diskPath.trim();
    }
    if (typeof parsed.vncPort === 'number' && Number.isFinite(parsed.vncPort)) {
      settings.vncPort = parsed.vncPort;
    }
    if (typeof parsed.vncUsername === 'string' && parsed.vncUsername.trim()) {
      settings.vncUsername = parsed.vncUsername.trim();
    }
    if (typeof parsed.vncPassword === 'string' && parsed.vncPassword.trim()) {
      settings.vncPassword = parsed.vncPassword.trim();
    }
    if (typeof parsed.vncViewOnly === 'boolean') {
      settings.vncViewOnly = parsed.vncViewOnly;
    }
    if (typeof parsed.vncShared === 'boolean') {
      settings.vncShared = parsed.vncShared;
    }
    return Object.keys(settings).length > 0 ? settings : undefined;
  } catch {
    return undefined;
  }
}

function serializeRdpSettings(settings?: RdpSettings): string {
  const normalized: RdpSettings = {};
  if (settings?.protocol === 'rdp' || settings?.protocol === 'vnc') {
    normalized.protocol = settings.protocol;
  }
  if (settings?.domain?.trim()) normalized.domain = settings.domain.trim();
  if (typeof settings?.desktopWidth === 'number' && Number.isFinite(settings.desktopWidth)) {
    normalized.desktopWidth = settings.desktopWidth;
  }
  if (typeof settings?.desktopHeight === 'number' && Number.isFinite(settings.desktopHeight)) {
    normalized.desktopHeight = settings.desktopHeight;
  }
  if (typeof settings?.enableClipboard === 'boolean') {
    normalized.enableClipboard = settings.enableClipboard;
  }
  if (typeof settings?.enableAudio === 'boolean') {
    normalized.enableAudio = settings.enableAudio;
  }
  if (typeof settings?.mapDisk === 'boolean') {
    normalized.mapDisk = settings.mapDisk;
  }
  if (settings?.diskPath?.trim()) {
    normalized.diskPath = settings.diskPath.trim();
  }
  if (typeof settings?.vncPort === 'number' && Number.isFinite(settings.vncPort)) {
    normalized.vncPort = settings.vncPort;
  }
  if (settings?.vncUsername?.trim()) {
    normalized.vncUsername = settings.vncUsername.trim();
  }
  if (settings?.vncPassword?.trim()) {
    normalized.vncPassword = settings.vncPassword.trim();
  }
  if (typeof settings?.vncViewOnly === 'boolean') {
    normalized.vncViewOnly = settings.vncViewOnly;
  }
  if (typeof settings?.vncShared === 'boolean') {
    normalized.vncShared = settings.vncShared;
  }
  return JSON.stringify(normalized);
}

function mapBackendHost(h: BackendHost): Host {
  return {
    id: h.id,
    name: h.name,
    ip: h.ip,
    port: h.port,
    authType: h.auth_type as Host['authType'],
    username: h.username,
    password: h.password,
    privateKey: h.private_key,
    os: h.os as Host['os'],
    tags: parseJsonArray(h.tags),
    groupId: h.group_id ?? undefined,
    remark: h.remark,
    jumpChain: parseJsonArray(h.jump_chain),
    rdpSettings: parseRdpSettings(h.rdp_settings),
    proxySettings: parseProxySettings(h.proxy_settings),
    createdAt: h.created_at,
    updatedAt: h.updated_at,
  };
}

function upsertHost(hosts: Host[], host: Host) {
  const index = hosts.findIndex((item) => item.id === host.id);
  if (index === -1) {
    return [...hosts, host].sort((a, b) => a.name.localeCompare(b.name));
  }

  const next = hosts.slice();
  next[index] = host;
  return next;
}

function mapBackendGroup(g: BackendAssetGroup): AssetGroup {
  return {
    id: g.id,
    name: g.name,
    parentId: g.parent_id ?? undefined,
    sortOrder: g.sort_order,
  };
}

function upsertGroup(groups: AssetGroup[], group: AssetGroup) {
  const index = groups.findIndex((item) => item.id === group.id);
  if (index === -1) {
    return [...groups, group].sort((a, b) => a.sortOrder - b.sortOrder || a.name.localeCompare(b.name));
  }
  const next = groups.slice();
  next[index] = group;
  return next.sort((a, b) => a.sortOrder - b.sortOrder || a.name.localeCompare(b.name));
}

function mapBackendTag(tag: BackendTag): Tag {
  return {
    id: tag.id,
    name: tag.name,
    color: tag.color,
    icon: tag.icon ?? '',
  };
}

function upsertTag(tags: Tag[], tag: Tag) {
  const index = tags.findIndex((item) => item.id === tag.id);
  if (index === -1) {
    return [...tags, tag].sort((a, b) => a.name.localeCompare(b.name));
  }
  const next = tags.slice();
  next[index] = tag;
  return next.sort((a, b) => a.name.localeCompare(b.name));
}

interface AssetsState {
  hosts: Host[];
  groups: AssetGroup[];
  defaultGroupName: string;
  tags: Tag[];
  selectedHostIds: string[];
  loading: boolean;

  loadHosts: () => Promise<void>;
  addHost: (host: Omit<Host, 'id' | 'createdAt' | 'updatedAt'>) => Promise<void>;
  updateHost: (host: Host) => Promise<void>;
  deleteHost: (id: string) => Promise<void>;
  importHostsCsv: (csvContent: string, mode?: 'incremental' | 'replace') => Promise<ImportHostsCsvResult>;
  exportHostsCsv: () => Promise<string>;
  setSelectedHostIds: (ids: string[]) => void;
  toggleHostSelection: (id: string) => void;
  selectAllHosts: () => void;
  clearSelection: () => void;

  loadGroups: () => Promise<void>;
  addGroup: (name: string, parentId?: string) => Promise<void>;
  updateGroup: (group: AssetGroup) => Promise<void>;
  deleteGroup: (id: string) => Promise<void>;
  setDefaultGroupName: (name: string) => void;

  loadTags: () => Promise<void>;
  addTag: (name: string, color: string, icon?: string) => Promise<void>;
  updateTag: (tag: Tag) => Promise<void>;
  deleteTag: (id: string) => Promise<void>;

  getHostMonitorSnapshot: (id: string) => Promise<HostMonitorSnapshot>;
}

export const useAssetsStore = create<AssetsState>((set, get) => ({
  hosts: [],
  groups: [],
  defaultGroupName: localStorage.getItem('defaultGroupName') ?? '',
  tags: [],
  selectedHostIds: [],
  loading: false,

  loadHosts: async () => {
    set({ loading: true });
    try {
      const raw = await invoke<BackendHost[]>('list_hosts');
      const hosts = raw.map(mapBackendHost);
      set({ hosts });
    } catch (e) {
      console.error('Failed to load hosts:', e);
      set({ hosts: [] });
    } finally {
      set({ loading: false });
    }
  },

  addHost: async (host) => {
    const backend = {
      name: host.name,
      ip: host.ip,
      port: host.port,
      auth_type: host.authType,
      username: host.username,
      password: host.password,
      private_key: host.privateKey,
      os: host.os,
      tags: JSON.stringify(host.tags ?? []),
      group_id: host.groupId ?? null,
      remark: host.remark,
      jump_chain: JSON.stringify(host.jumpChain ?? []),
      rdp_settings: serializeRdpSettings(host.rdpSettings),
      proxy_settings: serializeProxySettings(host.proxySettings),
    };
    try {
      const created = await invoke<BackendHost>('add_host', { host: backend });
      set((s) => ({ hosts: upsertHost(s.hosts, mapBackendHost(created)) }));
    } catch (error: unknown) {
      throw new Error(getErrorMessage(error));
    }
  },

  updateHost: async (host) => {
    const backend = {
      id: host.id,
      name: host.name,
      ip: host.ip,
      port: host.port,
      auth_type: host.authType,
      username: host.username,
      password: host.password,
      private_key: host.privateKey,
      os: host.os,
      tags: JSON.stringify(host.tags ?? []),
      group_id: host.groupId ?? null,
      remark: host.remark,
      jump_chain: JSON.stringify(host.jumpChain ?? []),
      rdp_settings: serializeRdpSettings(host.rdpSettings),
      proxy_settings: serializeProxySettings(host.proxySettings),
    };
    console.info('[host-secret] invoke update_host', {
      hostId: host.id,
      authType: host.authType,
      password: secretDebugState(host.password),
      privateKey: secretDebugState(host.privateKey),
    });
    try {
      const updated = await invoke<BackendHost>('update_host', { host: backend });
      console.info('[host-secret] update_host succeeded', { hostId: host.id });
      set((s) => ({ hosts: upsertHost(s.hosts, mapBackendHost(updated)) }));
    } catch (error: unknown) {
      console.error('[host-secret] update_host failed', {
        hostId: host.id,
        error: getErrorMessage(error),
      });
      throw new Error(getErrorMessage(error));
    }
  },

  deleteHost: async (id) => {
    try {
      await invoke('delete_host', { id });
      set((s) => ({
        hosts: s.hosts.filter((host) => host.id !== id),
        selectedHostIds: s.selectedHostIds.filter((h) => h !== id),
      }));
    } catch (error: unknown) {
      throw new Error(getErrorMessage(error));
    }
  },

  importHostsCsv: async (csvContent, mode = 'incremental') => {
    try {
      const result = await invoke<ImportHostsCsvResult>('import_hosts_csv', { csvContent, mode });
      await get().loadHosts();
      return result;
    } catch (error: unknown) {
      throw new Error(getErrorMessage(error));
    }
  },

  exportHostsCsv: async () => {
    try {
      return await invoke<string>('export_hosts_csv');
    } catch (error: unknown) {
      throw new Error(getErrorMessage(error));
    }
  },

  setSelectedHostIds: (ids) => set({ selectedHostIds: ids }),
  toggleHostSelection: (id) =>
    set((s) => ({
      selectedHostIds: s.selectedHostIds.includes(id)
        ? s.selectedHostIds.filter((h) => h !== id)
        : [...s.selectedHostIds, id],
    })),
  selectAllHosts: () => set((s) => ({ selectedHostIds: s.hosts.map((h) => h.id) })),
  clearSelection: () => set({ selectedHostIds: [] }),

  loadGroups: async () => {
    try {
      const raw = await invoke<BackendAssetGroup[]>('list_groups');
      const groups = raw.map(mapBackendGroup);
      set({ groups });
    } catch {
      set({ groups: [] });
    }
  },

  addGroup: async (name, parentId) => {
    try {
      const group = await invoke<BackendAssetGroup>('add_group', { name, parentId: parentId ?? null });
      set((s) => ({ groups: upsertGroup(s.groups, mapBackendGroup(group)) }));
    } catch (error: unknown) {
      throw new Error(getErrorMessage(error));
    }
  },

  updateGroup: async (group) => {
    try {
      const updated = await invoke<BackendAssetGroup>('update_group', {
        group: {
          id: group.id,
          name: group.name,
          parent_id: group.parentId ?? null,
          sort_order: group.sortOrder,
        },
      });
      set((s) => ({ groups: upsertGroup(s.groups, mapBackendGroup(updated)) }));
    } catch (error: unknown) {
      throw new Error(getErrorMessage(error));
    }
  },

  deleteGroup: async (id) => {
    try {
      await invoke('delete_group', { id });
      set((s) => ({
        groups: s.groups.filter((group) => group.id !== id),
        hosts: s.hosts.map((host) => (host.groupId === id ? { ...host, groupId: undefined } : host)),
      }));
    } catch (error: unknown) {
      throw new Error(getErrorMessage(error));
    }
  },

  setDefaultGroupName: (name: string) => {
    localStorage.setItem('defaultGroupName', name);
    set({ defaultGroupName: name });
  },

  loadTags: async () => {
    try {
      const tags = await invoke<BackendTag[]>('list_tags');
      set({ tags: tags.map(mapBackendTag) });
    } catch {
      set({ tags: [] });
    }
  },

  addTag: async (name, color, icon) => {
    try {
      const tag = await invoke<BackendTag>('add_tag', { name, color, icon });
      set((s) => ({ tags: upsertTag(s.tags, mapBackendTag({ ...tag, icon })) }));
    } catch (error: unknown) {
      throw new Error(getErrorMessage(error));
    }
  },

  updateTag: async (tag) => {
    try {
      const updated = await invoke<BackendTag>('update_tag', { tag });
      set((s) => ({ tags: upsertTag(s.tags, mapBackendTag({ ...updated, icon: tag.icon })) }));
    } catch (error: unknown) {
      throw new Error(getErrorMessage(error));
    }
  },

  deleteTag: async (id) => {
    try {
      await invoke('delete_tag', { id });
      set((s) => ({ tags: s.tags.filter((tag) => tag.id !== id) }));
    } catch (error: unknown) {
      throw new Error(getErrorMessage(error));
    }
  },

  getHostMonitorSnapshot: async (id) => {
    try {
      const raw = await invoke<BackendHostMonitorSnapshot>('get_host_monitor_snapshot', { id });
      const networks = raw.networks?.map(mapHostMonitorNetwork) ?? [];
      const network = raw.network ? mapHostMonitorNetwork(raw.network) : networks[0];

      return {
        timestamp: raw.timestamp,
        uptime: raw.uptime ?? undefined,
        loadAverage: raw.load_average ?? undefined,
        cpuPercent: raw.cpu_percent ?? undefined,
        cpuTimeUsed: raw.cpu_time_used ?? undefined,
        cpuTimeTotal: raw.cpu_time_total ?? undefined,
        memoryUsedMb: raw.memory_used_mb ?? undefined,
        memoryTotalMb: raw.memory_total_mb ?? undefined,
        swapUsedMb: raw.swap_used_mb ?? undefined,
        swapTotalMb: raw.swap_total_mb ?? undefined,
        os: raw.os ?? undefined,
        kernel: raw.kernel ?? undefined,
        processes: raw.processes,
        network,
        networks: networks.length > 0 ? networks : (network ? [network] : undefined),
        pingMs: raw.ping_ms ?? undefined,
        filesystems: raw.filesystems,
      };
    } catch (error: unknown) {
      throw new Error(getErrorMessage(error));
    }
  },
}));
