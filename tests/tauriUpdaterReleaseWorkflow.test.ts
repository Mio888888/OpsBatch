import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const workflow = readFileSync('.github/workflows/build.yml', 'utf8');

test('release builds upload the Tauri updater JSON for each desktop platform', () => {
  const tauriActionBlocks = [
    ...workflow.matchAll(/uses:\s*tauri-apps\/tauri-action@v0[\s\S]*?(?=\n\s+- name:|\n\s{2}[a-zA-Z0-9_-]+:|$)/g),
  ];

  assert.equal(tauriActionBlocks.length, 3);
  for (const block of tauriActionBlocks) {
    assert.match(block[0], /^\s+uploadUpdaterJson:\s+true$/m);
  }
});

test('release builds fail before publishing if updater signing is not configured', () => {
  const prepareConfigBlocks = [
    ...workflow.matchAll(/- name: Prepare updater config[\s\S]*?(?=\n\s+- name:|\n\s{2}[a-zA-Z0-9_-]+:|$)/g),
  ];

  assert.equal(prepareConfigBlocks.length, 3);
  for (const block of prepareConfigBlocks) {
    assert.match(block[0], /test -n "\$\{TAURI_UPDATER_PUBKEY\}"/);
    assert.match(block[0], /test -n "\$\{TAURI_SIGNING_PRIVATE_KEY\}"/);
    assert.match(block[0], /TAURI_SIGNING_PRIVATE_KEY:\s+\$\{\{\s*secrets\.TAURI_SIGNING_PRIVATE_KEY\s*\}\}/);
  }
});
