import assert from 'node:assert/strict';
import test from 'node:test';
import { basenameFromPath, dirnameFromPath, joinPath } from '../src/utils/pathNames.ts';

test('extracts basename from POSIX paths', () => {
  assert.equal(basenameFromPath('/Users/me/Desktop/report.csv'), 'report.csv');
});

test('extracts basename from Windows desktop paths', () => {
  assert.equal(basenameFromPath(String.raw`C:\Users\me\Desktop\report.csv`), 'report.csv');
});

test('drops trailing path separators before extracting basename', () => {
  assert.equal(basenameFromPath('/Users/me/Desktop/report.csv/'), 'report.csv');
});

test('joins Windows directory paths with backslash separators', () => {
  assert.equal(joinPath('C:\\Users\\me\\Downloads\\', 'report.csv'), 'C:\\Users\\me\\Downloads\\report.csv');
});

test('joins POSIX directory paths with slash separators', () => {
  assert.equal(joinPath('/Users/me/Downloads/', 'report.csv'), '/Users/me/Downloads/report.csv');
});

test('returns Windows parent directories', () => {
  assert.equal(dirnameFromPath('C:\\Users\\me\\Downloads'), 'C:\\Users\\me');
  assert.equal(dirnameFromPath('C:\\Users\\'), 'C:\\');
});
