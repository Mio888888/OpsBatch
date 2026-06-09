import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import test from 'node:test';

type SignCommand = {
  cmd: string;
  args: string[];
};

type TauriWindowsConfig = {
  bundle?: {
    windows?: {
      signCommand?: SignCommand;
    };
  };
};

const config = JSON.parse(
  readFileSync('src-tauri/tauri.windows.conf.json', 'utf8'),
) as TauriWindowsConfig;

test('passes the Windows signing target as an isolated sign command argument', () => {
  const signCommand = config.bundle?.windows?.signCommand;

  assert.ok(signCommand, 'windows signCommand should be configured');
  assert.match(signCommand.cmd, /powershell(\.exe)?$/i);
  assert.ok(signCommand.args.includes('-File'));
  assert.ok(signCommand.args.includes('scripts/windows-sign.ps1'));
  assert.equal(signCommand.args.at(-1), '%1');
  assert.equal(signCommand.args.filter((arg) => arg === '%1').length, 1);

  const scriptArg = signCommand.args[signCommand.args.indexOf('-File') + 1];
  assert.ok(existsSync(scriptArg), `${scriptArg} should exist from repo root`);
  assert.ok(
    existsSync(join('src-tauri', scriptArg)),
    `${scriptArg} should exist from src-tauri cwd`,
  );

  const commandScript = signCommand.args.join(' ');
  assert.doesNotMatch(commandScript, /\$target\s*=\s*['"]%1['"]/);
  assert.doesNotMatch(commandScript, /-Command/);
});
