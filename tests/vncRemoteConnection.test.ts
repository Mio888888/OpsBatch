import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import {
  buildRdpSettings,
  isVncRemoteDesktopHost,
} from '../src/utils/rdpSettings.ts';
import type { Host } from '../src/types/index.ts';

test('builds VNC settings when VNC is selected as the host system', () => {
  const settings = buildRdpSettings({
    vncPort: 5901,
    vncPassword: '  secret  ',
    vncViewOnly: true,
    vncShared: false,
  }, 'vnc');

  assert.deepEqual(settings, {
    protocol: 'vnc',
    vncPort: 5901,
    vncPassword: 'secret',
    vncViewOnly: true,
    vncShared: false,
  });
});

test('maps Linux and Windows systems to their default remote access behavior', () => {
  assert.equal(buildRdpSettings({}, 'linux'), undefined);
  assert.deepEqual(buildRdpSettings({}, 'windows'), {
    protocol: 'rdp',
    enableClipboard: true,
    enableAudio: true,
    mapDisk: false,
  });
});

test('identifies hosts that should open through VNC instead of terminal or RDP', () => {
  const host = {
    os: 'vnc',
    rdpSettings: { protocol: 'vnc' },
  } as Pick<Host, 'os' | 'rdpSettings'>;

  assert.equal(isVncRemoteDesktopHost(host), true);
});

test('selects VNC from the system field instead of a remote access field', () => {
  const source = readFileSync('src/components/MainLayout.tsx', 'utf8');

  assert.match(source, /assets\.openVnc/);
  assert.match(source, /\{\s*value:\s*'vnc',\s*label:\s*t\('assets\.vncHost'\)\s*\}/);
  assert.doesNotMatch(source, /remoteDesktopProtocol/);
  assert.match(source, /vncPort/);
  assert.match(source, /kind:\s*'vnc'/);
});

test('opens VNC before prompting for SSH keychain secrets', () => {
  const source = readFileSync('src/components/MainLayout.tsx', 'utf8');
  const vncBranchIndex = source.indexOf('if (isVncHost(host))');
  const keychainNoticeIndex = source.indexOf('if (hostUsesStoredSecret(host)');

  assert.notEqual(vncBranchIndex, -1);
  assert.notEqual(keychainNoticeIndex, -1);
  assert.ok(vncBranchIndex < keychainNoticeIndex);
});

test('registers a managed VNC window and backend VNC commands', () => {
  const windowSource = readFileSync('src-tauri/src/commands/window.rs', 'utf8');
  const libSource = readFileSync('src-tauri/src/lib.rs', 'utf8');
  const appSource = readFileSync('src/App.tsx', 'utf8');
  const capabilitySource = readFileSync('src-tauri/capabilities/default.json', 'utf8');

  assert.match(windowSource, /Vnc/);
  assert.match(windowSource, /"vnc"/);
  assert.match(windowSource, /\/vnc\?hostId=/);
  assert.match(libSource, /commands::vnc::vnc_connect/);
  assert.match(libSource, /commands::vnc::vnc_send_input/);
  assert.match(libSource, /commands::vnc::vnc_disconnect/);
  assert.match(appSource, /VncPage/);
  assert.match(capabilitySource, /"vnc-\*"/);
});

test('backend defaults missing VNC ports to 5900 instead of SSH or RDP ports', () => {
  const source = readFileSync('src-tauri/src/commands/vnc.rs', 'utf8');

  assert.match(source, /const DEFAULT_VNC_PORT:\s*u16\s*=\s*5900/);
  assert.doesNotMatch(source, /\.or_else\(\|\| u16::try_from\(_?fallback\)\.ok\(\)\)/);
});
