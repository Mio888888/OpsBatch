import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { parseAiPendingActionsAsync, parseSshPendingActions } from '../src/utils/aiActionParser.ts';
import { formatTerminalCommandResult } from '../src/utils/terminalFollowUp.ts';

test('SSH AI keeps only the first action from multiple action blocks', () => {
  const result = parseSshPendingActions(`
[ACTION:检查服务]
systemctl status nginx
[/ACTION]
[ACTION:读取日志]
journalctl -u nginx -n 50
[/ACTION]
`);

  assert.equal(result.actions.length, 1);
  assert.equal(result.actions[0].command, 'systemctl status nginx');
});

test('SSH AI preserves one compound shell action', () => {
  const result = parseSshPendingActions(`
\`\`\`bash
systemctl restart nginx && systemctl status nginx
journalctl -u nginx -n 20 --no-pager
\`\`\`
`);

  assert.equal(result.actions.length, 1);
  assert.match(result.actions[0].command ?? '', /&&/);
  assert.match(result.actions[0].command ?? '', /\njournalctl/);
});

test('SSH AI keeps only the first validated COMMAND_PLAN step', async () => {
  Object.assign(globalThis, { window: globalThis });
  mockIPC((command) => {
    assert.equal(command, 'ai_validate_command_plan');
    return {
      version: 1,
      summary: '依次检查服务和日志',
      steps: [
        { id: 'step-1', description: '检查服务', command: 'systemctl status nginx', intent: 'observe', expected_outcome: '显示服务状态' },
        { id: 'step-2', description: '读取日志', command: 'journalctl -u nginx -n 50', intent: 'diagnose', expected_outcome: '显示最近日志' },
      ],
    };
  });

  try {
    const result = await parseAiPendingActionsAsync(`<COMMAND_PLAN>
{"version":1,"summary":"依次检查服务和日志","steps":[{"command":"first"},{"command":"second"}]}
</COMMAND_PLAN>`, undefined, true);
    assert.equal(result.actions.length, 1);
    assert.equal(result.actions[0].command, 'systemctl status nginx');
  } finally {
    clearMocks();
  }
});

test('SSH AI reports completed commands even when they have no output', () => {
  const output = formatTerminalCommandResult({
    status: 'completed',
    output: '',
    exitCode: 0,
  });

  assert.match(output, /^\[当前命令：completed，退出码 0\]/);
  assert.match(output, /没有产生输出/);
});

test('AI chat enables single-command parsing only for terminal scopes', async () => {
  const store = await readFile('src/stores/aiChat.ts', 'utf8');

  assert.match(store, /scope\.scope === 'ssh_host'/);
  assert.match(store, /scope\.scope === 'terminal_session'/);
  assert.match(store, /parseAiPendingActionsAsync\(rawContent, rdpCtx, singleCommand\)/);
  assert.doesNotMatch(store, /scope\.scope === 'rdp_host'[^;]*singleCommand/s);
});

test('SSH AI panel enforces one command round at a time', async () => {
  const panel = await readFile('src/components/AiChatPanel.tsx', 'utf8');

  assert.match(panel, /需要用户[^\n]*选择[^\n]*不输出[^\n]*命令/);
  assert.match(panel, /steps[^\n]*必须恰好包含一个步骤/);
  assert.match(panel, /收到当前命令[^\n]*执行结果[^\n]*再/);
  assert.doesNotMatch(panel, /可以提议多个命令/);
  assert.doesNotMatch(panel, /ai\.executeAll/);
  assert.doesNotMatch(panel, /以下命令已全部执行完毕/);
  assert.doesNotMatch(panel, /decision\.shouldFollowUp \|\|/);
  assert.match(panel, /void sendDirectMessage\(/);
  assert.doesNotMatch(panel, /autoFollowUpDoneRef/);
  assert.match(panel, /const aiSessionId = sessionId/);
  assert.match(panel, /approveAction\(actionId, undefined, aiSessionId\)/);
  assert.match(panel, /sendDirectMessage\([\s\S]*aiSessionId[\s\S]*\)/);
  assert.match(panel, /disabled=\{streaming \|\| hasUnfinishedAction\}/);
  assert.match(panel, /disabled=\{streaming \|\| hasUnfinishedAction \|\| !inputText\.trim\(\)\}/);
  const followUp = await readFile('src/utils/terminalFollowUp.ts', 'utf8');
  assert.match(followUp, /\[当前命令：\$\{result\.status\}/);
});

test('AI store can complete and follow up in the originating session', async () => {
  const store = await readFile('src/stores/aiChat.ts', 'utf8');

  assert.match(store, /sendDirectMessage: async \(text, context, images, targetSessionId\)/);
  assert.match(store, /approveAction: async \(actionId, sessionId, targetSessionId\)/);
  assert.match(store, /const sid = targetSessionId \?\? state\.activeSessionId/);
});
