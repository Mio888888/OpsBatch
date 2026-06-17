import assert from 'node:assert/strict';
import test from 'node:test';
import { isSshAuthenticationFailure } from '../src/utils/terminalAuth.ts';

test('detects SSH authentication failures in Chinese and English messages', () => {
  assert.equal(isSshAuthenticationFailure('连接失败: 认证被拒绝'), true);
  assert.equal(isSshAuthenticationFailure('Connection failed: Authentication failed'), true);
  assert.equal(isSshAuthenticationFailure('Permission denied (publickey,password)'), true);
});

test('does not treat ordinary connection failures as authentication failures', () => {
  assert.equal(isSshAuthenticationFailure('连接失败: connection refused'), false);
  assert.equal(isSshAuthenticationFailure('连接超时'), false);
  assert.equal(isSshAuthenticationFailure(undefined), false);
});
