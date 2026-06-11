import type { ProxySettings } from '../types';

export interface HostProxySettingsFormValues {
  proxyEnabled?: boolean;
  proxyType?: ProxySettings['type'];
  proxyHost?: string;
  proxyPort?: number;
  proxyUsername?: string;
  proxyPassword?: string;
}

export const DEFAULT_PROXY_PORTS: Record<ProxySettings['type'], number> = {
  http: 8080,
  socks5: 1080,
};

function normalizeProxyPort(value: unknown, type: ProxySettings['type']) {
  if (typeof value !== 'number' || !Number.isFinite(value)) return DEFAULT_PROXY_PORTS[type];
  return Math.min(65535, Math.max(1, Math.round(value)));
}

export function buildProxySettings(values: HostProxySettingsFormValues): ProxySettings | undefined {
  if (!values.proxyEnabled) return undefined;

  const type = values.proxyType === 'http' ? 'http' : 'socks5';
  const host = values.proxyHost?.trim();
  if (!host) return undefined;

  const settings: ProxySettings = {
    enabled: true,
    type,
    host,
    port: normalizeProxyPort(values.proxyPort, type),
  };

  if (values.proxyUsername?.trim()) {
    settings.username = values.proxyUsername.trim();
  }
  if (values.proxyPassword?.trim()) {
    settings.password = values.proxyPassword.trim();
  }

  return settings;
}

export function parseProxySettings(value?: string | null): ProxySettings | undefined {
  if (!value) return undefined;
  try {
    const parsed = JSON.parse(value) as Partial<ProxySettings>;
    return normalizeProxySettings(parsed);
  } catch {
    return undefined;
  }
}

export function normalizeProxySettings(value?: Partial<ProxySettings>): ProxySettings | undefined {
  if (!value?.enabled) return undefined;
  if (value.type !== 'http' && value.type !== 'socks5') return undefined;
  if (typeof value.host !== 'string' || !value.host.trim()) return undefined;

  const settings: ProxySettings = {
    enabled: true,
    type: value.type,
    host: value.host.trim(),
    port: normalizeProxyPort(value.port, value.type),
  };

  if (typeof value.username === 'string' && value.username.trim()) {
    settings.username = value.username.trim();
  }
  if (typeof value.password === 'string' && value.password.trim()) {
    settings.password = value.password.trim();
  }

  return settings;
}

export function serializeProxySettings(settings?: ProxySettings): string {
  return JSON.stringify(normalizeProxySettings(settings) ?? {});
}
