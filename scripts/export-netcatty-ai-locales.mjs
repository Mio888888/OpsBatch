#!/usr/bin/env node
// Export the AI-settings strings of Netcatty (https://github.com/binaricat/Netcatty) into
// locales/extra/{zh-CN,en-US}.json under the `ncai.` prefix. The product name is rebranded to OpsBatch.
// usage: node scripts/export-netcatty-ai-locales.mjs path/to/Netcatty
import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';

const root = process.argv[2];
if (!root) {
  console.error('usage: export-netcatty-ai-locales.mjs <netcatty checkout>');
  process.exit(1);
}
const dir = path.join(root, 'application/i18n/locales');

function load(file) {
  const src = fs
    .readFileSync(file, 'utf8')
    .replace(/^import .*$/gm, '')
    .replace(/export const \w+: Messages =/, 'result =');
  const sandbox = { result: null };
  vm.runInNewContext(src, sandbox);
  return sandbox.result;
}

const rebrand = (s) => s.replace(/Netcatty/g, 'OpsBatch').replace(/netcatty/g, 'opsbatch').replace(/NETCATTY/g, 'OPSBATCH');
const extra = { 'confirm.removeProvider': 'ncai.confirmRemoveProvider', 'action.remove': 'ncai.actionRemove', 'ai.chat.targetLabel': 'ncai.chat.targetLabel' };

const targets = {
  'zh-CN': [path.join(dir, 'zh-CN/ai.ts'), path.join(dir, 'zh-CN/core.ts')],
  'en-US': [path.join(dir, 'en/ai.ts'), path.join(dir, 'en/core.ts')],
};
const outDir = path.resolve(import.meta.dirname, '../locales/extra');
for (const [lang, files] of Object.entries(targets)) {
  const merged = Object.assign({}, ...files.map(load));
  const out = {};
  for (const [key, value] of Object.entries(merged)) {
    let name = null;
    if (extra[key]) name = extra[key];
    else if (key.startsWith('ai.') && !key.startsWith('ai.chat.')) name = `ncai.${key.slice(3)}`;
    if (name) out[name] = rebrand(String(value));
  }
  const file = path.join(outDir, `${lang}.json`);
  const current = JSON.parse(fs.readFileSync(file, 'utf8'));
  for (const k of Object.keys(current)) if (k.startsWith('ncai.')) delete current[k];
  const next = Object.fromEntries(Object.entries({ ...current, ...out }).sort(([a], [b]) => (a < b ? -1 : 1)));
  fs.writeFileSync(file, JSON.stringify(next, null, 2) + '\n');
  console.log(lang, Object.keys(out).length);
}
