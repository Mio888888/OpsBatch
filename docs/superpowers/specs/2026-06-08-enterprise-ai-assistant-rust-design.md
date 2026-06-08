# 企业级应用内 AI 助手原生 Rust 治理设计

## 目标

参考 LinuxAgent 的安全控制面，把 OpsBatch 现有应用内 AI 助手从“前端解析命令并人工点击执行”升级为“Rust 原生策略评估、HITL 审批、阻断与审计”的企业级运维助手第一阶段。

## 参考原则

LinuxAgent README 强调：模型不可信、命令先结构化计划再策略判断、有副作用动作必须人机确认、破坏性动作永不静默放行、批量操作必须显式确认、审批和执行必须 append-only 审计。

OpsBatch 当前已有：

- React AI 聊天和右侧命令确认栏
- 前端 ACTION 块解析
- Tauri/Rust OpenAI 兼容流式调用
- SQLite 会话持久化
- danger_rules 设置表
- 终端执行与输出自动跟进

第一阶段不重写聊天模型，也不引入 LangGraph。先在原生 Rust 后端补上确定性治理入口，让现有 UI 的每条 AI 命令进入执行前都经过 Rust 策略和审计。

## 架构

新增 Rust AI action governance 模块，职责是：

1. 对命令做确定性安全评估，输出 `SAFE` / `CONFIRM` / `BLOCK`、风险等级、风险分数、命中规则和原因。
2. 读取内置规则和 `danger_rules` 自定义规则。
3. 将 AI 命令的 `proposed`、`approved`、`rejected`、`blocked` 事件写入 SQLite 审计表。
4. 暴露 Tauri 命令给前端调用。

前端职责是：

1. ACTION 解析后，为每条待审批命令调用 Rust 评估。
2. 确认栏展示风险等级、决策、命中规则和原因。
3. `BLOCK` 命令禁用执行按钮，仅允许拒绝。
4. 点击执行或拒绝时调用 Rust 写审计，再保持现有终端执行流程。

## 范围

本阶段包含：

- Rust 确定性策略评估
- SQLite 审计表
- Tauri 命令注册
- 前端待审批命令风险展示
- 阻断命令禁止执行
- 单元测试与构建验证

本阶段不包含：

- LLM JSON `CommandPlan` 强制输出
- 文件修改 unified diff 和事务化写入
- 审计 hash-chain JSONL
- MCP tool 细粒度策略
- 对话级 SAFE 免确认
- 集群批量确认
- 后端直接执行终端命令

这些作为后续阶段在同一治理边界上扩展。

## 数据模型

新增表 `ai_action_audit`：

- `id`: UUID
- `conversation_id`: 可为空
- `session_id`: 可为空
- `action_id`: 前端 ACTION id
- `event`: `proposed` / `approved` / `rejected` / `blocked`
- `command`: 原始命令
- `decision`: `SAFE` / `CONFIRM` / `BLOCK`
- `risk_level`: `low` / `medium` / `high` / `critical`
- `risk_score`: 0 到 100
- `matched_rule`: 命中规则
- `reason`: 审计原因
- `host`: 当前主机上下文
- `created_at`: 本地时间

## 策略

内置规则使用 Rust 正则与 token 检查：

- 输入为空、过长、包含 NUL 或 BiDi 控制字符：`BLOCK`
- `rm -rf /`、`mkfs`、`dd of=/dev`、fork bomb、`shutdown`、`reboot`：`BLOCK`
- 命令替换、管道到 shell、远程脚本直接执行：`BLOCK`
- `systemctl stop/restart/disable`、`service stop`、`kill -9`、包管理安装/卸载、`chmod -R`、`chown -R`：`CONFIRM`
- 首次 LLM 命令默认至少 `CONFIRM`
- 自定义 `danger_rules` 命中时升级为 `BLOCK`

## 错误处理

- 评估失败时前端保守处理为不可执行，并显示错误原因。
- 审计写入失败时执行拒绝继续，避免不可追溯的命令落地。
- 自定义规则正则无效时跳过该规则，并保留内置规则结果。

## 测试

Rust 单元测试覆盖：

- 只读命令评估为 `CONFIRM`，因为 LLM 首次命令必须确认。
- 破坏性命令评估为 `BLOCK`。
- 远程脚本管道到 shell 评估为 `BLOCK`。
- 服务重启评估为 `CONFIRM` 且风险较高。
- 自定义 danger rule 命中后升级为 `BLOCK`。

前端通过 TypeScript 构建验证类型接入。
