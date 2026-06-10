import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

test('default Tauri capability allows dynamic RDP windows to listen for events', () => {
  const capability = JSON.parse(readFileSync('src-tauri/capabilities/default.json', 'utf8')) as {
    windows?: string[];
    permissions?: string[];
  };

  assert.ok(capability.windows?.includes('rdp-*'));
  assert.ok(capability.permissions?.includes('core:event:allow-listen'));
});
