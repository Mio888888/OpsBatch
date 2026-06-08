# AI CommandPlan 结构化校验设计

## 目标

在第一阶段 Rust 策略评估和审计之上，增加结构化 `CommandPlan` 支持。AI 可以输出可校验 JSON 计划，OpsBatch 先由 Rust 解析、校验、归一化，再转成待审批命令并进入现有策略评估流程。

## 设计原则

- 不信任模型输出：JSON 必须通过 Rust fail-fast 校验。
- 不破坏现有体验：旧的 `[ACTION]...[/ACTION]` 和 shell 代码块继续可用。
- 小步接入：本阶段只做命令计划，不做文件 patch、MCP tool 或批量集群计划。
- 命令仍不自动执行：结构化计划通过后仍进入右侧确认栏，由第一阶段策略评估决定 `CONFIRM` 或 `BLOCK`。

## CommandPlan 格式

模型可以输出以下任一形式：

```json
{
  "version": 1,
  "summary": "查看 nginx 状态并读取最近日志",
  "steps": [
    {
      "description": "查看 nginx 服务状态",
      "command": "systemctl status nginx --no-pager",
      "intent": "diagnose",
      "expectedOutcome": "确认 nginx 是否运行"
    }
  ]
}
```

或包裹格式：

```text
<COMMAND_PLAN>
{ ...同上... }
</COMMAND_PLAN>
```

前端也支持 fenced JSON：

````markdown
```json
{ ...同上... }
```
````

## Rust 校验规则

Rust 新增 `ai_validate_command_plan` Tauri 命令：

- `version` 必须为 `1`。
- `summary` 必须非空，最长 200 字符。
- `steps` 必须为 1 到 8 步。
- 每步 `description` 必须非空，最长 120 字符。
- 每步 `command` 必须非空，最长 4000 字符，不能包含 NUL 或 BiDi 控制字符。
- 每步 `intent` 允许 `observe` / `diagnose` / `change` / `verify` / `rollback`，缺省为 `diagnose`。
- 每步 `expectedOutcome` 可选，最长 240 字符。
- 重复命令会去重，保留首次出现的步骤。

校验通过返回 `ValidatedCommandPlan`，其中每步包含稳定 id、description、command、intent、expectedOutcome。

校验失败返回错误字符串，前端不生成待审批命令，并保留原始文本作为普通回复。

## 前端接入

`parseAiPendingActions` 先提取 `CommandPlan`：

1. 寻找 `<COMMAND_PLAN>...</COMMAND_PLAN>`。
2. 寻找 fenced JSON，且 JSON 顶层含 `steps`。
3. 找到后调用 Rust `ai_validate_command_plan`。
4. 校验通过后把 steps 转成 `ParsedPendingAction`，source 标记为 `command_plan`。
5. 展示文本中移除结构化 JSON，避免用户看到大块机器格式。
6. 若没有计划或校验失败，回退现有 ACTION / shell fence 解析。

## 错误处理

- Rust 解析失败：前端不产生 action，显示模型原文。
- Rust 校验失败：前端不产生 action，保留原文，避免错误计划悄悄执行。
- Rust 调用异常：回退现有 ACTION 解析，不影响旧流程。

## 测试

Rust 单元测试覆盖：

- 有效计划能解析成两个步骤。
- 非 version 1 失败。
- 空 steps 失败。
- 包含 NUL 或 BiDi 的命令失败。
- 重复命令去重。

前端构建验证 TypeScript 类型和异步解析流程。
