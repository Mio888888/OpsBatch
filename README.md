# OpsBatch (GPUI)

[简体中文](#简体中文) · [English](#english)

---

## English

OpsBatch is a desktop workbench for operations: SSH terminals, SFTP, batch commands, script / command libraries,
quick actions, RDP / VNC remote desktops, AI assistance, workflows and a global log in one app. This repository is the
native port of the Tauri + React build ([`../OpsBatch`](../OpsBatch)) to Rust + [GPUI](https://gpui-kit.com/docs/)
(`gpui-kit =0.8.0`). Feature parity is tracked in [`FEATURE_MATRIX.md`](FEATURE_MATRIX.md), the plan and per-milestone
reports in [`PLAN.md`](PLAN.md) and [`TASKS.md`](TASKS.md).

### Build and run

```sh
cargo run -p opsbatch                 # debug build, uses the isolated "-dev" data directory (see below)
cargo test --workspace                # all crates + headless UI tests (no display needed)
```

Linux needs the usual GPUI system packages (see `.github/workflows/ci.yml`).

Environment variables:

| Variable | Effect |
| --- | --- |
| `OPSBATCH_DATA_DIR` | Use this directory instead of the platform data directory (tests, side-by-side profiles). |
| `OPSBATCH_SKIP_UPDATE_CHECK` | Do not look for a new release at startup. |
| `OPSBATCH_SKIP_REPO_UPDATE` | Do not pull "update on startup" library repositories at startup. |
| `OPSBATCH_UPDATE_BASE` | Use `http://host:port` as the release source instead of GitHub (same URL layout; used for testing and mirrors). |
| `OPSBATCH_EXIT_AFTER` | Quit after N seconds (smoke tests). |

### Packaging

Installers are produced by scripts in [`packaging/`](packaging); each one writes into `dist/`:

| Platform | Command | Output |
| --- | --- | --- |
| macOS | `packaging/macos/build.sh` | `OpsBatch.app`, `OpsBatch-<version>-macos-<arch>.dmg` |
| Windows | `powershell -File packaging\windows\build.ps1` (needs Inno Setup 6) | `OpsBatch-<version>-windows-<arch>-setup.exe` |
| Linux | `packaging/linux/build.sh` (needs `appimagetool`, `dpkg-deb`) | `OpsBatch-<version>-linux-<arch>.AppImage`, `opsbatch_<version>_<arch>.deb` |

`scripts/checksums.sh dist` writes `dist/SHA256SUMS`. The release workflow
([`.github/workflows/release.yml`](.github/workflows/release.yml)) builds all of them on a `v*` tag and publishes the
release with the checksum file.

Signing is opt-in through environment variables / repository secrets and is skipped (with a warning) when absent:
`MACOS_SIGN_IDENTITY` (+ `APPLE_ID`, `APPLE_TEAM_ID`, `APPLE_APP_PASSWORD` to notarize), `WINDOWS_CODESIGN_CERT_PATH` +
`WINDOWS_CODESIGN_CERT_PASSWORD` or `WINDOWS_CODESIGN_CERT_THUMBPRINT`.

**Production builds need `OPSBATCH_FEATURES=real-vault`.** The encrypted vault / OS keychain is the last milestone item
(TASKS M1-05, M2-09). Without it host passwords and keys live in memory only, and the app deliberately uses a separate
`com.mio.OpsBatch-dev` data directory so it can never strip secrets from a Tauri installation.

### In-app update

*Settings → update button* (the red dot appears when a newer GitHub release exists). The app downloads the installer for
its platform, verifies it against the release's `SHA256SUMS` (a missing entry or a mismatch aborts and deletes the file),
opens it (`.dmg` on macOS, the setup program on Windows, the folder of the AppImage on Linux) and "Restart" quits so
the installer can replace the running version. When an updater key is configured (`OPSBATCH_UPDATER_PUBKEY` at build time, the
base64 public key of the Tauri updater config, or at run time for mirrors) every installer also needs a valid minisign
signature `<installer>.sig` (base64 of the signature file, as `tauri signer sign` writes it) next to it, and a missing or
wrong signature aborts the download like the Tauri updater does. Without a key, authenticity rests on HTTPS to GitHub
plus the checksum file.

### Migrating from the Tauri build

* Same data: once `real-vault` is enabled the app reads and writes the Tauri build's database and vault
  (`com.mio.OpsBatch` application-data directory); nothing needs exporting.
* **Do not run both builds at the same time** against the same data directory (they share one SQLite file and vault).
* Settings → Data → *Export* makes a plain copy of `opsbatch.db` that either build can restore.
* Not ported (see `FEATURE_MATRIX.md` for the full list): scheduled tasks never ran in the original either; the workflow
  editor is a staged list instead of a free canvas; PDF / audio / video previews.

---

## 简体中文

OpsBatch 是面向运维的桌面工作台：SSH 终端、SFTP、批量命令、脚本/命令库、快捷指令、RDP/VNC 远程桌面、AI 助手、
工作流与全局日志。本仓库是 Tauri + React 版本（[`../OpsBatch`](../OpsBatch)）到 Rust + [GPUI](https://gpui-kit.com/docs/)
（`gpui-kit =0.8.0`）的原生移植。功能对照见 [`FEATURE_MATRIX.md`](FEATURE_MATRIX.md)，计划与各里程碑报告见
[`PLAN.md`](PLAN.md)、[`TASKS.md`](TASKS.md)。

### 构建与运行

```sh
cargo run -p opsbatch        # 调试运行，使用隔离的 “-dev” 数据目录
cargo test --workspace       # 全部 crate 与无界面 UI 测试
```

环境变量：`OPSBATCH_DATA_DIR`（自定义数据目录）、`OPSBATCH_SKIP_UPDATE_CHECK`（启动时不检查更新）、
`OPSBATCH_SKIP_REPO_UPDATE`（启动时不拉取仓库）、`OPSBATCH_UPDATE_BASE`（用 `http://host:port` 代替 GitHub 作为更新源）、
`OPSBATCH_EXIT_AFTER`（N 秒后退出，冒烟测试用）。

### 打包

`packaging/` 下的脚本输出到 `dist/`：macOS `packaging/macos/build.sh`（.app + .dmg）；Windows
`packaging\windows\build.ps1`（需要 Inno Setup 6）；Linux `packaging/linux/build.sh`（AppImage + deb）。
`scripts/checksums.sh dist` 生成 `SHA256SUMS`；推送 `v*` 标签时 `.github/workflows/release.yml` 会构建全部平台并发布。
签名与公证通过环境变量/仓库 Secret 启用（见上表），未设置时跳过并给出警告。

**正式发布必须使用 `OPSBATCH_FEATURES=real-vault`**：加密保险库/系统钥匙串是最后一个里程碑项（TASKS M1-05、M2-09）。
未启用时主机密码与密钥只保存在内存里，并且程序刻意使用独立的 `com.mio.OpsBatch-dev` 数据目录，避免破坏 Tauri 版的密钥。

### 应用内更新

更新按钮在发现新版本时显示红点。程序下载当前平台的安装包，并用发布页的 `SHA256SUMS` 校验（缺少条目或不一致会中止并删除文件），
随后打开安装包（macOS 打开 .dmg，Windows 启动安装程序，Linux 打开 AppImage 所在目录）；“重启应用”会退出当前版本，
以便安装程序替换它。配置了更新公钥（构建时或运行时设置环境变量 `OPSBATCH_UPDATER_PUBKEY`，格式同 Tauri updater 的 pubkey）后，安装包旁必须有有效的 minisign 签名 `<安装包>.sig`，缺失或不匹配会中止下载（与 Tauri updater 一致）；未配置公钥时真实性依赖 HTTPS 与校验文件。

### 从 Tauri 版迁移

* 数据相同：启用 `real-vault` 后，程序读写 Tauri 版的数据库与保险库（`com.mio.OpsBatch` 应用数据目录），无需导出。
* **不要同时运行两个版本**（共用同一个 SQLite 文件与保险库）。
* 设置 → 数据管理 → 导出会生成 `opsbatch.db` 的普通副本，两个版本都能恢复。
* 未移植内容见 `FEATURE_MATRIX.md`：定时任务在原版也从未运行；工作流编辑器是分阶段列表而非自由画布；PDF/音视频预览。
