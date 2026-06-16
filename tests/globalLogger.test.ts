import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import * as logger from '../src/utils/globalLogger.ts';

test('redacts sensitive values from frontend log messages', () => {
  const sanitizeLogMessage = (logger as {
    sanitizeLogMessage?: (message: string) => string;
  }).sanitizeLogMessage;

  assert.equal(typeof sanitizeLogMessage, 'function');
  assert.equal(
    sanitizeLogMessage('Bearer abc123 token=secret password: hunter2 api_key = live-key'),
    'Bearer *** token=*** password: *** api_key = ***',
  );
  assert.equal(
    sanitizeLogMessage('{"vncPassword":"secret","proxyPassword":"proxy-secret","apiKey":"live-key"}'),
    '{"vncPassword":"***","proxyPassword":"***","apiKey":"***"}',
  );
});

test('keeps long parser diagnostics in frontend log messages', () => {
  const sanitizeLogMessage = (logger as {
    sanitizeLogMessage?: (message: string) => string;
  }).sanitizeLogMessage;

  assert.equal(typeof sanitizeLogMessage, 'function');
  const longDiagnostic = `rdp.ai.parse\n${'x'.repeat(6000)}\nend-marker`;

  assert.match(sanitizeLogMessage(longDiagnostic), /end-marker$/);
});

test('serializes unknown log arguments without dropping errors', () => {
  const formatLogArguments = (logger as {
    formatLogArguments?: (args: unknown[]) => string;
  }).formatLogArguments;

  assert.equal(typeof formatLogArguments, 'function');
  const message = formatLogArguments([
    'failed',
    new Error('boom'),
    { code: 500, nested: { ok: false } },
  ]);

  assert.match(message, /failed/);
  assert.match(message, /Error: boom/);
  assert.match(message, /"code":500/);
});

test('maps console methods to global log levels', () => {
  const getConsoleLogLevel = (logger as {
    getConsoleLogLevel?: (method: string) => string;
  }).getConsoleLogLevel;

  assert.equal(typeof getConsoleLogLevel, 'function');
  assert.equal(getConsoleLogLevel('log'), 'info');
  assert.equal(getConsoleLogLevel('info'), 'info');
  assert.equal(getConsoleLogLevel('warn'), 'warn');
  assert.equal(getConsoleLogLevel('error'), 'error');
});

test('exposes a helper for handled exceptions', () => {
  const logHandledError = (logger as {
    logHandledError?: (source: string, error: unknown, level?: string) => Promise<unknown>;
  }).logHandledError;

  assert.equal(typeof logHandledError, 'function');
});

test('registers a backend command for persisted frontend logs', () => {
  const appLogSource = readFileSync('src-tauri/src/commands/app_log.rs', 'utf8');
  const libSource = readFileSync('src-tauri/src/lib.rs', 'utf8');
  const mainSource = readFileSync('src/main.tsx', 'utf8');

  assert.match(appLogSource, /pub fn emit_frontend_log/);
  assert.match(appLogSource, /pub fn clear_log_history/);
  assert.match(libSource, /commands::app_log::emit_frontend_log/);
  assert.match(libSource, /commands::app_log::clear_log_history/);
  assert.match(mainSource, /installGlobalLogHandler/);
  assert.doesNotMatch(mainSource, /console\.log =/);
});

test('routes visible UI messages to the persisted global log stream', () => {
  const uiSource = readFileSync('src/components/ui/index.tsx', 'utf8');

  assert.match(uiSource, /emitFrontendGlobalLog/);
  assert.match(uiSource, /showMessage\('success', 'success', content\)/);
  assert.match(uiSource, /showMessage\('error', 'error', content\)/);
  assert.match(uiSource, /showMessage\('warning', 'warn', content\)/);
  assert.match(uiSource, /showMessage\('info', 'info', content\)/);
  assert.match(uiSource, /emitFrontendGlobalLog\(logLevel, 'ui',/);
});

test('routes silent handled exceptions through the global logger', () => {
  const sourceFiles = [
    'src/pages/CommandLib/CommandLibPage.tsx',
    'src/pages/Workflow/ScheduledTaskManager.tsx',
    'src/pages/Workflow/WorkflowPage.tsx',
    'src/pages/QuickActions/QuickActionsPage.tsx',
    'src/pages/GitHub/GitHubPage.tsx',
    'src/pages/Settings/SettingsPage.tsx',
    'src/pages/ScriptLib/ScriptLibPage.tsx',
    'src/pages/Commands/BatchTerminalWindow.tsx',
    'src/pages/Transfer/BatchTransferWindow.tsx',
    'src/pages/Terminal/TerminalPage.tsx',
    'src/pages/Rdp/useRdpConnection.ts',
    'src/components/CodeEditor.tsx',
    'src/components/TerminalView.tsx',
    'src/stores/language.ts',
    'src/stores/theme.ts',
    'src/stores/aiChat.ts',
  ];

  for (const path of sourceFiles) {
    const source = readFileSync(path, 'utf8');
    assert.doesNotMatch(source, /catch\s*\{\s*\}/, path);
    assert.doesNotMatch(source, /\.catch\(\s*\(\s*\)\s*=>\s*\{\s*\}\s*\)/, path);
  }
});

test('does not log expected RDP media play aborts', () => {
  const source = readFileSync('src/pages/Rdp/useRdpConnection.ts', 'utf8');

  assert.match(source, /isExpectedMediaPlayAbort/);
  assert.match(source, /if \(isExpectedMediaPlayAbort\(error\)\) return/);
  assert.match(source, /void logHandledError\('rdp\.media\.play', error, 'warn'\)/);
});
