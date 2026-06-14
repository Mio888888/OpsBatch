import assert from 'node:assert/strict';
import test from 'node:test';
import { parseRdpActions } from '../src/utils/aiActionParser.ts';

const maxCoords = { width: 1280, height: 720 };

test('parses an RDP plan whose operations field is a single operation object', () => {
  const result = parseRdpActions(
    `<RDP_PLAN>
{
  "version": 1,
  "summary": "打开开始菜单",
  "steps": [
    {
      "description": "按下 Windows 键",
      "intent": "key",
      "operations": { "type": "key", "keys": ["Meta"] },
      "expected_outcome": "开始菜单打开"
    }
  ]
}
</RDP_PLAN>`,
    maxCoords,
  );

  assert.equal(result?.parseError, undefined);
  assert.equal(result?.actions.length, 1);
  assert.deepEqual(result?.actions[0].rdpOperations, [
    { type: 'key', keys: ['Meta'] },
  ]);
});

test('parses wrapped and stringified RDP operations from goal-mode output', () => {
  const result = parseRdpActions(
    `<RDP_PLAN>
{
  "version": 1,
  "summary": "搜索记事本",
  "steps": [
    {
      "description": "输入搜索内容",
      "intent": "type",
      "operations": {
        "operations": "[{\\"type\\":\\"type\\",\\"text\\":\\"notepad\\"},{\\"type\\":\\"key\\",\\"keys\\":[\\"Enter\\"]}]"
      },
      "expected_outcome": "记事本启动"
    }
  ]
}
</RDP_PLAN>`,
    maxCoords,
  );

  assert.equal(result?.parseError, undefined);
  assert.equal(result?.actions.length, 1);
  assert.deepEqual(result?.actions[0].rdpOperations, [
    { type: 'type', text: 'notepad' },
    { type: 'key', keys: ['Enter'] },
  ]);
});

test('normalizes AI wait key output into an RDP wait operation', () => {
  const result = parseRdpActions(
    `<RDP_PLAN>
{
  "version": 1,
  "summary": "等待下载完成",
  "steps": [
    {
      "description": "等待浏览器完成下载",
      "intent": "wait",
      "operations": [{ "type": "key", "keys": ["Wait"] }],
      "expected_outcome": "下载文件出现"
    }
  ]
}
</RDP_PLAN>`,
    maxCoords,
  );

  assert.equal(result?.parseError, undefined);
  assert.deepEqual(result?.actions[0].rdpOperations, [
    { type: 'wait', ms: 1500 },
  ]);
});

test('parses numbered operation objects from terse RDP plans', () => {
  const result = parseRdpActions(
    `<RDP_PLAN>
{
  "version": 1,
  "summary": "打开浏览器",
  "steps": [
    {
      "description": "打开 Edge",
      "intent": "launch",
      "operations": {
        "1": { "type": "key", "keys": ["Meta"] },
        "2": { "type": "type", "text": "edge" },
        "3": { "type": "key", "keys": ["Enter"] }
      },
      "expected_outcome": "Edge 浏览器打开"
    }
  ]
}
</RDP_PLAN>`,
    maxCoords,
  );

  assert.equal(result?.parseError, undefined);
  assert.deepEqual(result?.actions[0].rdpOperations, [
    { type: 'key', keys: ['Meta'] },
    { type: 'type', text: 'edge' },
    { type: 'key', keys: ['Enter'] },
  ]);
});

test('extracts fenced JSON inside an RDP plan tag', () => {
  const result = parseRdpActions(
    `<RDP_PLAN>
下面是下一步操作：
\`\`\`json
{
  "version": 1,
  "summary": "打开浏览器",
  "steps": [
    {
      "description": "启动 Edge 浏览器",
      "intent": "launch",
      "operations": [
        { "type": "key", "keys": ["Meta"] },
        { "type": "type", "text": "edge" },
        { "type": "key", "keys": ["Enter"] }
      ],
      "expected_outcome": "浏览器打开"
    }
  ]
}
\`\`\`
</RDP_PLAN>`,
    maxCoords,
  );

  assert.equal(result?.parseError, undefined);
  assert.deepEqual(result?.actions[0].rdpOperations, [
    { type: 'key', keys: ['Meta'] },
    { type: 'type', text: 'edge' },
    { type: 'key', keys: ['Enter'] },
  ]);
});

test('returns detailed diagnostics when RDP plan JSON cannot be parsed', () => {
  const result = parseRdpActions(
    `<RDP_PLAN>
{
  "version": 1,
  "summary": "坏 JSON",
  "steps": [
    {
      "description": "缺少逗号"
      "intent": "launch",
      "operations": [{ "type": "key", "keys": ["Meta"] }]
    }
  ]
}
</RDP_PLAN>`,
    maxCoords,
  );

  assert.equal(result?.parseError, 'RdpPlan JSON 解析失败');
  assert.match(result?.parseDiagnostic ?? '', /contentLength=/);
  assert.match(result?.parseDiagnostic ?? '', /candidateSource=/);
  assert.match(result?.parseDiagnostic ?? '', /candidateRange=/);
  assert.match(result?.parseDiagnostic ?? '', /maxCoords=1280x720/);
  assert.match(result?.parseDiagnostic ?? '', /rawLength=/);
  assert.match(result?.parseDiagnostic ?? '', /candidate\[0\]/);
  assert.match(result?.parseDiagnostic ?? '', /JSON\.parse/);
  assert.match(result?.parseDiagnostic ?? '', /缺少逗号/);
});

test('parses JavaScript-like RDP plan output from goal mode', () => {
  const result = parseRdpActions(
    `<RDP_PLAN>
{
  // 模型有时会输出 JS 对象而不是严格 JSON
  version: 1,
  summary: '打开 Edge 搜索 Telegram',
  steps: [
    {
      description: '打开 Edge 浏览器',
      intent: 'launch',
      operations: [
        { type: 'key', keys: ['Meta'] },
        { type: 'type', text: 'edge' },
        { type: 'key', keys: ['Enter'] },
      ],
      expected_outcome: 'Edge 浏览器打开',
    },
  ],
}
</RDP_PLAN>`,
    maxCoords,
  );

  assert.equal(result?.parseError, undefined);
  assert.deepEqual(result?.actions[0].rdpOperations, [
    { type: 'key', keys: ['Meta'] },
    { type: 'type', text: 'edge' },
    { type: 'key', keys: ['Enter'] },
  ]);
});

test('ignores malformed closing RDP tag and streaming tail after a complete JSON plan', () => {
  const result = parseRdpActions(
    `<RDP_PLAN>
{
  "version": 1,
  "summary": "启动 Microsoft Edge 浏览器，准备搜索并下载 Telegram 安装程序。",
  "steps": [
    {
      "description": "在桌面双击 Microsoft Edge 图标以启动浏览器。",
      "intent": "click",
      "operations": [
        {
          "type": "click",
          "x": 26,
          "y": 218,
          "doubleClick": true
        },
        {
          "type": "wait",
          "ms": 3000
        }
      ],
      "expected_outcome": "Microsoft Edge 浏览器窗口打开并显示主页或标签页。"
    }
  ]
}
</RDP_PLAN"}]}]}`,
    maxCoords,
  );

  assert.equal(result?.parseError, undefined);
  assert.deepEqual(result?.actions[0].rdpOperations, [
    { type: 'click', x: 26, y: 218, button: 0, doubleClick: true },
    { type: 'wait', ms: 3000 },
  ]);
});

test('normalizes top-level operations into a single RDP plan step', () => {
  const result = parseRdpActions(
    `<RDP_PLAN>
{
  "version": 1,
  "summary": "打开浏览器",
  "description": "通过开始菜单启动 Edge",
  "intent": "launch",
  "operations": [
    { "type": "key", "keys": ["Meta"] },
    { "type": "type", "text": "edge" },
    { "type": "key", "keys": ["Enter"] }
  ],
  "expected_outcome": "Edge 浏览器打开"
}
</RDP_PLAN>`,
    maxCoords,
  );

  assert.equal(result?.parseError, undefined);
  assert.equal(result?.actions.length, 1);
  assert.equal(result?.actions[0].description, '通过开始菜单启动 Edge');
  assert.deepEqual(result?.actions[0].rdpOperations, [
    { type: 'key', keys: ['Meta'] },
    { type: 'type', text: 'edge' },
    { type: 'key', keys: ['Enter'] },
  ]);
});

test('repairs trailing commas in RDP plan JSON', () => {
  const result = parseRdpActions(
    `<RDP_PLAN>
{
  "version": 1,
  "summary": "打开 Edge 搜索 Telegram",
  "steps": [
    {
      "description": "打开 Edge",
      "intent": "launch",
      "operations": [
        { "type": "key", "keys": ["Meta"] },
        { "type": "type", "text": "edge" },
        { "type": "key", "keys": ["Enter"] },
      ],
      "expected_outcome": "Edge 打开",
    },
  ],
}
</RDP_PLAN>`,
    maxCoords,
  );

  assert.equal(result?.parseError, undefined);
  assert.deepEqual(result?.actions[0].rdpOperations, [
    { type: 'key', keys: ['Meta'] },
    { type: 'type', text: 'edge' },
    { type: 'key', keys: ['Enter'] },
  ]);
});

test('repairs smart quotes in RDP plan JSON', () => {
  const result = parseRdpActions(
    `<RDP_PLAN>
{
  “version”: 1,
  “summary”: “搜索 Telegram 下载页”,
  “steps”: [
    {
      “description”: “输入 Telegram 下载地址”,
      “intent”: “navigate”,
      “operations”: [
        { “type”: “type”, “text”: “https://desktop.telegram.org/” },
        { “type”: “key”, “keys”: [“Enter”] }
      ],
      “expected_outcome”: “打开 Telegram 下载页”
    }
  ]
}
</RDP_PLAN>`,
    maxCoords,
  );

  assert.equal(result?.parseError, undefined);
  assert.deepEqual(result?.actions[0].rdpOperations, [
    { type: 'type', text: 'https://desktop.telegram.org/' },
    { type: 'key', keys: ['Enter'] },
  ]);
});

test('clamps out-of-range RDP coordinates to desktop bounds', () => {
  const result = parseRdpActions(
    `<RDP_PLAN>
{
  "version": 1,
  "summary": "点击下载按钮",
  "steps": [
    {
      "description": "点击估算出的下载按钮位置",
      "intent": "click",
      "operations": [
        { "type": "click", "x": 1320, "y": 960 },
        { "type": "drag", "fromX": -12, "fromY": 900, "toX": 1400, "toY": -30 }
      ],
      "expected_outcome": "触发下载"
    }
  ]
}
</RDP_PLAN>`,
    maxCoords,
  );

  assert.equal(result?.parseError, undefined);
  assert.deepEqual(result?.actions[0].rdpOperations, [
    { type: 'click', x: 1279, y: 719, button: 0, doubleClick: false },
    { type: 'drag', fromX: 0, fromY: 719, toX: 1279, toY: 0, button: 0 },
  ]);
});
