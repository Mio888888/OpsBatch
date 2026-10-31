# Security policy / 安全政策

[English](#english) · [简体中文](#简体中文)

## English

OpsBatch stores host credentials, SSH keys and AI provider keys, and opens connections to your servers, so security
reports are taken seriously.

### Reporting a vulnerability

Please report privately; do not open a public issue.

* **GitHub:** [report a vulnerability](https://github.com/Mio888888/OpsBatch/security/advisories/new)
* **Email:** mio@866619.xyz

Include what you found, how to reproduce it (or a proof of concept), what it affects, and how to reach you if you want
to be credited.

### What to expect

1. We acknowledge your report within **48 hours**.
2. We assess the severity and plan a fix.
3. A fix or a mitigation is released within **14 days** for confirmed issues.
4. After the fix ships we publish an advisory and credit you, unless you prefer to stay anonymous.

### Supported versions

| Version | Supported |
| --- | --- |
| Latest release (`master`) | ✅ |
| Older releases, including the Tauri build on `0.x` | ❌ — update to the latest release; it reads your existing data |

### In scope

Examples: leaking or weakening the vault and keychain protection, bypassing the dangerous-command confirmation or the
MCP approval prompt, accepting an update without a valid signature or checksum, command or path injection through
host, library or repository data, and anything that sends secrets off the machine.

### Hardening tips

* Install only from the [official releases](https://github.com/Mio888888/OpsBatch/releases); the in-app updater checks
  every download against `SHA256SUMS` and the release signature.
* Keep the encrypted vault enabled (production builds use `real-vault`).
* Give AI providers and MCP clients only the access they need, and review each approval prompt.

## 简体中文

OpsBatch 保存主机凭据、SSH 密钥和 AI 服务商密钥，并连接你的服务器，因此我们认真对待安全报告。

### 报告漏洞

请私下报告，不要公开提 Issue。

* **GitHub：**[提交安全公告](https://github.com/Mio888888/OpsBatch/security/advisories/new)
* **邮箱：**mio@866619.xyz

请写明漏洞内容、复现步骤（或 PoC）、影响范围，以及（希望署名时）你的联系方式。

### 处理流程

1. 收到报告后 **48 小时**内确认。
2. 评估严重程度并制定修复计划。
3. 确认的问题在 **14 天**内提供修复或缓解方案。
4. 修复发布后公布公告并致谢（除非你希望匿名）。

### 受支持的版本

| 版本 | 是否支持 |
| --- | --- |
| 最新发布版（`master`） | ✅ |
| 旧版本，包括 `0.x` 上的 Tauri 版 | ❌ —— 请更新到最新版，它会读取你现有的数据 |

### 范围示例

泄露或削弱保险库/钥匙串保护；绕过危险命令确认或 MCP 审批；接受没有有效签名或校验和的更新；通过主机、库或仓库数据造成命令或路径注入；任何把密钥送出本机的行为。

### 加固建议

* 只从[官方 Releases](https://github.com/Mio888888/OpsBatch/releases) 安装；应用内更新会用 `SHA256SUMS` 与发布签名校验每次下载。
* 保持加密保险库开启（正式构建使用 `real-vault`）。
* 只给 AI 服务商和 MCP 客户端必要的权限，并认真看每一次审批提示。
