import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import {
  buildProxySettings,
  serializeProxySettings,
  parseProxySettings,
} from '../src/utils/proxySettings.ts';

test('normalizes enabled proxy settings from host form values', () => {
  const settings = buildProxySettings({
    proxyEnabled: true,
    proxyType: 'socks5',
    proxyHost: ' proxy.internal ',
    proxyPort: 1080,
    proxyUsername: ' deploy ',
    proxyPassword: ' secret ',
  });

  assert.deepEqual(settings, {
    enabled: true,
    type: 'socks5',
    host: 'proxy.internal',
    port: 1080,
    username: 'deploy',
    password: 'secret',
  });
});

test('drops incomplete proxy settings', () => {
  assert.equal(buildProxySettings({ proxyEnabled: true, proxyType: 'http', proxyHost: '', proxyPort: 8080 }), undefined);
  assert.equal(buildProxySettings({ proxyEnabled: false, proxyType: 'socks5', proxyHost: 'proxy', proxyPort: 1080 }), undefined);
});

test('round trips proxy settings json safely', () => {
  const json = serializeProxySettings({
    enabled: true,
    type: 'http',
    host: 'proxy.internal',
    port: 8080,
  });

  assert.deepEqual(parseProxySettings(json), {
    enabled: true,
    type: 'http',
    host: 'proxy.internal',
    port: 8080,
  });
  assert.equal(parseProxySettings('{bad json'), undefined);
});

test('host form includes proxy settings in advanced tabs', () => {
  const source = readFileSync('src/components/MainLayout.tsx', 'utf8');

  assert.match(source, /proxyEnabled/);
  assert.match(source, /assets\.proxySettings/);
  assert.match(source, /key:\s*'linux-advanced'/);
});
