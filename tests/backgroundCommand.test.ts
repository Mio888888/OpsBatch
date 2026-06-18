import assert from 'node:assert/strict';
import test from 'node:test';
import { parseOpsBatchBackgroundCommand } from '../src/utils/backgroundCommand.ts';

test('parses opsbatch background terminal commands', () => {
  assert.equal(
    parseOpsBatchBackgroundCommand('opsbatch-bg ./scripts/worker.sh --port 8080'),
    './scripts/worker.sh --port 8080',
  );
  assert.equal(
    parseOpsBatchBackgroundCommand('opsbatch bg npm run worker'),
    'npm run worker',
  );
});

test('ignores non-background terminal commands', () => {
  assert.equal(parseOpsBatchBackgroundCommand('npm run dev &'), null);
  assert.equal(parseOpsBatchBackgroundCommand('opsbatch-bg   '), null);
});
