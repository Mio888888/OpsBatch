import assert from 'node:assert/strict';
import test from 'node:test';
import { basenameFromPath } from '../src/utils/pathNames.ts';

test('extracts basename from POSIX paths', () => {
  assert.equal(basenameFromPath('/Users/me/Desktop/report.csv'), 'report.csv');
});

test('extracts basename from Windows desktop paths', () => {
  assert.equal(basenameFromPath(String.raw`C:\Users\me\Desktop\report.csv`), 'report.csv');
});

test('drops trailing path separators before extracting basename', () => {
  assert.equal(basenameFromPath('/Users/me/Desktop/report.csv/'), 'report.csv');
});
