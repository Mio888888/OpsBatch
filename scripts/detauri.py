#!/usr/bin/env python3
"""Mechanical first pass of porting a Tauri command module to a plain service module.

usage: detauri.py <src.rs> <dst.rs>

* drops `#[tauri::command]`
* `db: tauri::State<'_, Database>` -> `db: &Database`
* drops `use tauri::...;` lines
Everything else (AppHandle/emit/State of other types) is reported and fixed by hand.
"""
import re, sys

src, dst = sys.argv[1], sys.argv[2]
text = open(src).read()
text = re.sub(r'^\s*#\[tauri::command\]\s*\n', '', text, flags=re.M)
text = re.sub(r"tauri::State<'_,\s*Database>", '&Database', text)
text = re.sub(r"State<'_,\s*Database>", '&Database', text)
text = re.sub(r'^use tauri::[^\n]*;\n', '', text, flags=re.M)
# --ctx mode: services take `Arc<Ctx>` (ops-ssh's bundle of managers) instead of Tauri's AppHandle / State<T>.
if '--ctx' in sys.argv:
    FIELD = {
        'Database': 'core.db', 'SftpManager': 'sftp', 'SshConnectionRegistry': 'registry',
        'ForwardManager': 'forwards', 'LocalFsManager': 'local_fs', 'TerminalManager': 'terminals_mgr',
    }
    def state_repl(m):
        var, ty = m.group(1), m.group(2)
        return f'{var}.{FIELD[ty]}.clone()'
    text = re.sub(r'(\w+)\.state::<(?:crate::)?(?:ssh::)?(\w+)>\(\)', state_repl, text)
    text = re.sub(r'\b(?:tauri::)?AppHandle\b', 'Arc<Ctx>', text)
    text = re.sub(r"(?:tauri::)?State<'_,\s*([\w:]+)>", r'&\1', text)
    text = re.sub(r'^use tauri_plugin_[^\n]*;\n', '', text, flags=re.M)

# crate path rewrites (ops-core split)
for a, b in [
    ('crate::db::', 'ops_core::db::'),
    ('crate::keychain::', 'ops_core::keychain::'),
    ('crate::security::', 'ops_core::security::'),
    ('crate::commands::app_log::emit_log(', 'ops_core::log::emit_log('),
    ('crate::commands::diagnostics::', 'ops_core::log::'),
]:
    text = text.replace(a, b)
text = re.sub(r'crate::commands::(\w+)::', r'crate::\1::', text)
open(dst, 'w').write(text)
left = [(i + 1, l.strip()) for i, l in enumerate(text.splitlines()) if re.search(r'tauri|AppHandle|\.emit\(|State<|try_state|\.state::<', l)]
for n, l in left:
    print(f'{dst}:{n}: {l[:140]}')
