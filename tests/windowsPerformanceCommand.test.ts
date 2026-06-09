import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

test('Windows local performance sampling hides PowerShell console windows', () => {
  const source = readFileSync('src-tauri/src/commands/system.rs', 'utf8');

  assert.match(source, /const CREATE_NO_WINDOW: u32 = 0x08000000;/);
  assert.match(source, /use std::os::windows::process::CommandExt;/);

  const powershellCommands = source.match(/Command::new\("powershell"\)/g) ?? [];
  const hiddenPowershellCommands = source.match(/Command::new\("powershell"\)[\s\S]*?\.creation_flags\(CREATE_NO_WINDOW\)[\s\S]*?\.output\(\)/g) ?? [];

  assert.equal(powershellCommands.length, 2);
  assert.equal(hiddenPowershellCommands.length, powershellCommands.length);
});
