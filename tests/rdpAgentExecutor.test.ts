import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { rdpOperationsToInputEvents } from '../src/utils/rdpAgentExecutor.ts';

test('splits an AI double click into held down/up clicks for desktop icon activation', () => {
  const batches = rdpOperationsToInputEvents([
    { type: 'click', x: 25, y: 225, button: 0, doubleClick: true },
  ]);

  assert.deepEqual(batches, [
    [{ type: 'mouse_move', x: 25, y: 225 }],
    [
      { type: 'mouse_button', x: 25, y: 225, button: 0, down: true },
    ],
    [
      { type: 'mouse_button', x: 25, y: 225, button: 0, down: false },
    ],
    [
      { type: 'mouse_button', x: 25, y: 225, button: 0, down: true },
    ],
    [
      { type: 'mouse_button', x: 25, y: 225, button: 0, down: false },
    ],
  ]);
});

test('splits an AI single click into mouse down and mouse up batches', () => {
  const batches = rdpOperationsToInputEvents([
    { type: 'click', x: 640, y: 400, button: 0, doubleClick: false },
  ]);

  assert.deepEqual(batches, [
    [{ type: 'mouse_move', x: 640, y: 400 }],
    [
      { type: 'mouse_button', x: 640, y: 400, button: 0, down: true },
    ],
    [
      { type: 'mouse_button', x: 640, y: 400, button: 0, down: false },
    ],
  ]);
});

test('guides RDP app launch plans away from brittle desktop icon double clicks', () => {
  const source = readFileSync('src/components/RdpAiPanel.tsx', 'utf8');

  assert.match(source, /启动或打开应用.*优先使用键盘路径/);
  assert.match(source, /Win.*搜索.*运行框/);
  assert.match(source, /不要优先依赖桌面图标坐标双击/);
});
