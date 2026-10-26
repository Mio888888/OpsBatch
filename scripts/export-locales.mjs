#!/usr/bin/env node
// Export OpsBatch's i18n dictionaries (src/i18n/dictionaries.ts) to locales/{zh-CN,en-US}.json.
// usage: node scripts/export-locales.mjs [path/to/OpsBatch]
import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';

const root = process.argv[2] ?? path.resolve(import.meta.dirname, '../../OpsBatch');
const src = fs.readFileSync(path.join(root, 'src/i18n/dictionaries.ts'), 'utf8')
  .replace(/export const dictionaries\s*=/, 'result =')
  .replace(/\}\s*as const\s*;?\s*$/m, '};');
const sandbox = { result: null };
vm.runInNewContext(src, sandbox);
const out = path.resolve(import.meta.dirname, '../locales');
fs.mkdirSync(out, { recursive: true });
for (const [lang, dict] of Object.entries(sandbox.result)) {
  const sorted = Object.fromEntries(Object.entries(dict).sort(([a], [b]) => (a < b ? -1 : 1)));
  fs.writeFileSync(path.join(out, `${lang}.json`), JSON.stringify(sorted, null, 2) + '\n');
  console.log(lang, Object.keys(sorted).length);
}
