import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

test('SSH idle status emitted by the backend matches the terminal listener', () => {
  const sshSource = readFileSync('src-tauri/src/ssh/mod.rs', 'utf8');
  const terminalSource = readFileSync('src/pages/Terminal/TerminalPage.tsx', 'utf8');

  assert.match(sshSource, /ConnectionState::Idle => "idle_disconnected"/);
  assert.match(terminalSource, /status === 'idle_disconnected' \|\| status === 'link_down'/);
});

test('terminal activity refreshes the shared SSH connection before idle reaping', () => {
  const sshSource = readFileSync('src-tauri/src/ssh/mod.rs', 'utf8');
  const terminalSource = readFileSync('src-tauri/src/commands/terminal.rs', 'utf8');

  assert.match(sshSource, /pub fn touch_connection\(&self, host_id: &str\) -> bool \{/);
  assert.match(terminalSource, /SessionHandle \{[\s\S]*ssh_connection_key: Option<String>,/);
  assert.match(terminalSource, /TerminalCommand::Write\(data\.into_bytes\(\)\)[\s\S]*touch_terminal_connection\(&pool, host_id\.as_deref\(\)\)/);
  assert.match(terminalSource, /TerminalCommand::Resize \{ cols, rows \}[\s\S]*touch_terminal_connection\(&pool, host_id\.as_deref\(\)\)/);
});

test('SSH reader activity keeps long-running interactive terminal sessions alive', () => {
  const terminalSource = readFileSync('src-tauri/src/commands/terminal.rs', 'utf8');

  assert.match(terminalSource, /fn spawn_ssh_reader\([\s\S]*host_id: Option<String>,/);
  assert.match(terminalSource, /Some\(Some\(ChannelMsg::Data \{ data \}\)\)[\s\S]*touch_ssh_connection_by_host\(&app_handle, host_id\.as_deref\(\)\)/);
});
