# OpsBatch

[简体中文](README_CN.md) | [English](README_EN.md)

OpsBatch 是一个面向运维场景的本地桌面工作台，用于集中管理服务器资产、SSH 终端、批量命令、脚本库、文件传输、自动化工作流与运维知识同步。项目当前版本为 `0.1.1`，桌面端基于 Tauri v2、React 19、TypeScript/Vite 与 Rust 后端构建，核心数据持久化在本机 SQLite 数据库中。


## 产品定位

OpsBatch 旨在为需要频繁连接、检查和批量操作多台服务器的人员提供一个无需部署中心服务的桌面客户端。

适用人群包括：

- 运维工程师、SRE、应用运维人员
- DBA、数据库运维人员
- DevOps / 平台工程师
- 安全运维、应急响应和值班人员
- 需要维护命令、脚本、主机资产和执行记录的团队

核心目标：

- 在本地统一管理主机资产、分组、SSH 连接和跳板机链路。
- 将常用命令、脚本、快捷动作和工作流沉淀为可复用资产。
- 支持多主机批量执行、文件分发、交互式广播终端和执行历史追踪。
- 通过本地数据库保存配置和操作数据，减少对中心服务的依赖。

## 功能概览

### 资产与连接管理

- 主机资产管理：支持主机名称、IP、端口、认证方式、操作系统、分组、标签字段和跳板机链路等信息。
- 分组与选择：支持嵌套分组、资产搜索、主机选择和基于所选主机发起终端/命令/传输操作。
- 导入导出：支持主机 CSV 导入导出。
- 云主机导入：支持阿里云、AWS、腾讯云实例拉取与导入。
- SSH 配置/跳板机：支持解析 `~/.ssh/config` 中的主机与 `ProxyJump` 信息，并构建跳板链路。

### 终端、监控与远程文件

- 终端工作区：支持本地终端和远程 SSH 终端标签页。
- 主机监控：远程连接后可查看 CPU、内存、磁盘、网络、进程等快照信息；当前采集逻辑主要面向 Linux/procfs 环境。
- SFTP 文件面板：支持本地/远程双栏浏览、上传、下载、预览、重命名、删除、新建目录、书签、解压和传输队列。
- 远程编辑器：可从 SFTP 面板打开远程文件/目录，使用 CodeMirror 编辑并通过 SFTP 保存。
- 端口转发：终端底部面板包含端口转发入口。

### 批量执行与文件分发

- 批量命令执行：对选中主机执行命令，支持并发数、超时、实时结果、失败主机重试和历史记录。
- 危险命令防护：内置和自定义危险命令规则，在执行高风险命令前触发确认；也可结合 AI 做风险说明。
- 执行回放：批量执行后端支持记录历史与 asciinema 形式的终端回放数据。
- 广播终端：可打开独立批量终端窗口，对多台已连接主机广播输入；当前窗口最多处理 16 台选中主机。
- 批量文件传输：支持选择本地文件或目录上传到多台主机，远程路径支持 `{host}` 与 `{firstdir:path}` 变量。当前 UI 主要面向批量上传。

### 命令库、脚本库与快捷动作

- 命令库：支持搜索、分类筛选、自定义命令、风险等级、平台字段、远程 URL 命令、复制和加入快捷动作。
- 脚本库：支持语言/分类/搜索筛选，自定义脚本，查看、编辑、版本历史、恢复版本和加入快捷动作。
- 快捷动作：支持 CRUD、分类/星标/语言筛选、JSON 导入导出、参数占位符提示，并可对选中主机执行。
- Git 仓库同步：支持配置仓库地址、分支、Token、拉取策略、启用状态、手动同步和定时检查，将仓库中的命令/脚本/快捷动作导入本地库。

### 工作流编排

- 可视化工作流：基于 React Flow 的工作流列表、编辑器、模板和执行入口。
- 节点类型：包含开始/结束、选择主机、命令、脚本、快捷动作、传输、条件、分支、延迟、人工确认和回滚等节点。
- 工作流执行：前端执行器按节点依赖运行，支持结果变量、条件/分支路径和人工确认。
- 定时任务：后端保存工作流定时任务；当前调度解析以简单 `every:N` 间隔为主，不是完整 Cron 表达式实现。

### AI、日志与设置

- AI 配置：支持 OpenAI、本地 Ollama 和自定义 OpenAI-compatible 服务配置。
- AI 能力：包含聊天、流式聊天、脚本生成、命令/结果分析、错误诊断和风险评估等后端能力。
- 终端 AI 面板：可从终端上下文发起 AI 对话、生成命令并在用户确认后执行。
- 全局日志：后端日志和前端 console/window error 会进入全局日志窗口并持久化。
- 设置：包含通用、外观、快捷动作、命令库、脚本库、AI、危险规则和数据备份等页面。
- 国际化：应用内支持 `zh-CN` 与 `en-US`，可跟随系统语言。

## 架构与技术栈

### 高层数据流

```text
React/Vite UI
  -> Zustand stores
  -> Tauri invoke() commands and Tauri events
  -> Rust command modules
  -> SQLite / SSH / SFTP / local PTY / Git / HTTP / OS keychain
```

### 技术栈

| 层级 | 技术 |
| --- | --- |
| 桌面容器 | Tauri v2 |
| 前端构建 | Vite 7、TypeScript 5.8 |
| UI 框架 | React 19、React Router 7 |
| 状态管理 | Zustand 5 |
| UI 基础 | Radix UI primitives、项目自有 `src/components/ui` 封装、`App.css` 变量/样式 |
| 图标 | `lucide-react` |
| 终端 | xterm.js |
| 编辑器 | CodeMirror / `@uiw/react-codemirror` |
| 工作流画布 | React Flow / `@xyflow/react` |
| 拖拽/动效 | dnd-kit、GSAP |
| 后端语言 | Rust |
| 后端能力 | Tauri commands、SQLite (`rusqlite`)、SSH/SFTP (`russh`, `russh-sftp`)、本地 PTY、Git (`git2`)、HTTP (`reqwest`) |
| 本地数据 | Tauri app data 目录下的 SQLite 数据库 `opsbatch.db` |

## 仓库结构

```text
.
├── README.md                 # 中文 README（默认入口）
├── README_CN.md              # 中文 README 副本/命名版本
├── README_EN.md              # English README
├── package.json              # 桌面/Web 前端与 Tauri CLI 脚本
├── vite.config.ts            # Vite 构建配置
├── tsconfig.json             # TypeScript 配置
├── src/                      # React 桌面端前端源码
│   ├── components/           # 布局、SFTP、AI、UI 封装等组件
│   ├── pages/                # Terminal、Commands、Libraries、Workflow、Settings 等页面
│   ├── stores/               # Zustand 领域状态与 Tauri invoke 封装
│   ├── types/                # 前端领域类型
│   └── i18n/                 # zh-CN / en-US 字典与语言解析
├── src-tauri/                # Tauri v2 Rust 后端
│   ├── src/commands/         # hosts、terminal、execution、sftp、workflow、ai 等命令模块
│   ├── src/db/               # SQLite schema 与迁移
│   ├── Cargo.toml            # Rust 依赖
│   ├── tauri.conf.json       # Tauri 应用与打包配置
│   └── tauri.windows.conf.json # Windows 签名扩展配置
├── tests/                    # Node 测试文件
├── scripts/                  # 发布和签名辅助脚本
├── website/                  # 独立静态官网与下载页
├── docs/superpowers/         # 项目规格、计划和协作资料
└── .github/workflows/        # 构建与 GitHub Pages 发布工作流
```

## 环境要求

建议准备以下环境：

- Node.js 与 npm（项目使用 npm 脚本）。
- Rust toolchain（`cargo`、`rustc`）。
- Tauri v2 所需的系统依赖。不同平台要求不同，请参考 [Tauri 官方 prerequisites](https://v2.tauri.app/start/prerequisites/)。
- Git（用于仓库同步功能和常规开发）。
- 可访问的 SSH 主机、云厂商凭据或 AI 服务凭据是可选的运行时配置，不是启动开发环境的必要条件。

## 快速开始

### 1. 安装依赖

```bash
npm install
```

### 2. 启动桌面端开发模式

```bash
npm run tauri -- dev
```

该命令会按照 `src-tauri/tauri.conf.json` 中的配置先运行 Vite 开发服务，再启动 Tauri 桌面窗口。

如果只需要启动前端 Vite 开发服务：

```bash
npm run dev
```

## 常用命令

| 命令 | 说明 |
| --- | --- |
| `npm run dev` | 启动桌面前端 Vite 开发服务 |
| `npm run tauri -- dev` | 启动 Tauri 桌面开发模式 |
| `npm run build` | 执行 `tsc && vite build`，构建桌面前端产物 |
| `npm run tauri -- build` | 构建 Tauri 桌面应用/安装包 |
| `npm run preview` | 预览 Vite 构建产物 |
| `npm run build:all` | 桌面前端构建别名，等价于 `npm run build` |
| `cargo check --manifest-path src-tauri/Cargo.toml` | 检查 Rust/Tauri 后端类型与编译问题 |
| `npx tsc --noEmit` | 只运行 TypeScript 类型检查 |

> 当前仓库没有单独的 `lint` npm 脚本；前端验证主要依赖 TypeScript 检查和 Vite 构建，后端验证可运行 `cargo check`。

## Windows 发布与 SmartScreen

Windows 安装包如果未进行 Authenticode 代码签名，或签名证书/发布源还没有积累 Microsoft 信誉，用户安装时可能看到“Windows 已保护你的电脑 / Microsoft Defender SmartScreen 阻止了无法识别的应用启动”。这不是应用运行时代码能直接关闭的提示，正式发布应使用可信代码签名证书签署安装包与可执行文件。

仓库包含 Windows 专用 Tauri 配置 `src-tauri/tauri.windows.conf.json`。在 Windows 上运行 `npm run tauri -- build` 时，Tauri 会自动调用 `scripts/windows-sign.ps1` 对生成的 Windows 可执行文件和安装包进行签名；如果没有配置证书环境变量，脚本会跳过 Authenticode 签名并输出 warning，构建继续产出未签名发布件。

推荐发布流程：

1. 准备 OV 或 EV 代码签名证书。EV 证书通常更容易通过 SmartScreen 信誉检查；OV 证书也需要逐步积累下载和安装信誉。
2. 在 Windows 构建机安装 Windows SDK，确保 `signtool.exe` 可用。
3. 通过环境变量提供证书，不要把证书文件或密码提交到仓库：

```powershell
$env:WINDOWS_CODESIGN_CERT_PATH = 'C:\secure\OpsBatch.pfx'
$env:WINDOWS_CODESIGN_CERT_PASSWORD = '<pfx-password>'
npm run tauri -- build
```

如果证书已安装到 Windows 证书存储，也可以使用指纹：

```powershell
$env:WINDOWS_CODESIGN_CERT_THUMBPRINT = '<certificate-sha1-thumbprint>'
npm run tauri -- build
```

需要对已有产物补签时，也可以直接运行：

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\windows-sign.ps1 .\src-tauri\target\release\bundle
```

签名后可用 `signtool verify /pa /v <installer.exe>` 验证签名。即使已经签名，新证书或低分发量应用仍可能短期触发 SmartScreen，持续使用同一发布者证书、稳定版本号和官方分发渠道有助于建立信誉。

## 本地数据与安全说明

OpsBatch 的核心设计是本地桌面客户端，数据主要保存在本机：

- Tauri 启动时会在系统 app data 目录下打开/创建 SQLite 数据库 `opsbatch.db`。
- 数据库包含主机、分组、标签、执行历史、命令库、脚本库、快捷动作、Git 仓库配置、危险规则、通用设置、工作流、AI 会话、RAG/MCP 表和全局日志等表。
- 设置页面提供数据库备份功能，会将当前 `opsbatch.db` 复制到用户选择的位置。

请注意以下安全现状，避免过度假设：

- 主机密码、私钥字段和 Git 仓库 Token 会优先写入系统 keychain，SQLite 中保存 `***keychain***` 占位符；历史数据启动迁移时也会尽量转入 keychain。
- AI API Key 有尽力写入系统 keychain 的逻辑，但当前配置保存路径仍会把真实 Key 写入本地 settings JSON；不能简单宣称所有敏感信息都已经完整加密保护。
- 云厂商凭据和部分运行时配置仍可能保存在本地数据库或配置中，使用前请结合设备可信度和权限范围评估风险。
- SSH 主机密钥采用首次信任（TOFU）方式记录指纹；后续指纹不匹配会阻止连接并提示重新确认主机。首次连接前仍应确认目标网络和主机可信。
- Tauri 已配置基础 CSP，限制脚本、对象和 frame 加载，并允许应用所需的 IPC、HTTP(S)、本地开发服务和媒体资源；生产环境如新增资源来源，应同步评估并收紧策略。
- 建议不要在不可信设备上保存高权限凭据；对团队环境，建议配合最小权限账号、临时凭据、跳板机审计和系统磁盘加密使用。

## 合规与法律声明

OpsBatch 面向授权运维场景，提供 SSH、SFTP、批量命令、文件分发、工作流和 AI 辅助等能力。工具本身不会替用户取得授权，也不会替代组织内部审批、审计、变更管理和安全制度。

### 网络合规与授权访问

- 仅应在自己拥有、管理或已获得明确授权的服务器、账号、网络、云资源和数据范围内使用 OpsBatch。
- 批量命令、脚本、文件分发、端口转发和工作流可能造成配置变更、数据移动、权限变化或服务中断；执行前应确认目标范围、命令影响、审批要求和回滚方案。
- 用户应遵守所在组织政策、服务商条款、账号权限边界以及适用法律法规，不得将 OpsBatch 用于未授权访问、绕过安全控制、破坏服务或处理无权处理的数据。

### 生成式 AI 服务说明

- AI 功能是可选辅助能力，可配置 OpenAI、本地 Ollama 或 OpenAI-compatible 服务；是否启用、使用哪个服务、发送哪些上下文由用户自行决定。
- 当使用第三方 AI 服务时，终端输出、命令片段、错误信息、脚本内容或其他上下文可能会被发送给对应服务商处理；请在发送前移除密钥、Token、个人信息、客户数据和其他敏感内容。
- AI 生成的命令、脚本、解释、诊断和风险评估仅供参考，不保证正确、完整或适用于你的环境。所有生成内容必须经用户审查、测试和确认后再执行。

### 数据安全与隐私边界

- OpsBatch 采用本地优先设计，主机资产、执行历史、命令库、脚本库、工作流、AI 会话和日志等主要保存在本机 SQLite 数据库或本地配置中。
- 主机密码、私钥字段和 Git 仓库 Token 会优先写入系统 keychain，但部分 AI 配置、云厂商凭据、历史数据或运行时配置仍可能保存在本地数据库或配置文件中。
- 使用前请结合设备可信度、账号权限、磁盘加密、系统登录保护、备份位置和团队审计要求评估风险；不建议在不可信设备上保存高权限凭据。

### 许可协议与第三方服务

- OpsBatch 及其依赖遵循仓库声明的开源许可和第三方组件许可；分发、修改或集成时应同时遵守相关许可证要求。
- 通过 OpsBatch 连接或调用的 SSH 主机、云平台、Git 仓库、AI 服务、API 服务和网络资源，仍受对应服务条款、组织制度、地区法规和账号授权约束。
- 用户应自行负责第三方服务账号、API Key、访问令牌、模型输出和数据处理方式的合规性。

### 非法律与专业建议

README、官网、应用界面、风险提示和 AI 输出均不构成法律、合规、安全审计或专业运维建议。不同组织、行业和地区要求可能不同；涉及生产变更、个人信息、敏感数据、监管行业或跨境处理时，请咨询具备资质的专业人员，并遵循所在组织的正式流程。

## 当前状态与已知限制

- README 语言映射：`README.md` 与 `README_CN.md` 均为中文完整文档，`README_EN.md` 为英文文档；后续维护时请保持中文两份同步。
- 工作流定时：定时任务当前以简单 `every:N` 间隔解析为主，尚不是完整 Cron 表达式调度器。
- RAG/MCP：后端已存在 RAG collection/import/search 与 MCP server/tool 命令及数据表，但当前桌面路由中没有独立的顶层 RAG 或 MCP 页面。
- 监控采集：主机监控命令主要面向 Linux/procfs 场景，其他系统的指标完整性可能不同。
- 批量终端：广播终端窗口当前限制最多 16 台选中主机。
- 批量传输：当前 UI 主要呈现批量上传；传输并发输入尚未完整传递到后端请求。
- Git 仓库同步：当前实现使用语言相关元数据/文件约定（如 `library_cn.json`、`library_en.json` 和对应后缀），请按当前实现准备仓库内容。

## 开发与贡献说明

- 修改功能前建议先阅读相关源码、`docs/superpowers/` 下的规格/计划资料，以及对应页面、store 和 Rust command 的实现。
- 前端类型集中在 `src/types/index.ts`，页面位于 `src/pages/`，共享组件位于 `src/components/`，状态与后端调用封装位于 `src/stores/`。
- 前端通过 Zustand store 调用 Tauri `invoke()`，后端命令通常使用 snake_case 字段，前端 store 负责转换为 camelCase 类型。
- UI 优先使用项目自有 `src/components/ui` 封装和 `App.css` 变量/类；应用内文案已支持中文和英文语言模式。
- 新增持久化后端能力时，应同步考虑 SQLite schema/迁移、Tauri command 注册、前端类型、store 调用和验证命令。
- 官网位于 `website/`，通过 `.github/workflows/website.yml` 发布到 GitHub Pages；桌面端安装包由 `.github/workflows/build.yml` 在版本号标签不存在时构建并发布。
- 文档或代码变更后可按影响范围运行：

```bash
npm run build
cargo check --manifest-path src-tauri/Cargo.toml
```

文档类改动通常不需要运行完整构建，但 README 中的命令和功能描述应与仓库实际实现保持一致。

## 相关文档

- [README_CN.md](README_CN.md) — 中文 README。
- [README_EN.md](README_EN.md) — English README.
- [website/README.md](website/README.md) — 静态官网预览与发布说明。
