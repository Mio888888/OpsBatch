import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

test('main window minimize button hides to tray when the setting is enabled', () => {
  const capability = JSON.parse(readFileSync('src-tauri/capabilities/default.json', 'utf8')) as {
    permissions?: string[];
  };
  const source = readFileSync('src/components/WindowControls.tsx', 'utf8');

  assert.ok(capability.permissions?.includes('core:window:allow-hide'));
  assert.match(source, /invoke<Record<string, string>>\('get_general_settings'\)/);
  assert.match(source, /window\.label === 'main'/);
  assert.match(source, /settings\.minimizeToTray !== 'false'/);
  assert.match(source, /window\.hide\(\)/);
  assert.match(source, /window\.minimize\(\)/);
});
