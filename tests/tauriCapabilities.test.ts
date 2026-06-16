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

test('backend does not register direct keychain read commands for AI secrets', () => {
  const libSource = readFileSync('src-tauri/src/lib.rs', 'utf8');
  const aiSource = readFileSync('src-tauri/src/commands/ai.rs', 'utf8');

  assert.doesNotMatch(libSource, /ai_keychain_get/);
  assert.doesNotMatch(aiSource, /pub fn ai_keychain_get/);
});
