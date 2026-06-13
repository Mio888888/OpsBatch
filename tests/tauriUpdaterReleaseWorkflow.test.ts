import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const workflow = readFileSync('.github/workflows/build.yml', 'utf8');
const updaterConfig = JSON.parse(readFileSync('src-tauri/tauri.updater.conf.json', 'utf8'));

test('release builds upload the Tauri updater JSON for each desktop platform', () => {
  const tauriActionBlocks = [
    ...workflow.matchAll(/uses:\s*tauri-apps\/tauri-action@v0[\s\S]*?(?=\n\s+- name:|\n\s{2}[a-zA-Z0-9_-]+:|$)/g),
  ];

  assert.equal(tauriActionBlocks.length, 3);
  for (const block of tauriActionBlocks) {
    assert.match(block[0], /^\s+includeUpdaterJson:\s+true$/m);
    assert.doesNotMatch(block[0], /uploadUpdaterJson/);
  }
});

test('release updater config generates signed updater artifacts', () => {
  assert.equal(updaterConfig.bundle.createUpdaterArtifacts, true);
});

test('release builds are skipped before publishing if updater signing secrets are missing', () => {
  const checkVersionBlock = workflow.match(
    /- name: Check version change[\s\S]*?(?=\n\s+- name:|\n\s{2}[a-zA-Z0-9_-]+:|$)/,
  );

  assert.ok(checkVersionBlock);
  assert.match(checkVersionBlock[0], /\[ -z "\$\{TAURI_UPDATER_PUBKEY\}" \]/);
  assert.match(checkVersionBlock[0], /\[ -z "\$\{TAURI_SIGNING_PRIVATE_KEY\}" \]/);
  assert.match(checkVersionBlock[0], /changed=false/);
  assert.match(checkVersionBlock[0], /TAURI_UPDATER_PUBKEY:\s+\$\{\{\s*secrets\.TAURI_UPDATER_PUBKEY\s*\}\}/);
  assert.match(
    checkVersionBlock[0],
    /TAURI_SIGNING_PRIVATE_KEY:\s+\$\{\{\s*secrets\.TAURI_UPDATER_PRIVATE_KEY\s*\}\}/,
  );
  assert.match(checkVersionBlock[0], /GITHUB_TOKEN:\s+\$\{\{\s*secrets\.GITHUB_TOKEN\s*\}\}/);
  assert.match(checkVersionBlock[0], /releases\/tags\/\$\{TAG\}/);
  assert.match(checkVersionBlock[0], /select\(\.name == "latest\.json"\)/);
  assert.match(checkVersionBlock[0], /Tag \$\{TAG\} exists but latest\.json is missing/);
  assert.match(checkVersionBlock[0], /echo "changed=true" >> "\$GITHUB_OUTPUT"/);
});

test('release build steps expose official Tauri signing env from existing updater secrets', () => {
  const prepareConfigBlocks = [
    ...workflow.matchAll(/- name: Prepare updater config[\s\S]*?(?=\n\s+- name:|\n\s{2}[a-zA-Z0-9_-]+:|$)/g),
  ];

  assert.equal(prepareConfigBlocks.length, 3);
  for (const block of prepareConfigBlocks) {
    assert.match(block[0], /test -n "\$\{TAURI_UPDATER_PUBKEY\}"/);
    assert.match(block[0], /test -n "\$\{TAURI_SIGNING_PRIVATE_KEY\}"/);
    assert.match(block[0], /TAURI_SIGNING_PRIVATE_KEY:\s+\$\{\{\s*secrets\.TAURI_UPDATER_PRIVATE_KEY\s*\}\}/);
  }

  const tauriActionBlocks = [
    ...workflow.matchAll(/uses:\s*tauri-apps\/tauri-action@v0[\s\S]*?(?=\n\s+- name:|\n\s{2}[a-zA-Z0-9_-]+:|$)/g),
  ];

  assert.equal(tauriActionBlocks.length, 3);
  for (const block of tauriActionBlocks) {
    assert.match(block[0], /TAURI_SIGNING_PRIVATE_KEY:\s+\$\{\{\s*secrets\.TAURI_UPDATER_PRIVATE_KEY\s*\}\}/);
    assert.match(
      block[0],
      /TAURI_SIGNING_PRIVATE_KEY_PASSWORD:\s+\$\{\{\s*secrets\.TAURI_UPDATER_PRIVATE_KEY_PASSWORD\s*\}\}/,
    );
  }
});
