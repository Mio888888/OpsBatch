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

test('SFTP download uses cross-platform local path joining', () => {
  const source = readFileSync('src/stores/sftp.ts', 'utf8');

  assert.match(source, /import \{ basenameFromPath, dirnameFromPath, joinPath \} from '\.\.\/utils\/pathNames'/);
  assert.match(source, /download: async \(hostId, remoteFilePath, localDir\) => \{[\s\S]*const localPath = joinPath\(localDir, fileName\);[\s\S]*invoke\('sftp_download'/);
});

test('remote SFTP files and folders open through the editor window', () => {
  const source = readFileSync('src/components/SftpPanel.tsx', 'utf8');
  const capabilities = readFileSync('src-tauri/capabilities/default.json', 'utf8');

  assert.match(source, /invoke\('open_managed_window', \{[\s\S]*kind: 'editor'[\s\S]*mode: entry\.is_dir \? 'dir' : 'file'[\s\S]*path: entry\.path/);
  assert.match(source, /if \(menu\.side === 'remote'\) \{[\s\S]*label: menu\.entry\.is_dir \? tText\('sftp\.ideOpenDir'\) : tText\('sftp\.ideOpenFile'\)[\s\S]*action: handleIdeOpen/);
  assert.doesNotMatch(source, /menu\.side === 'remote' && !menu\.entry\.is_dir && isPreviewable\(menu\.entry\)/);
  assert.match(capabilities, /"editor-\*"/);
});

test('SFTP dictionaries include create file labels for zh and en', () => {
  const source = readFileSync('src/i18n/dictionaries.ts', 'utf8');

  assert.match(source, /'sftp\.newFile': '新建文件'/);
  assert.match(source, /'sftp\.newFilePrompt': '新文件名称:'/);
  assert.match(source, /'sftp\.newFile': 'New file'/);
  assert.match(source, /'sftp\.newFilePrompt': 'New file name:'/);
});
