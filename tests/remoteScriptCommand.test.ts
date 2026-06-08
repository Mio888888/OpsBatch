import assert from 'node:assert/strict';
import test from 'node:test';
import { buildRemoteScriptCommand } from '../src/utils/remoteScriptCommand.ts';

test('omits empty remote script parameters', () => {
  const command = buildRemoteScriptCommand(
    'https://raw.githubusercontent.com/spiritLHLS/ecs/main/ecs.sh',
    [
      { name: '-m', defaultValue: '' },
      { name: '-en', defaultValue: '' },
      { name: 'args', defaultValue: '' },
    ],
    {},
  );

  assert.equal(
    command,
    "curl -sSL 'https://raw.githubusercontent.com/spiritLHLS/ecs/main/ecs.sh' | bash",
  );
});

test('passes non-empty option parameters to bash argv', () => {
  const command = buildRemoteScriptCommand(
    'https://raw.githubusercontent.com/spiritLHLS/ecs/main/ecs.sh',
    [
      { name: '-m', defaultValue: '' },
      { name: '-banup', defaultValue: '' },
      { name: 'args', defaultValue: '' },
    ],
    {
      '-m': '1',
      '-banup': 'true',
      args: '-base -ctype gb5',
    },
  );

  assert.equal(
    command,
    "curl -sSL 'https://raw.githubusercontent.com/spiritLHLS/ecs/main/ecs.sh' | bash -s -- -m '1' -banup -base -ctype gb5",
  );
});

test('quotes parameter values passed as argv', () => {
  const command = buildRemoteScriptCommand(
    'https://example.com/install.sh',
    [
      { name: '--name', defaultValue: '' },
      { name: '--path', defaultValue: '' },
    ],
    {
      '--name': "mio's app",
      '--path': '/opt/Ops Batch',
    },
  );

  assert.equal(
    command,
    "curl -sSL 'https://example.com/install.sh' | bash -s -- --name 'mio'\\''s app' --path '/opt/Ops Batch'",
  );
});
