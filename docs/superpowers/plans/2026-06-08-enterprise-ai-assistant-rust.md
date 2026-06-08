# 企业级应用内 AI 助手原生 Rust 治理实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划。步骤使用复选框（`- [ ]`）语法来跟踪进度。

**目标：** 为 OpsBatch 应用内 AI 助手增加原生 Rust 策略评估、HITL 阻断和审计能力。

**架构：** 现有 React AI 助手继续负责对话和命令提议。新增 Rust governance 命令在执行前评估每条 AI 命令并写审计，前端确认栏显示评估结果并阻止 BLOCK 命令执行。

**技术栈：** Tauri 2、Rust、rusqlite、regex、React、TypeScript、Zustand。

---

## 文件结构

- 修改 `src-tauri/src/commands/ai.rs`：新增 action 风险评估、审计类型、Tauri 命令和 Rust 单元测试。
- 修改 `src-tauri/src/db/mod.rs`：新增 `ai_action_audit` 表。
- 修改 `src-tauri/src/lib.rs`：注册新增 Tauri 命令。
- 修改 `src/utils/aiActionParser.ts`：扩展 `PendingAction` 评估字段。
- 修改 `src/stores/aiChat.ts`：ACTION 解析后调用 Rust 评估；审批/拒绝时写审计。
- 修改 `src/components/AiChatPanel.tsx`：确认栏展示风险，BLOCK 禁用执行。
- 修改 `src/App.css`：新增风险徽标和阻断样式。
- 修改 `src/i18n/dictionaries.ts`：补充中英文 UI 文案。

## 任务 1：Rust 策略评估与审计

**文件：**

- 修改：`src-tauri/src/commands/ai.rs`
- 修改：`src-tauri/src/db/mod.rs`
- 修改：`src-tauri/src/lib.rs`

- [ ] **步骤 1：写 Rust 单元测试**

在 `ai.rs` 中加入测试，断言 destructive、pipe-to-shell、自定义 danger rule 等场景的决策。

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test ai_action --manifest-path src-tauri/Cargo.toml`
预期：新增测试因类型或函数未定义失败。

- [ ] **步骤 3：实现评估和审计命令**

新增 `AiActionAssessment`、`AiActionAuditEvent`、`ai_assess_action`、`ai_record_action_event`。评估函数使用内置规则和 `danger_rules`。

- [ ] **步骤 4：注册命令并补表**

在 `db/mod.rs` 创建 `ai_action_audit`。在 `lib.rs` 注册新增命令。

- [ ] **步骤 5：运行 Rust 测试**

运行：`cargo test ai_action --manifest-path src-tauri/Cargo.toml`
预期：测试通过。

## 任务 2：前端 ACTION 风险接入

**文件：**

- 修改：`src/utils/aiActionParser.ts`
- 修改：`src/stores/aiChat.ts`
- 修改：`src/components/AiChatPanel.tsx`

- [ ] **步骤 1：扩展 action 类型**

给 `ParsedPendingAction` 增加 `assessment`、`assessmentLoading`、`assessmentError`。

- [ ] **步骤 2：ACTION 解析后调用 Rust 评估**

在 stream done 时先生成 actions，再异步调用 `ai_assess_action`。评估结果写回对应 action。

- [ ] **步骤 3：审批/拒绝写审计**

执行前调用 `ai_record_action_event` 记录 `approved`。拒绝时记录 `rejected`。BLOCK action 执行前记录 `blocked` 并拒绝执行。

- [ ] **步骤 4：确认栏展示风险和阻断**

`CmdPendingItem` 显示风险等级、决策、规则和原因。`BLOCK` 禁用执行按钮，保留拒绝按钮。

## 任务 3：样式、文案与验证

**文件：**

- 修改：`src/App.css`
- 修改：`src/i18n/dictionaries.ts`

- [ ] **步骤 1：新增风险样式**

新增 `.ai-cmd-risk-*`、`.ai-cmd-policy-*`、`.ai-cmd-item-blocked` 样式。

- [ ] **步骤 2：新增中英文文案**

新增风险、策略决策、阻断、评估失败相关 key。

- [ ] **步骤 3：运行验证**

运行：

```bash
npm run build
cargo test ai_action --manifest-path src-tauri/Cargo.toml
```

预期：构建和 Rust 测试均通过。
