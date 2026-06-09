# OpsBatch

[简体中文](README_CN.md) | [English](README_EN.md)

OpsBatch is a local desktop workbench for operations workflows. It centralizes server assets, SSH terminals, batch commands, script libraries, file transfer, automation workflows, and operations knowledge synchronization. The current project version is `0.1.0`. The desktop app is built with Tauri v2, React 19, TypeScript/Vite, and a Rust backend, with core data persisted in a local SQLite database.


## Product Positioning

OpsBatch is designed for people who frequently connect to, inspect, and batch-operate many servers, while avoiding the need to deploy a central server.

Target users include:

- Operations engineers, SREs, and application operations teams
- DBAs and database operations engineers
- DevOps / platform engineers
- Security operations, incident response, and on-call engineers
- Teams that need to maintain commands, scripts, host assets, and execution records

Core goals:

- Manage host assets, groups, SSH connections, and jump-host chains locally in one place.
- Turn common commands, scripts, quick actions, and workflows into reusable assets.
- Support multi-host batch execution, file distribution, interactive broadcast terminals, and execution history tracking.
- Store configuration and operation data in a local database, reducing dependency on centralized services.

## Feature Overview

### Asset and Connection Management

- Host asset management: supports host name, IP, port, authentication method, operating system, group, tag fields, and jump-host chains.
- Grouping and selection: supports nested groups, asset search, host selection, and starting terminal/command/transfer operations from selected hosts.
- Import/export: supports host CSV import and export.
- Cloud host import: supports fetching and importing instances from Aliyun, AWS, and Tencent Cloud.
- SSH config / jump hosts: supports parsing hosts and `ProxyJump` from `~/.ssh/config`, then building jump chains.

### Terminal, Monitoring, and Remote Files

- Terminal workspace: supports local terminal tabs and remote SSH terminal tabs.
- Host monitoring: after a remote connection, OpsBatch can display snapshots for CPU, memory, disk, network, processes, and related metrics. The current collection logic is mainly Linux/procfs oriented.
- SFTP file panel: supports local/remote two-pane browsing, upload, download, preview, rename, delete, mkdir, bookmarks, archive extraction, and transfer queues.
- Remote editor: files/directories can be opened from the SFTP panel, edited with CodeMirror, and saved back through SFTP.
- Port forwarding: the terminal bottom panel includes an entry for port forwarding.

### Batch Execution and File Distribution

- Batch command execution: executes commands on selected hosts, with concurrency, timeout, real-time results, failed-host retry, and history.
- Dangerous command protection: built-in and custom dangerous-command rules trigger confirmation before high-risk commands; AI can also be used to explain risks.
- Execution replay: the backend persists history and supports asciinema-style terminal replay data.
- Broadcast terminal: opens a separate batch terminal window and broadcasts input to multiple connected hosts. The current window caps selected hosts at 16.
- Batch file transfer: supports selecting a local file or directory and uploading it to multiple hosts. Remote paths support `{host}` and `{firstdir:path}` variables. The current UI is primarily focused on batch upload.

### Command Library, Script Library, and Quick Actions

- Command library: supports search, category filters, custom commands, risk levels, platform fields, remote URL commands, copy, and adding commands to quick actions.
- Script library: supports language/category/search filters, custom scripts, viewing, editing, version history, version restore, and adding scripts to quick actions.
- Quick actions: supports CRUD, category/star/language filters, JSON import/export, parameter placeholder prompts, and execution against selected hosts.
- Git repository sync: supports repository URL, branch, token, pull strategy, enabled state, manual sync, and scheduled checks. Commands, scripts, and quick actions can be imported into local libraries from repositories.

### Workflow Orchestration

- Visual workflows: includes a workflow list, editor, templates, and execution entry based on React Flow.
- Node types: start/end, select host, command, script, quick action, transfer, condition, switch, delay, manual confirmation, rollback, and related nodes.
- Workflow execution: the frontend executor runs nodes according to dependencies, supports result variables, condition/switch paths, and manual confirmation.
- Scheduled tasks: the backend persists workflow scheduled tasks. Current scheduling mainly parses simple `every:N` intervals; it is not a full Cron expression scheduler.

### AI, Logs, and Settings

- AI configuration: supports OpenAI, local Ollama, and custom OpenAI-compatible providers.
- AI capabilities: backend support includes chat, streaming chat, script generation, command/result analysis, error diagnosis, and risk assessment.
- Terminal AI panel: starts AI conversations from terminal context, generates commands, and executes them after user confirmation.
- Global logs: backend logs and frontend console/window errors are collected in a global log window and persisted.
- Settings: includes general, appearance, quick actions, command library, script library, AI, dangerous rules, and data backup sections.
- Internationalization: the app supports `zh-CN` and `en-US`, with a system-language option.

## Architecture and Tech Stack

### High-Level Data Flow

```text
React/Vite UI
  -> Zustand stores
  -> Tauri invoke() commands and Tauri events
  -> Rust command modules
  -> SQLite / SSH / SFTP / local PTY / Git / HTTP / OS keychain
```

### Tech Stack

| Layer | Technology |
| --- | --- |
| Desktop container | Tauri v2 |
| Frontend build | Vite 7, TypeScript 5.8 |
| UI framework | React 19, React Router 7 |
| State management | Zustand 5 |
| UI foundation | Radix UI primitives, project-owned `src/components/ui` wrappers, `App.css` variables/classes |
| Icons | `lucide-react` |
| Terminal | xterm.js |
| Editor | CodeMirror / `@uiw/react-codemirror` |
| Workflow canvas | React Flow / `@xyflow/react` |
| Drag-and-drop / animation | dnd-kit, GSAP |
| Backend language | Rust |
| Backend capabilities | Tauri commands, SQLite (`rusqlite`), SSH/SFTP (`russh`, `russh-sftp`), local PTY, Git (`git2`), HTTP (`reqwest`) |
| Local data | SQLite database `opsbatch.db` under the Tauri app data directory |

## Repository Structure

```text
.
├── README.md                 # Chinese README, default entry
├── README_CN.md              # Chinese README copy / named version
├── README_EN.md              # English README
├── package.json              # Desktop/web frontend and Tauri CLI scripts
├── vite.config.ts            # Vite build configuration
├── tsconfig.json             # TypeScript configuration
├── src/                      # React desktop frontend source
│   ├── components/           # Layout, SFTP, AI, UI wrappers, and shared components
│   ├── pages/                # Terminal, Commands, Libraries, Workflow, Settings, etc.
│   ├── stores/               # Zustand domain stores and Tauri invoke wrappers
│   ├── types/                # Frontend domain types
│   └── i18n/                 # zh-CN / en-US dictionaries and language resolution
├── src-tauri/                # Tauri v2 Rust backend
│   ├── src/commands/         # hosts, terminal, execution, sftp, workflow, ai, etc.
│   ├── src/db/               # SQLite schema and migrations
│   ├── Cargo.toml            # Rust dependencies
│   ├── tauri.conf.json       # Tauri app and bundle configuration
│   └── tauri.windows.conf.json # Windows signing extension config
├── tests/                    # Node test files
├── scripts/                  # Release and signing helper scripts
├── website/                  # Standalone static website and download page
├── docs/superpowers/         # Project specs, plans, and collaboration notes
└── .github/workflows/        # Build and GitHub Pages workflows
```

## Prerequisites

Recommended environment:

- Node.js and npm. The project uses npm scripts.
- Rust toolchain (`cargo`, `rustc`).
- System dependencies required by Tauri v2. Requirements vary by platform; see the [Tauri official prerequisites](https://v2.tauri.app/start/prerequisites/).
- Git, for repository synchronization and general development.
- Reachable SSH hosts, cloud provider credentials, or AI service credentials are optional runtime configuration, not required to start the development environment.

## Getting Started

### 1. Install Dependencies

```bash
npm install
```

### 2. Start Desktop Development Mode

```bash
npm run tauri -- dev
```

This follows `src-tauri/tauri.conf.json`: it starts the Vite dev server first, then launches the Tauri desktop window.

If you only need the frontend Vite dev server:

```bash
npm run dev
```

## Common Commands

| Command | Description |
| --- | --- |
| `npm run dev` | Start the desktop frontend Vite dev server |
| `npm run tauri -- dev` | Start Tauri desktop development mode |
| `npm run build` | Run `tsc && vite build` and build the desktop frontend assets |
| `npm run tauri -- build` | Build the Tauri desktop app / installer bundle |
| `npm run preview` | Preview Vite build output |
| `npm run build:all` | Desktop frontend build alias, equivalent to `npm run build` |
| `cargo check --manifest-path src-tauri/Cargo.toml` | Check Rust/Tauri backend types and compilation issues |
| `npx tsc --noEmit` | Run TypeScript type checking only |

> The repository currently does not define a dedicated `lint` npm script. Frontend verification mainly relies on TypeScript checks and Vite builds; backend verification can use `cargo check`.

## Windows Releases and SmartScreen

If the Windows installer is unsigned, or if the signing certificate/publisher has not built enough Microsoft reputation yet, users may see the Microsoft Defender SmartScreen warning that Windows protected the PC from an unrecognized app. This cannot be disabled from application runtime code. Production Windows releases should be signed with a trusted Authenticode code-signing certificate.

The repository includes a Windows-specific Tauri config at `src-tauri/tauri.windows.conf.json`. When `npm run tauri -- build` runs on Windows, Tauri automatically calls `scripts/windows-sign.ps1` to sign generated Windows executables and installers. If certificate environment variables are not configured, the script skips Authenticode signing with a warning and the build continues with unsigned release artifacts.

Recommended release flow:

1. Prepare an OV or EV code-signing certificate. EV certificates usually establish SmartScreen trust more quickly; OV certificates still need reputation over time.
2. Install the Windows SDK on the Windows build machine so `signtool.exe` is available.
3. Provide signing material via environment variables. Do not commit certificate files or passwords:

```powershell
$env:WINDOWS_CODESIGN_CERT_PATH = 'C:\secure\OpsBatch.pfx'
$env:WINDOWS_CODESIGN_CERT_PASSWORD = '<pfx-password>'
npm run tauri -- build
```

If the certificate is installed in the Windows certificate store, use its thumbprint instead:

```powershell
$env:WINDOWS_CODESIGN_CERT_THUMBPRINT = '<certificate-sha1-thumbprint>'
npm run tauri -- build
```

To sign existing artifacts after a build, run the script directly:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\windows-sign.ps1 .\src-tauri\target\release\bundle
```

Verify signed artifacts with `signtool verify /pa /v <installer.exe>`. Even signed apps can still trigger SmartScreen for a while when the certificate or app is new; using the same publisher certificate, stable versioning, and official distribution channels helps build reputation.

## Local Data and Security Notes

OpsBatch is primarily a local desktop client. Core data is stored on the local machine:

- On startup, Tauri opens or creates the SQLite database `opsbatch.db` under the system app data directory.
- The database contains hosts, groups, tags, execution history, command libraries, script libraries, quick actions, Git repository configuration, dangerous-command rules, general settings, workflows, AI conversations, RAG/MCP tables, global logs, and related tables.
- The settings page provides database backup by copying the current `opsbatch.db` to a user-selected destination.

Please note the current security status and avoid over-assumptions:

- Host passwords, private-key fields, and Git repository tokens are preferentially stored in the OS keychain, while SQLite stores the `***keychain***` placeholder; existing historical data is also migrated to the keychain on startup when possible.
- AI API keys have best-effort logic for storing in the OS keychain, but the current configuration save path still writes the real key to local settings JSON. Do not describe all sensitive data as fully encrypted.
- Cloud provider credentials and some runtime configuration may still be stored in the local database or config data. Evaluate risk according to device trust and credential scope.
- SSH host keys use a trust-on-first-use (TOFU) fingerprint flow. Later fingerprint mismatches block the connection and ask the user to re-confirm the host. The first connection still requires a trusted network and host.
- Tauri now has a baseline CSP that restricts script, object, and frame loading while allowing required IPC, HTTP(S), local development services, and media resources. Reassess and tighten the policy when adding new resource origins for production.
- Avoid storing high-privilege credentials on untrusted devices. In team environments, use least-privilege accounts, temporary credentials, jump-host auditing, and system disk encryption where appropriate.

## Current Status and Known Limitations

- README language mapping: `README.md` and `README_CN.md` are both complete Chinese documents; `README_EN.md` is the English document. Keep the two Chinese files synchronized in future updates.
- Workflow scheduling: scheduled tasks mainly use simple `every:N` interval parsing and are not yet a full Cron expression scheduler.
- RAG/MCP: backend commands and tables exist for RAG collection/import/search and MCP server/tool operations, but the desktop route map does not currently include standalone top-level RAG or MCP pages.
- Monitoring: host monitoring commands are mainly Linux/procfs oriented; metric completeness may differ on other systems.
- Batch terminal: the broadcast terminal window currently limits selected hosts to 16.
- Batch transfer: the current UI primarily presents batch upload; the transfer concurrency input is not fully passed through to the backend request yet.
- Git repository sync: the current implementation expects language-specific metadata/file conventions, such as `library_cn.json`, `library_en.json`, and corresponding suffixes. Prepare repository content according to the current implementation.

## Development and Contribution Notes

- Before modifying features, read the relevant source files, specs/plans under `docs/superpowers/`, and the corresponding page, store, and Rust command implementations.
- Frontend types are centralized in `src/types/index.ts`; pages live under `src/pages/`; shared components live under `src/components/`; stores and backend-call wrappers live under `src/stores/`.
- The frontend calls Tauri `invoke()` through Zustand stores. Backend commands often use snake_case fields, and frontend stores normalize them to camelCase domain types.
- Prefer project-owned `src/components/ui` wrappers and `App.css` variables/classes for UI. Application text already supports Chinese and English language modes.
- When adding persisted backend capabilities, consider SQLite schema/migrations, Tauri command registration, frontend types, store calls, and verification commands together.
- The product website lives under `website/` and is deployed to GitHub Pages through `.github/workflows/website.yml`. Desktop installers are built and published by `.github/workflows/build.yml` when the version tag does not already exist.
- After documentation or code changes, run commands according to the affected scope:

```bash
npm run build
cargo check --manifest-path src-tauri/Cargo.toml
```

Documentation-only changes usually do not require a full build, but README commands and feature descriptions should remain aligned with the actual repository implementation.

## Related Documents

- [README_CN.md](README_CN.md) — Chinese README.
- [README_EN.md](README_EN.md) — English README.
- [website/README.md](website/README.md) — Static website preview and deployment notes.
