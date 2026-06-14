import assert from 'node:assert/strict';
import test from 'node:test';
import { extractEmptyCommandPlanNotice, stripCommandPlanBlock } from '../src/utils/aiActionParser.ts';

test('extracts an empty COMMAND_PLAN when the closing tag is missing its bracket', () => {
  const result = extractEmptyCommandPlanNotice(`<COMMAND_PLAN>
{
  "version": 1,
  "summary": "当前只需要观察目录，不需要后续操作",
  "steps": []
}
</COMMAND_PLAN`);

  assert.equal(result?.displayContent, '');
  assert.deepEqual(result?.commandPlanNotice, {
    version: 1,
    summary: '当前只需要观察目录，不需要后续操作',
  });
});

test('strips a non-empty COMMAND_PLAN with a malformed closing tag from chat display', () => {
  const result = stripCommandPlanBlock(`<COMMAND_PLAN>
{
  "version": 1,
  "summary": "查看当前目录下的文件及详细信息",
  "steps": [
    {
      "description": "列出当前目录下的所有文件（包括隐藏文件），并显示详细信息",
      "command": "ls -la",
      "intent": "observe",
      "expectedOutcome": "显示当前目录下的文件和文件夹列表，包括权限、所有者、大小和修改时间"
    }
  ]
}
</COMMAND_PLAN`);

  assert.equal(result, '');
});
