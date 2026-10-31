# Contributing / 贡献指南

[English](#english) · [简体中文](#简体中文)

## English

Thanks for helping with OpsBatch. Bug reports, fixes, documentation and new features are all welcome.

### Before you start

* Search the [issues](https://github.com/Mio888888/OpsBatch/issues) first. For a bug, include the OpsBatch version, your
  OS, the remote server software (for SSH, RDP and VNC problems), steps to reproduce and the relevant lines of the
  global log window, with passwords and tokens removed.
* For a larger change, open an issue first so the approach can be agreed before you spend time on it.
* Security problems go through [SECURITY.md](SECURITY.md), not a public issue.
* By taking part you agree to the [Code of Conduct](CODE_OF_CONDUCT.md). Contributions are accepted under the
  project's license, [AGPL-3.0](../LICENSE).

### Branches

| Branch | Contents |
| --- | --- |
| `master` | The Rust + GPUI build. Keep it releasable. Open pull requests against it. |
| `0.x` | The previous Tauri + React build. Only fixes for that build go there. |

Work on a branch named `feature/<topic>` or `fix/<topic>` in your fork.

### Set up

You need a stable Rust toolchain. On Linux, install the GPUI system packages listed in
[`.github/workflows/ci.yml`](workflows/ci.yml).

```sh
git clone https://github.com/<you>/OpsBatch && cd OpsBatch
cargo run -p opsbatch        # debug build with the isolated "-dev" data directory
```

The debug build never touches the data of an installed OpsBatch. Keep it that way: point tests and experiments at a
scratch directory with `OPSBATCH_DATA_DIR`, and do not run it with `real-vault` against your real vault while
developing.

### Before you open a pull request

CI runs these three commands on macOS, Windows and Linux; run them locally first.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

* UI behaviour is tested headlessly (no display needed) in `app/tests/`; add a test next to the code you change.
  Protocol code (`ops-vnc`, `ops-rdp`, `ops-ssh`) is tested against in-process mock servers.
* All user-facing text goes through `t!()` and must exist in both `locales/zh-CN.json` and `locales/en-US.json`.
* The SQLite schema and the vault must stay readable by the Tauri build. If a change cannot, say so in the pull request.
* Keep secrets, tokens and real hostnames out of code, tests, fixtures and logs.
* Update [`FEATURE_MATRIX.md`](../FEATURE_MATRIX.md) and the READMEs when behaviour changes.

### Commits and pull requests

* Commit subject: `<area>: <what changed>`, imperative, no trailing period, about 72 characters at most. For example
  `VNC: pick the authentication type automatically`. The body explains why, not what.
* One logical change per commit; do not mix a reformat with a fix.
* The pull request description follows the template: what and why, how you tested it, screenshots for UI changes.
* Reviews are required before merging. Please answer review comments and push fixes to the same branch.

## 简体中文

感谢你为 OpsBatch 做贡献。无论是报告 Bug、修复、文档还是新功能，我们都欢迎。

### 开始之前

* 先搜索已有的 [Issue](https://github.com/Mio888888/OpsBatch/issues)。提交 Bug 时请写明 OpsBatch 版本、操作系统、
  远端服务器软件（SSH、RDP、VNC 问题）、复现步骤，以及全局日志窗口中的相关内容（去掉密码和令牌）。
* 较大的改动请先开 Issue 讨论方案，再动手。
* 安全问题请按 [SECURITY.md](SECURITY.md) 处理，不要公开提 Issue。
* 参与即表示遵守[行为准则](CODE_OF_CONDUCT.md)；贡献按项目许可证 [AGPL-3.0](../LICENSE) 接受。

### 分支

| 分支 | 内容 |
| --- | --- |
| `master` | Rust + GPUI 版本，保持可发布，PR 提到这里。 |
| `0.x` | 旧的 Tauri + React 版本，只接收该版本的修复。 |

请在你的 fork 里用 `feature/<主题>` 或 `fix/<主题>` 命名分支。

### 开发环境

需要稳定版 Rust 工具链；Linux 先安装 [`ci.yml`](workflows/ci.yml) 中列出的 GPUI 系统依赖。

```sh
git clone https://github.com/<you>/OpsBatch && cd OpsBatch
cargo run -p opsbatch        # 调试运行，使用隔离的 “-dev” 数据目录
```

调试构建不会碰已安装 OpsBatch 的数据。请保持这一点：测试和实验用 `OPSBATCH_DATA_DIR` 指向临时目录，
开发时不要用 `real-vault` 去连你真实的保险库。

### 提交 PR 之前

CI 会在 macOS、Windows、Linux 上运行下面三条命令，请先在本地跑通：

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

* 界面行为在 `app/tests/` 里做无界面测试（不需要显示器），改动旁请补测试；协议代码（`ops-vnc`、`ops-rdp`、`ops-ssh`）用进程内模拟服务器测试。
* 所有界面文案都走 `t!()`，并且要同时写入 `locales/zh-CN.json` 与 `locales/en-US.json`。
* SQLite 表结构与保险库必须保持 Tauri 版可读；做不到时请在 PR 里说明。
* 代码、测试、fixture、日志里不要出现密钥、令牌或真实主机名。
* 行为有变化时更新 [`FEATURE_MATRIX.md`](../FEATURE_MATRIX.md) 与 README。

### 提交与 PR

* 提交标题：`<模块>: <做了什么>`，祈使句，不加句号，尽量不超过 72 字符，例如 `VNC: pick the authentication type automatically`。正文写“为什么”，不写“做了什么”。
* 一次提交只做一件事，不要把格式化和修复混在一起。
* PR 描述按模板填写：改了什么与为什么、怎么测的、界面改动附截图。
* 合并前需要审查，请及时回复审查意见并把修复推到同一分支。
