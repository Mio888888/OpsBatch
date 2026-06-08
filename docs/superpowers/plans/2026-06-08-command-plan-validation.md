# AI CommandPlan 结构化校验实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划。步骤使用复选框（`- [ ]`）语法来跟踪进度。

**目标：** 让 OpsBatch AI 助手支持 Rust 校验的结构化 `CommandPlan`，并把通过校验的步骤接入现有审批和风险评估流程。

**架构：** Rust 提供 `ai_validate_command_plan`，负责 JSON 解析、fail-fast 校验和去重。前端先提取结构化计划，调用 Rust 校验，通过后转为 `PendingAction`，然后沿用第一阶段的 `ai_assess_action` 和确认栏。

**技术栈：** Rust、serde、serde_json、Tauri、React、TypeScript。

---

## 文件结构

- 修改 `src-tauri/src/commands/ai.rs`：新增 CommandPlan 类型、校验函数、Tauri 命令、单元测试。
- 修改 `src-tauri/src/lib.rs`：注册 `ai_validate_command_plan`。
- 修改 `src/utils/aiActionParser.ts`：新增结构化计划提取和异步解析入口。
- 修改 `src/stores/aiChat.ts`：stream done 时改用异步 action 解析。

## 任务 1：Rust CommandPlan 校验

**文件：**

- 修改：`src-tauri/src/commands/ai.rs`
- 修改：`src-tauri/src/lib.rs`

- [ ] **步骤 1：编写失败测试**

新增测试：

```rust
#[test]
fn command_plan_validation_accepts_valid_plan() {
    let raw = r#"{"version":1,"summary":"检查服务","steps":[{"description":"查看状态","command":"systemctl status nginx --no-pager","intent":"diagnose","expectedOutcome":"确认状态"}]}"#;
    let plan = validate_command_plan(raw).expect("valid plan");
    assert_eq!(1, plan.version);
    assert_eq!(1, plan.steps.len());
    assert_eq!("diagnose", plan.steps[0].intent);
}
```

运行：`cargo test command_plan --manifest-path src-tauri/Cargo.toml`
预期：因 `validate_command_plan` 未定义失败。

- [ ] **步骤 2：实现类型与校验函数**

新增 `RawCommandPlan`、`RawCommandPlanStep`、`ValidatedCommandPlan`、`ValidatedCommandPlanStep` 和 `validate_command_plan(raw: &str)`。

- [ ] **步骤 3：实现 Tauri 命令并注册**

新增：

```rust
#[tauri::command]
pub fn ai_validate_command_plan(raw_plan: String) -> Result<ValidatedCommandPlan, String> {
    validate_command_plan(&raw_plan)
}
```

并在 `src-tauri/src/lib.rs` 注册。

- [ ] **步骤 4：运行 Rust 测试**

运行：`cargo test command_plan --manifest-path src-tauri/Cargo.toml`
预期：CommandPlan 测试通过。

## 任务 2：前端结构化计划解析

**文件：**

- 修改：`src/utils/aiActionParser.ts`
- 修改：`src/stores/aiChat.ts`

- [ ] **步骤 1：扩展 action 来源字段**

给 `ParsedPendingAction` 增加：

```ts
source?: 'action_block' | 'fence' | 'command_plan';
intent?: string;
expectedOutcome?: string;
```

- [ ] **步骤 2：新增异步解析函数**

新增 `parseAiPendingActionsAsync(content)`：

- 提取 `<COMMAND_PLAN>` 或 fenced JSON。
- 调用 `invoke<ValidatedCommandPlan>('ai_validate_command_plan', { rawPlan })`。
- 成功时返回 plan steps 生成的 actions。
- 失败或无计划时调用现有 `parseAiPendingActions(content)`。

- [ ] **步骤 3：store 改用异步解析**

`initStreamListener` 的 `chunk.done` 分支改为先设置 streaming false，然后异步解析 actions；解析完成后写回 message 的 `content` 和 `pendingActions`，再进入评估。

## 任务 3：验证

**文件：**

- 修改：无新增文件。

- [ ] **步骤 1：运行前端构建**

运行：`npm run build`
预期：TypeScript 和 Vite build 通过。

- [ ] **步骤 2：运行 Rust 测试**

运行：`cargo test --lib --manifest-path src-tauri/Cargo.toml`
预期：全部 Rust lib 测试通过。
