import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

test('update modal exposes a manual check action from the update hook', () => {
  const hookSource = readFileSync('src/hooks/useUpdate.ts', 'utf8');
  const layoutSource = readFileSync('src/components/MainLayout.tsx', 'utf8');
  const modalSource = readFileSync('src/components/asset/UpdateModal.tsx', 'utf8');

  assert.match(hookSource, /const \[updateCheckBusy, setUpdateCheckBusy\]/);
  assert.match(hookSource, /checkForUpdates,\s*\n\s*updateCheckBusy,/);
  assert.match(layoutSource, /checkForUpdates=\{updateState\.checkForUpdates\}/);
  assert.match(layoutSource, /updateCheckBusy=\{updateState\.updateCheckBusy\}/);
  assert.match(modalSource, /checkForUpdates: \(silent\?: boolean\) => Promise<AppUpdateInfo \| null>/);
  assert.match(modalSource, /loading=\{updateCheckBusy\}/);
  assert.match(modalSource, /disabled=\{updateBusy \|\| updateCheckBusy\}/);
  assert.match(modalSource, /t\('appUpdate\.check'\)/);
});
