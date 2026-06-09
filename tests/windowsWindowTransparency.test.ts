import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

type TauriWindowConfig = {
  transparent?: boolean;
  backgroundColor?: string;
};

type TauriConfig = {
  app?: {
    windows?: TauriWindowConfig[];
  };
};

test('main window keeps transparent background for rounded window corners', () => {
  const config = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8')) as TauriConfig;
  const mainWindow = config.app?.windows?.[0];

  assert.ok(mainWindow, 'main window should be configured');
  assert.equal(mainWindow.transparent, true);
  assert.equal(mainWindow.backgroundColor?.toLowerCase(), '#00000000');
});

test('managed child windows keep transparent WebViews for rounded window corners', () => {
  const source = readFileSync('src-tauri/src/commands/window.rs', 'utf8');

  assert.match(source, /\.transparent\(true\)/);
});
