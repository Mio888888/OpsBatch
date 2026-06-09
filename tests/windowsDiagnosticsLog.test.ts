import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

test('release diagnostics write to an app-data log outside the UI', () => {
  const modSource = readFileSync('src-tauri/src/commands/mod.rs', 'utf8');
  const diagnosticsSource = readFileSync('src-tauri/src/commands/diagnostics.rs', 'utf8');
  const libSource = readFileSync('src-tauri/src/lib.rs', 'utf8');
  const windowSource = readFileSync('src-tauri/src/commands/window.rs', 'utf8');
  const mainSource = readFileSync('src/main.tsx', 'utf8');

  assert.match(modSource, /pub mod diagnostics;/);
  assert.match(diagnosticsSource, /opsbatch-diagnostics\.log/);
  assert.match(diagnosticsSource, /OpenOptions::new\(\)[\s\S]*\.append\(true\)/);
  assert.match(diagnosticsSource, /#\[tauri::command\]\s*pub fn write_diagnostic_log/);
  assert.match(libSource, /\.on_page_load\(/);
  assert.match(libSource, /commands::diagnostics::append_diagnostic_log/);
  assert.match(windowSource, /append_diagnostic_log\(\s*app\.app_handle\(\),\s*"window",/);
  assert.match(mainSource, /installDiagnosticsBridge\(\)/);
  assert.match(mainSource, /void import\("\.\/App"\)/);
  assert.match(mainSource, /invoke\("write_diagnostic_log"/);
});
