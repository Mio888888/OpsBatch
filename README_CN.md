# OpsBatch

[English](README.md) | **简体中文**

[![CI](https://github.com/Mio888888/OpsBatch/actions/workflows/ci.yml/badge.svg)](https://github.com/Mio888888/OpsBatch/actions/workflows/ci.yml)
[![License: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](LICENSE)

OpsBatch 是面向运维场景的本地桌面工作台：SSH 终端、SFTP、批量命令、脚本/命令库、快捷动作、RDP/VNC 远程桌面、
AI 助手、工作流与全局日志集中在一个应用里。数据保存在本机的 SQLite 数据库与加密保险库中，无需部署任何中心服务。

本仓库是基于 [GPUI](https://gpui-kit.com/docs/)（`gpui-kit =0.8.0`）的 Rust 原生版本，取代 Tauri + React 版本
（保留在 [`0.x`](https://github.com/Mio888888/OpsBatch/tree/0.x) 分支），并读写同一份数据。

## 功能

| 模块 | 内容 |
| --- | --- |
| 资产 | 主机分组、标签、搜索、CSV 导入导出、云主机导入（阿里云、AWS、腾讯云）、`~/.ssh/config` 与 `ProxyJump` 链路、SOCKS5 / HTTP 代理 |
| 终端 | 本地与 SSH 终端标签页、命令跟踪、危险命令拦截、终端内搜索、端口转发、主机监控（CPU、内存、磁盘、网络、进程） |
| SFTP 与编辑器 | 本地/远程双栏浏览、传输队列、预览、书签、带语法高亮的远程文件编辑 |
| 批量执行 | 对多台主机并发执行，支持并发数、超时、重试与历史；asciinema 风格回放；广播终端；带 `{host}` 变量的批量上传 |
| 库 | 命令库、带版本历史的脚本库、快捷动作，三者均可通过 Git 仓库同步 |
| 工作流 | 可视化画布，含主机、命令、脚本、传输、条件、分支、延迟与人工确认等节点 |
| 远程桌面 | 基于 [IronRDP](https://github.com/Devolutions/IronRDP) 的 RDP（H.264 与位图两条显示路径、剪贴板、音频、磁盘映射）；原生 RFB 客户端用于 VNC（无认证 / VNC / Apple ARD 认证，Raw、CopyRect、Hextile、ZRLE 编码） |
| AI | OpenAI 兼容对话（带主机上下文）、命令风险说明、RAG 知识库、服务商/智能体/安全设置，以及让外部智能体在逐次审批下操作 OpsBatch 的 MCP 桥 |
| 其他 | 中文 / English 界面、跟随系统外观、14 套终端主题、界面缩放、托盘图标、全局日志窗口、应用内更新 |

与 Tauri 版逐项的功能对照及已知差异见 [`FEATURE_MATRIX.md`](FEATURE_MATRIX.md)。

## 安装

在 [Releases](https://github.com/Mio888888/OpsBatch/releases) 下载对应平台的安装包：

| 平台 | 文件 |
| --- | --- |
| macOS（Apple 芯片、Intel） | `OpsBatch-<version>-macos-<aarch64\|x86_64>.dmg` |
| Windows（x64） | `OpsBatch-<version>-windows-x86_64-setup.exe` |
| Linux（x64） | `OpsBatch-<version>-linux-x86_64.AppImage`，或 `opsbatch_<version>_amd64.deb` |

没有用 Apple Developer ID 签名的构建下载后会被 Gatekeeper 拦截，执行一次
`xattr -cr /Applications/OpsBatch.app` 清除隔离标记即可。

### 从 Tauri 版迁移

* **数据相同**：程序读写 Tauri 版的数据库与保险库（`com.mio.OpsBatch` 应用数据目录），无需导出。
* **不要同时运行两个版本**，它们共用同一个 SQLite 文件与保险库。
* **更新**：发布时会附带用同一更新密钥签名的 `latest.json`，已安装的 Tauri 版会像收到普通版本一样收到更新。
* *设置 → 数据管理 → 导出*会生成 `opsbatch.db` 的普通副本，两个版本都能恢复。

### 应用内更新

发现新版本时，顶栏的更新按钮会显示红点。程序下载当前平台的安装包，并用发布页的 `SHA256SUMS` 校验（缺少条目或不一致会中止并删除文件），
然后打开安装包：macOS 打开 `.dmg`，Windows 启动安装程序，Linux 打开 AppImage 所在目录。点“重启应用”会退出当前版本，
以便安装程序替换它。

构建时内置了更新公钥（`OPSBATCH_UPDATER_PUBKEY`，格式同 Tauri updater 的 pubkey）后，安装包旁还必须有有效的 minisign
签名 `<安装包>.sig`，缺失或不匹配都会中止下载，与 Tauri updater 一致；未配置公钥时，真实性依赖 HTTPS 与校验文件。

## 从源码构建

需要稳定版 Rust 工具链。Linux 需先安装 GPUI 的系统依赖（列表见 [`.github/workflows/ci.yml`](.github/workflows/ci.yml)）。

```sh
cargo run -p opsbatch        # 调试运行，使用隔离的 “-dev” 数据目录
cargo test --workspace       # 全部 crate 与无界面 UI 测试（不需要显示器）
cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings   # CI 的检查项
```

未启用 `real-vault` 特性时，主机密码与密钥只保存在内存里，并且程序刻意使用独立的 `com.mio.OpsBatch-dev` 数据目录，
避免破坏 Tauri 版的密钥。**正式构建必须使用 `OPSBATCH_FEATURES=real-vault`**，它把加密保险库的密钥放进系统钥匙串
（macOS 钥匙串、Windows 凭据管理器、Linux Secret Service）。

| 环境变量 | 作用 |
| --- | --- |
| `OPSBATCH_FEATURES` | 打包脚本使用的 Cargo 特性（正式构建用 `real-vault`）。 |
| `OPSBATCH_DATA_DIR` | 使用该目录代替平台数据目录（测试、并行配置）。 |
| `OPSBATCH_SKIP_UPDATE_CHECK` | 启动时不检查新版本。 |
| `OPSBATCH_SKIP_REPO_UPDATE` | 启动时不拉取设置了“启动时更新”的库仓库。 |
| `OPSBATCH_UPDATE_BASE` | 用 `http://host:port` 代替 GitHub 作为更新源（URL 布局相同，用于测试与镜像）。 |
| `OPSBATCH_UPDATER_PUBKEY` | 更新公钥，构建时或运行时设置（见上文）。 |
| `OPSBATCH_EXIT_AFTER` | N 秒后退出（冒烟测试用）。 |

### 打包

[`packaging/`](packaging) 下的脚本输出到 `dist/`：

| 平台 | 命令 | 产物 |
| --- | --- | --- |
| macOS | `packaging/macos/build.sh` | `OpsBatch.app`、`OpsBatch-<version>-macos-<arch>.dmg` |
| Windows | `powershell -File packaging\windows\build.ps1`（需要 Inno Setup 6） | `OpsBatch-<version>-windows-<arch>-setup.exe` |
| Linux | `packaging/linux/build.sh`（需要 `appimagetool`、`dpkg-deb`） | `OpsBatch-<version>-linux-<arch>.AppImage`、`opsbatch_<version>_<arch>.deb` |

签名需要主动开启，未设置相应变量时跳过并给出警告：`MACOS_SIGN_IDENTITY`（再加 `APPLE_ID`、`APPLE_TEAM_ID`、
`APPLE_APP_PASSWORD` 即可公证），以及 `WINDOWS_CODESIGN_CERT_PATH` 配合 `WINDOWS_CODESIGN_CERT_PASSWORD` 或
`WINDOWS_CODESIGN_CERT_THUMBPRINT`。

## 发布

[`.github/workflows/release.yml`](.github/workflows/release.yml) 会用真实保险库构建 macOS（Apple 芯片与 Intel）、
Windows 和 Linux 的安装包，并用更新密钥签名。

* 推送到 `master` 只构建，安装包保存为工作流产物。
* **正式发布**：先把 `Cargo.toml` 里的 `version` 调高（已安装的 Tauri 版只会提示更高的版本），再推送标签
  `v<version>`，或手动运行工作流并勾选 *publish*。标签必须与版本号一致。
* 一次发布包含两种更新器所需的全部内容：安装包、本版使用的 `SHA256SUMS` 与 `.sig` 文件，以及给 Tauri 更新器用的
  `latest.json`（指向 `.app.tar.gz`、安装程序和 AppImage）。发布先建为草稿，最后才转正，已发布的版本不会被覆盖。

仓库 Secrets：`TAURI_UPDATER_PUBKEY`、`TAURI_UPDATER_PRIVATE_KEY`、`TAURI_UPDATER_PRIVATE_KEY_PASSWORD`（更新密钥，
缺少时发布会失败），以及可选的 `MACOS_CERT_P12`、`MACOS_CERT_PASSWORD`、`MACOS_SIGN_IDENTITY`、`APPLE_ID`、
`APPLE_TEAM_ID`、`APPLE_APP_PASSWORD`、`WINDOWS_CODESIGN_CERT_BASE64`、`WINDOWS_CODESIGN_CERT_PASSWORD`。

## 仓库结构

| 路径 | 内容 |
| --- | --- |
| `app/` | GPUI 应用：窗口、视图、store 与无界面 UI 测试 |
| `crates/ops-core` | 数据模型、SQLite 数据库、保险库与钥匙串、设置、事件总线、更新检查 |
| `crates/ops-ssh` | SSH 连接池、跳板机、终端通道、SFTP、端口转发、批量执行与传输 |
| `crates/ops-terminal` | 不含界面的终端逻辑：命令跟踪、危险命令拦截、搜索 |
| `crates/ops-rdp`、`crates/ops-vnc` | 基于 IronRDP 的 RDP 客户端；原生 RFB 客户端 |
| `crates/ops-ai` | AI 客户端、动作解析、风险评估、RAG、MCP、偏好设置 |
| `crates/ops-library` | 命令/脚本/快捷动作库、仓库同步、云主机导入 |
| `crates/ops-workflow` | 工作流模型、存储、执行器、调度 |
| `crates/ops-i18n`、`locales/` | 中英文词典与 `t!()` |
| `packaging/`、`scripts/` | 安装包脚本，以及校验和、签名、`latest.json` 辅助脚本 |
| `vendor/`、`spikes/` | 打过补丁的第三方 crate；早期原型 |

计划与各里程碑报告见 [`PLAN.md`](PLAN.md)、[`TASKS.md`](TASKS.md)。

## 许可证

Copyright © 2025-2026 Mio。采用 [GNU Affero 通用公共许可证第 3 版](LICENSE)（`AGPL-3.0-only`）：修改 OpsBatch
后作为网络服务运行或分发时，必须以同一许可证向用户提供你那个版本的对应源代码。
