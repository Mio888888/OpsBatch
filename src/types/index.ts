export interface RdpSettings {
  protocol?: 'rdp' | 'vnc';
  domain?: string;
  desktopWidth?: number;
  desktopHeight?: number;
  enableClipboard?: boolean;
  enableAudio?: boolean;
  mapDisk?: boolean;
  diskPath?: string;
  vncPort?: number;
  vncUsername?: string;
  vncPassword?: string;
  vncAuthMethod?: 'vnc' | 'ard';
  vncViewOnly?: boolean;
  vncShared?: boolean;
}

export interface ProxySettings {
  enabled: boolean;
  type: 'http' | 'socks5';
  host: string;
  port: number;
  username?: string;
  password?: string;
}

export interface Host {
  id: string;
  name: string;
  ip: string;
  port: number;
  authType: 'password' | 'key';
  username: string;
  password?: string;
  privateKey?: string;
  os: 'linux' | 'windows' | 'vnc';
  tags: string[];
  remark: string;
  groupId?: string;
  jumpChain: string[];
  rdpSettings?: RdpSettings;
  proxySettings?: ProxySettings;
  createdAt: string;
  updatedAt: string;
}

export interface AssetGroup {
  id: string;
  name: string;
  parentId?: string;
  sortOrder: number;
}

export interface HostMonitorNetwork {
  interface: string;
  rxBytes: number;
  txBytes: number;
}

export interface HostMonitorSnapshot {
  timestamp: number;
  uptime?: string;
  loadAverage?: string;
  cpuPercent?: number;
  cpuTimeUsed?: number;
  cpuTimeTotal?: number;
  memoryUsedMb?: number;
  memoryTotalMb?: number;
  swapUsedMb?: number;
  swapTotalMb?: number;
  os?: string;
  kernel?: string;
  processes: Array<{
    memory: string;
    cpu: string;
    command: string;
  }>;
  network?: HostMonitorNetwork;
  networks?: HostMonitorNetwork[];
  pingMs?: number;
  filesystems: Array<{
    path: string;
    used: string;
    available: string;
    total: string;
    percent: string;
  }>;
}

export interface Tag {
  id: string;
  name: string;
  color: string;
  icon: string;
}

export interface CommandParameter {
  name: string;
  description: string;
  required: boolean;
  defaultValue: string;
}

export interface CommandEntry {
  id: string;
  name: string;
  command: string;
  url: string;
  kind?: 'command' | 'docker';
  category: string;
  tags: string[];
  risk: 'low' | 'medium' | 'high' | 'critical';
  description: string;
  platform: 'linux' | 'windows' | 'both';
  parameters: CommandParameter[];
  starred: boolean;
  isBuiltin: boolean;
}

export interface ScriptEntry {
  id: string;
  name: string;
  language: 'shell' | 'python' | 'powershell';
  category: string;
  tags: string[];
  risk: 'low' | 'medium' | 'high' | 'critical';
  description: string;
  content: string;
  parameters: ScriptParameter[];
  url: string;
  platform: 'linux' | 'windows' | 'both';
  starred: boolean;
  isBuiltin: boolean;
}

export interface ScriptParameter {
  name: string;
  description: string;
  required: boolean;
  defaultValue: string;
}

export interface QuickAction {
  id: string;
  name: string;
  command: string;
  category: string;
  parameters: string[];
  order: number;
  starred: boolean;
  description: string;
  tags: string[];
  language: 'shell' | 'python' | 'powershell';
  lastRunAt: string;
  lastStatus: string;
}

export interface ExecutionTask {
  id: string;
  hostIds: string[];
  command: string;
  status: 'pending' | 'running' | 'completed' | 'failed' | 'cancelled';
  results: HostExecutionResult[];
  concurrency: number;
  timeout: number;
  startedAt?: string;
  completedAt?: string;
}

export interface HostExecutionResult {
  hostId: string;
  hostName: string;
  hostIp: string;
  status: 'pending' | 'running' | 'success' | 'failed' | 'timeout';
  output: string;
  exitCode?: number;
  startedAt?: string;
  completedAt?: string;
  duration?: number;
}

export interface ExecutionHistory {
  id: string;
  command: string;
  hostIds: string[];
  hostCount: number;
  successCount: number;
  failCount: number;
  startedAt: string;
  completedAt: string;
  duration: number;
}
