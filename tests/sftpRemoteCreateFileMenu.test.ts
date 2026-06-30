import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

test('remote SFTP context menu exposes create file for files and folders', () => {
  const source = readFileSync('src/components/SftpPanel.tsx', 'utf8');

  assert.match(source, /const remoteCreateFile = useSftpStore\(\(s\) => s\.remoteCreateFile\)/);
  assert.match(source, /const handleCreateFile = async \(\) => \{[\s\S]*prompt\(tText\('sftp\.newFilePrompt'\)\)[\s\S]*remoteCreateFile\(hostId, newPath\)/);
  assert.match(source, /if \(menu\.side === 'remote'\) \{[\s\S]*items\.push\(\{ label: tText\('sftp\.newFile'\), action: handleCreateFile, separator: true \}\);[\s\S]*items\.push\(\{ label: tText\('sftp\.newFolder'\), action: handleMkdir \}\);[\s\S]*\}/);
});

test('SFTP store creates a remote file through the backend write command and refreshes', () => {
  const source = readFileSync('src/stores/sftp.ts', 'utf8');

  assert.match(source, /remoteCreateFile: \(hostId: string, path: string\) => Promise<void>/);
  assert.match(source, /remoteCreateFile: async \(hostId, path\) => \{[\s\S]*await invoke\('sftp_write_file', \{ hostId, path, content: '' \}\);[\s\S]*await get\(\)\.refreshRemote\(hostId\);[\s\S]*\}/);
});

test('SFTP dictionaries include create file labels for zh and en', () => {
  const source = readFileSync('src/i18n/dictionaries.ts', 'utf8');

  assert.match(source, /'sftp\.newFile': '新建文件'/);
  assert.match(source, /'sftp\.newFilePrompt': '新文件名称:'/);
  assert.match(source, /'sftp\.newFile': 'New file'/);
  assert.match(source, /'sftp\.newFilePrompt': 'New file name:'/);
});
