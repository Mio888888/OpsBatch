# OpsBatch

**English** | [简体中文](README_CN.md) | [Website](https://mio888888.github.io/OpsBatch/)

[![CI](https://github.com/Mio888888/OpsBatch/actions/workflows/ci.yml/badge.svg)](https://github.com/Mio888888/OpsBatch/actions/workflows/ci.yml)
[![License: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](LICENSE)

OpsBatch is a local desktop workbench for operations work: SSH terminals, SFTP, batch commands, script and command
libraries, quick actions, RDP / VNC remote desktops, AI assistance, workflows and a global log in one app. Everything is
stored on your machine in a SQLite database and an encrypted vault; there is no central server to deploy.

This is the native Rust build on [GPUI](https://gpui-kit.com/docs/) (`gpui-kit =0.8.0`). It replaces the Tauri + React
build, which lives on the [`0.x`](https://github.com/Mio888888/OpsBatch/tree/0.x) branch, and reads and writes the same
data.

## Features

| Area | What you get |
| --- | --- |
| Assets | Hosts with groups, tags, search, CSV import / export, cloud-instance import (Alibaba Cloud, AWS, Tencent Cloud), `~/.ssh/config` and `ProxyJump` chains, SOCKS5 / HTTP proxies |
| Terminal | Local and SSH terminal tabs, command tracking, dangerous-command guard, in-terminal search, port forwarding, host monitoring (CPU, memory, disk, network, processes) |
| SFTP and editor | Two-pane local / remote browser, transfer queue, previews, bookmarks, remote file editing with syntax highlighting |
| Batch | Run commands on many hosts with concurrency, timeout, retry and history; asciinema-style replay; broadcast terminal; batch upload with `{host}` variables |
| Libraries | Command library, script library with version history, quick actions, Git-repository sync of all three |
| Workflows | Visual canvas with host, command, script, transfer, condition, branch, delay and approval nodes |
| Remote desktop | RDP on [IronRDP](https://github.com/Devolutions/IronRDP) (H.264 and bitmap paths, clipboard, audio, drive redirection) and a native RFB client for VNC (None / VNC / Apple ARD authentication, Raw, CopyRect, Hextile and ZRLE encodings) |
| AI | OpenAI-compatible chat with host context, risk explanations for commands, RAG knowledge bases, provider / agent / safety settings, and an MCP bridge that lets external agents drive OpsBatch with per-call approval |
| Everything else | Chinese / English UI, follow-system appearance, 14 terminal themes, UI scaling, tray icon, global log window, in-app updates |

Feature-by-feature parity with the Tauri build, with the known differences, is tracked in
[`FEATURE_MATRIX.md`](FEATURE_MATRIX.md) (Chinese).

## Install

Download the installer for your platform from [Releases](https://github.com/Mio888888/OpsBatch/releases):

| Platform | File |
| --- | --- |
| macOS (Apple silicon, Intel) | `OpsBatch-<version>-macos-<aarch64\|x86_64>.dmg` |
| Windows (x64) | `OpsBatch-<version>-windows-x86_64-setup.exe` |
| Linux (x64) | `OpsBatch-<version>-linux-x86_64.AppImage`, or `opsbatch_<version>_amd64.deb` |

Builds that are not signed with an Apple Developer ID are blocked by Gatekeeper after download; clear the quarantine flag
once with `xattr -cr /Applications/OpsBatch.app`.

### Coming from the Tauri build

* **Same data.** The app reads and writes the Tauri build's database and vault (the `com.mio.OpsBatch` application-data
  directory), so nothing needs exporting.
* **Do not run both builds at the same time**; they share one SQLite file and one vault.
* **Updating.** Releases publish a `latest.json` signed with the same updater key, so an installed Tauri build offers
  the update like any other release.
* *Settings → Data → Export* writes a plain copy of `opsbatch.db` that either build can restore.

### In-app updates

The update button in the top bar shows a red dot when a newer release exists. The app downloads the installer for its
platform, verifies it against the release's `SHA256SUMS` (a missing entry or a mismatch aborts and deletes the file), then
opens it: the `.dmg` on macOS, the setup program on Windows, the folder of the AppImage on Linux. *Restart* quits the
app so the installer can replace it.

When an updater key is built in (`OPSBATCH_UPDATER_PUBKEY`, the same base64 public key as the Tauri updater), every
installer must also carry a valid minisign signature `<installer>.sig`; a missing or wrong signature aborts the download,
as it does in the Tauri updater. Without a key, authenticity rests on HTTPS to GitHub and the checksum file.

## Build from source

You need a stable Rust toolchain. On Linux, install the GPUI system packages first (the list is in
[`.github/workflows/ci.yml`](.github/workflows/ci.yml)).

```sh
cargo run -p opsbatch        # debug build; uses the isolated "-dev" data directory
cargo test --workspace       # every crate plus the headless UI tests (no display needed)
cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings   # what CI checks
```

Without the `real-vault` feature, host passwords and keys live in memory only, and the app deliberately uses a separate
`com.mio.OpsBatch-dev` data directory so it can never strip secrets from a Tauri installation. **Production builds need
`OPSBATCH_FEATURES=real-vault`**, which keeps the encrypted vault in the system keystore (macOS Keychain, Windows
Credential Manager, Linux Secret Service).

| Variable | Effect |
| --- | --- |
| `OPSBATCH_FEATURES` | Cargo features for the packaging scripts (`real-vault` for production). |
| `OPSBATCH_DATA_DIR` | Use this directory instead of the platform data directory (tests, side-by-side profiles). |
| `OPSBATCH_SKIP_UPDATE_CHECK` | Do not look for a new release at startup. |
| `OPSBATCH_SKIP_REPO_UPDATE` | Do not pull "update on startup" library repositories at startup. |
| `OPSBATCH_UPDATE_BASE` | Use `http://host:port` as the release source instead of GitHub (same URL layout; for tests and mirrors). |
| `OPSBATCH_UPDATER_PUBKEY` | Updater public key, at build time or at run time (see above). |
| `OPSBATCH_EXIT_AFTER` | Quit after N seconds (smoke tests). |

### Packaging

The scripts in [`packaging/`](packaging) write into `dist/`:

| Platform | Command | Output |
| --- | --- | --- |
| macOS | `packaging/macos/build.sh` | `OpsBatch.app`, `OpsBatch-<version>-macos-<arch>.dmg` |
| Windows | `powershell -File packaging\windows\build.ps1` (needs Inno Setup 6) | `OpsBatch-<version>-windows-<arch>-setup.exe` |
| Linux | `packaging/linux/build.sh` (needs `appimagetool`, `dpkg-deb`) | `OpsBatch-<version>-linux-<arch>.AppImage`, `opsbatch_<version>_<arch>.deb` |

Signing is opt-in and skipped with a warning when its variables are absent: `MACOS_SIGN_IDENTITY` (plus `APPLE_ID`,
`APPLE_TEAM_ID`, `APPLE_APP_PASSWORD` to notarize), and `WINDOWS_CODESIGN_CERT_PATH` with
`WINDOWS_CODESIGN_CERT_PASSWORD` or `WINDOWS_CODESIGN_CERT_THUMBPRINT`.

## Releasing

[`.github/workflows/release.yml`](.github/workflows/release.yml) builds the installers for macOS (Apple silicon and
Intel), Windows and Linux with the real vault, and signs them with the updater key.

* A push to `master` only builds; the installers are kept as workflow artifacts.
* **To publish**, raise `version` in `Cargo.toml` (an installed Tauri build only offers a higher version) and push the
  tag `v<version>`, or run the workflow by hand with *publish* ticked. The tag must equal the version.
* A release carries what both updaters need: the installers, `SHA256SUMS` and the `.sig` files for this build, and
  `latest.json` with the `.app.tar.gz`, setup program and AppImage for the Tauri updater. It is created as a draft and
  published last, and a published version is never overwritten.

Repository secrets: `TAURI_UPDATER_PUBKEY`, `TAURI_UPDATER_PRIVATE_KEY`, `TAURI_UPDATER_PRIVATE_KEY_PASSWORD` (the
updater key; publishing fails without it), and optionally `MACOS_CERT_P12`, `MACOS_CERT_PASSWORD`,
`MACOS_SIGN_IDENTITY`, `APPLE_ID`, `APPLE_TEAM_ID`, `APPLE_APP_PASSWORD`, `WINDOWS_CODESIGN_CERT_BASE64`,
`WINDOWS_CODESIGN_CERT_PASSWORD`.

## Repository layout

| Path | Contents |
| --- | --- |
| `app/` | The GPUI application: windows, views, stores and the headless UI tests |
| `crates/ops-core` | Models, SQLite database, vault and keychain, settings, event bus, update check |
| `crates/ops-ssh` | SSH connection pool, jump hosts, terminal channels, SFTP, forwarding, batch exec and transfer |
| `crates/ops-terminal` | Terminal logic without UI: command tracking, danger guard, search |
| `crates/ops-rdp`, `crates/ops-vnc` | RDP client on IronRDP; native RFB client |
| `crates/ops-ai` | AI client, action parser, risk assessment, RAG, MCP, preferences |
| `crates/ops-library` | Command / script / quick-action library, repository sync, cloud import |
| `crates/ops-workflow` | Workflow model, storage, executor, scheduler |
| `crates/ops-i18n`, `locales/` | Chinese and English dictionaries and `t!()` |
| `packaging/`, `scripts/` | Installer scripts, checksum, signing and `latest.json` helpers |
| `vendor/`, `spikes/` | Patched third-party crates; early prototypes |

The plan and per-milestone reports are in [`PLAN.md`](PLAN.md) and [`TASKS.md`](TASKS.md) (Chinese).

## Contributing

Issues and pull requests are welcome; read [`.github/CONTRIBUTING.md`](.github/CONTRIBUTING.md) first. Report security
problems privately as described in [`.github/SECURITY.md`](.github/SECURITY.md). The product site lives in
[`website/`](website) and is published to GitHub Pages.

## License

Copyright © 2025-2026 Mio. Licensed under the [GNU Affero General Public License v3.0](LICENSE) (`AGPL-3.0-only`): if you
modify OpsBatch and run it as a network service, or distribute it, you must offer the corresponding source code of your
version under the same license.
