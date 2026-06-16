# OpsBatch

[简体中文](README_CN.md) | [English](README_EN.md)

![](website/assets/screenshots/linux-ai-assistant.png)

OpsBatch is a local desktop workbench for operations, built with **Tauri v2 + React 19 + TypeScript + Rust**, with core data persisted in a local SQLite database.
Without deploying any central service, you can manage server assets, SSH terminals, RDP/VNC remote desktop, batch commands, script libraries, file transfer, automation workflows, and operations knowledge synchronization in one place.

## ✨ Features

### Asset & Connection Management

- **Host asset management**: host name, IP, port, auth method, OS, group, tags, jump-host chains, etc.
- **Grouping & selection**: nested groups, asset search, host selection, and starting terminal/command/transfer actions from selected hosts.
- **Import & export**: host CSV import and export.
- **Cloud host import**: fetch and import instances from Aliyun, AWS, and Tencent Cloud.
- **SSH config / jump hosts**: parse hosts and `ProxyJump` from `~/.ssh/config` and build jump chains automatically.

### Terminal, Monitoring & Remote Files

- **Terminal workspace**: local terminal and remote SSH terminal tabs.
- **Host monitoring**: CPU, memory, disk, network, and process snapshots after connecting (mainly for Linux/procfs).
- **SFTP file panel**: local/remote dual-pane browse, upload, download, preview, rename, delete, mkdir, bookmarks, extract, and transfer queue.
- **Remote editor**: open remote files/dirs from the SFTP panel, edit with CodeMirror, and save via SFTP.
- **Port forwarding**: entry in the terminal bottom panel.

### Remote Desktop

- **RDP**: connect to Windows hosts in a standalone window with domain, resolution, clipboard, audio, and drive mapping; H.264 direct and bitmap rendering paths.
- **VNC**: connect via noVNC and a local WebSocket bridge, with port, username, password, shared connection, and read-only mode.
- **Diagnostics & logs**: RDP/VNC pages output connection, framerate, transfer, and diagnostic logs for troubleshooting.

### Batch Execution & File Distribution

- **Batch command execution**: run against selected hosts with concurrency, timeout, live results, retry on failure, and history (connection semaphore limits concurrency to 4).
- **Dangerous command guard**: built-in and custom rules, confirm before execution, AI risk explanation, and globally precompiled regexes.
- **Execution replay**: history and asciinema-style terminal replay data.
- **Broadcast terminal**: standalone batch terminal window broadcasting input to multiple hosts (up to 16).
- **Batch file transfer**: upload local files/dirs to multiple hosts; remote path supports `{host}` and `{firstdir:path}` variables.

### Command, Script Libraries & Quick Actions

- **Command library**: search, category filter, custom commands, risk level, platform field, remote URL commands, copy, and add to quick actions.
- **Script library**: language/category/search filter, custom scripts, version history, restore versions, and add to quick actions.
- **Quick actions**: CRUD, category/star/language filter, JSON import/export, parameter placeholder hints, and execute against selected hosts.
- **Git repo sync**: configure repo URL, branch, token, pull strategy, and scheduled checks to import commands/scripts/quick actions into local libraries.

### Workflow Orchestration

- **Visual workflow**: React Flow-based workflow list, editor, templates, and execution entry.
- **Node types**: start/end, select hosts, command, script, quick action, transfer, condition, branch, delay, manual confirmation, rollback, etc.
- **Workflow execution**: frontend executor runs by node dependencies, with result variables, conditional/branch paths, and manual confirmation.

### AI Assistant & Knowledge Base

- **AI sessions**: OpenAI-compatible API, local and host-context sessions, with RAG retrieval support.
- **RAG knowledge base**: collection management and import for retrieval-augmented generation; underlying MCP/RAG capabilities are ready in the backend.
- **AI assistance**: command risk explanation, script explanation, and operations Q&A.

## 📦 Installation & Usage

### Download

Go to [Releases](https://github.com/Mio888888/OpsBatch/releases) to download the package for your platform:

- **Windows**: `.msi` / `.exe`
- **macOS**: `.dmg`
- **Linux**: `.deb` / `.AppImage` / `.rpm`

After installation, launch OpsBatch; configuration is managed in the local database and an encrypted vault. Unlock the vault on first use as prompted.

### Local Development

```bash
# Install frontend dependencies
npm install

# Development mode (starts Tauri as well)
npm run tauri dev

# Build production bundle
npm run tauri build
```

## 🔨 Build

```bash
# Frontend type check and build
npm run build

# Backend compile check
cargo check --manifest-path src-tauri/Cargo.toml

# Full desktop packaging
npm run tauri build
```

Desktop installers are built and published by `.github/workflows/build.yml` when the version tag does not already exist; the website is published to GitHub Pages via `.github/workflows/website.yml`.

## 📁 Directory Structure

```
OpsBatch/
├── src/                # Frontend source (React + TypeScript)
│   ├── pages/          # Pages
│   ├── components/     # Shared components / UI
│   ├── stores/         # Zustand stores & backend calls
│   ├── types/          # Frontend type definitions
│   ├── hooks/          # Custom Hooks
│   └── i18n/           # Chinese/English copy
├── src-tauri/          # Rust backend + Tauri config
│   ├── src/            # Rust impl (SSH, SFTP, RDP, VNC, SQLite, etc.)
│   └── tauri.conf.json # Tauri config
├── website/            # Static website
├── docs/               # Docs and specs
├── tests/              # Tests
└── package.json
```

## 🛠 Tech Stack

| Layer | Technology |
| --- | --- |
| Desktop framework | Tauri v2 |
| Frontend | React 19, TypeScript, Vite, Zustand |
| Backend | Rust |
| Terminal | xterm.js (WebGL/Serialize) |
| Editor | CodeMirror 6 |
| Workflow | React Flow (@xyflow/react) |
| SSH/SFTP | russh, russh-sftp |
| Remote desktop | ironrdp (RDP), noVNC (VNC) |
| Database | rusqlite (SQLite) |

## 🚀 Performance

- **DB connection pool**: replace single-connection Mutex with r2d2 pool to reduce lock contention under high concurrency.
- **SSH connection pool**: share a single runtime via the pool instead of one tokio Runtime per connection.
- **Batch execution memory**: use `Arc<str>` instead of repeated String clones to reduce allocations.
- **Regex precompilation**: dangerous-command and RAG tokenize/chunk regexes are global static constants.
- **File preview memory**: remove the `Array<number>` intermediate in SFTP preview, cutting ~5x memory bloat.
- **First paint**: lazy-load CodeEditor, optimize batched prefetchPages, and fix broken lazy loading.
- **SSH idle reaping**: fix idle reaper silently dropping connections without recycling, add active health checks.

## ⚠️ Security & Privacy

- Hosts, history, command/script libraries, workflows, AI sessions, and logs are mainly stored in the local SQLite database or local config.
- Host passwords, private keys, Git tokens, and AI API keys are stored in an encrypted local vault; after unlocking at startup, the vault master key is reused in memory for the session.
- Do not store high-privilege credentials on untrusted devices; assess risk based on device trust, disk encryption, and team audit requirements.

## 📌 Current Status & Known Limitations

- **Workflow scheduling**: currently parsed as simple `every:N` intervals, not a full cron scheduler.
- **RAG/MCP**: related commands and tables exist in the backend, but there is no standalone top-level page in the desktop app yet.
- **Monitoring collection**: mainly targets Linux/procfs; metrics on other systems may be less complete.
- **Batch terminal**: broadcast terminal window supports up to 16 hosts.
- **Batch transfer**: the current UI focuses on batch upload; transfer concurrency is not fully forwarded to the backend yet.

## 🤝 Development & Contributing

- The frontend calls Tauri `invoke()` via Zustand stores; backend commands use snake_case fields, and stores convert them to camelCase.
- UI prefers `src/components/ui` wrappers and `App.css` variables/classes; copy supports Chinese and English.
- When adding persistence, also update SQLite schema/migrations, Tauri command registration, frontend types, and store calls.
- Before modifying a feature, read the related source and specs/plans under `docs/superpowers/`.

## 📄 Related Docs

- [README.md](README.md) — README (Chinese)
- [README_CN.md](README_CN.md) — Chinese README
- [website/README.md](website/README.md) — Static website preview and publishing notes

## 🙏 Acknowledgements

OpsBatch would not exist without the following open-source projects and communities:

- [Tauri](https://tauri.app) — cross-platform desktop application framework
- [React](https://react.dev) / [TypeScript](https://www.typescriptlang.org) / [Vite](https://vitejs.dev) — frontend stack
- [xterm.js](https://xtermjs.org) — terminal emulator for the web
- [CodeMirror](https://codemirror.net) — code editor
- [React Flow](https://reactflow.dev) — visual workflow orchestration
- [Zustand](https://github.com/pmndrs/zustand) — lightweight state management
- [russh](https://github.com/warp-tech/russh) / [russh-sftp](https://github.com/warp-tech/russh) — Rust SSH/SFTP
- [ironrdp](https://github.com/Devolutions/IronRDP) — RDP protocol implementation
- [noVNC](https://github.com/novnc/noVNC) — VNC client
- [rusqlite](https://github.com/rusqlite/rusqlite) / [r2d2](https://github.com/sfackler/r2d2) — SQLite and connection pooling
- [tokio](https://tokio.rs) — Rust async runtime
- And all other dependencies and their contributors

- [linux.do](https://linux.do) — community support and discussion

## 📝 License

OpsBatch and its dependencies follow the open-source and third-party licenses declared in the repository; distribution, modification, or integration must comply with those licenses.
SSH hosts, cloud platforms, Git repositories, AI services, etc. accessed via OpsBatch remain subject to their service terms, organizational policies, and regional regulations.

The README, app UI, risk notices, and AI output do not constitute legal, compliance, security audit, or professional operations advice; consult qualified professionals for production changes, sensitive data, or regulated industries.
