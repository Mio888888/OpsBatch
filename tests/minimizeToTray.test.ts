import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

test('main window honors minimizeToTray setting through a system tray', () => {
  const manifest = readFileSync('src-tauri/Cargo.toml', 'utf8');
  const source = readFileSync('src-tauri/src/lib.rs', 'utf8');

  assert.match(manifest, /tauri\s*=\s*\{[^}]*features\s*=\s*\[[^\]]*"tray-icon"/s);
  assert.match(source, /TrayIconBuilder::with_id\("main"\)/);
  assert.match(source, /WindowEvent::CloseRequested\s*\{\s*api[\s\S]*\}/);
  assert.match(source, /SELECT value FROM general_settings WHERE key='minimizeToTray'/);
  assert.match(source, /settings_minimize_to_tray\(app_handle\)/);
  assert.match(source, /api\.prevent_close\(\)/);
  assert.match(source, /window\.hide\(\)/);
  assert.match(source, /on_tray_icon_event/);
  assert.match(source, /window\.show\(\)/);
  assert.match(source, /window\.set_focus\(\)/);
});
