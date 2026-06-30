import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

test('RDP disconnect button closes the managed RDP window', () => {
  const source = readFileSync('src/pages/Rdp/RdpPage.tsx', 'utf8');

  assert.match(source, /const disconnectAndCloseWindow = useCallback\(\(\) => \{\s*disconnectActive\(\);[\s\S]*getCurrentWindow\(\)\.destroy\(\)/);
  assert.match(source, /onClick=\{disconnectAndCloseWindow\}[\s\S]*\{t\('rdp\.disconnect'\)\}/);
});

test('VNC disconnect button closes the managed VNC window', () => {
  const source = readFileSync('src/pages/Vnc/VncPage.tsx', 'utf8');

  assert.match(source, /const disconnectAndCloseWindow = useCallback\(\(\) => \{\s*closeVncSession\(sessionIdRef\.current\);[\s\S]*getCurrentWindow\(\)\.destroy\(\)/);
  assert.match(source, /onClick=\{disconnectAndCloseWindow\}[\s\S]*\{t\('vnc\.disconnect'\)\}/);
});
