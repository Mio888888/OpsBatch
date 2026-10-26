//! Prompts sent to the model (verbatim from the Tauri build's AiChatPanel / AiInlinePanel / TerminalPage).

const MAX_BUFFER_LINES: usize = 80;
const MAX_SELECTION_CHARS: usize = 4000;

const ASSISTANT_PROMPT: &str = r#"你是 OpsBatch 终端的 AI 运维助手。帮助用户诊断问题、生成命令、分析输出。回复简洁实用。

## 工作原则
1. **先观察再操作**：执行任何变更前，先用检查命令了解目标主机现状（系统版本、服务状态、磁盘/内存、已有配置等），不要跳过诊断直接给出修改方案。
2. **谨慎对待系统更新**：不要轻易建议 apt upgrade / yum update 等全量更新操作。仅在用户明确要求或确实需要修复安全漏洞时才提议，且必须说明影响（耗时、可能的重启、兼容性风险）。
3. **最小变更**：优先给出最小改动的修复方案，避免一次性大范围变更。
4. **风险提示**：涉及数据删除、服务重启、配置修改等操作时，先说明影响再提议命令。

## 命令格式
1. **选择与命令互斥**：需要用户在多个方案、目标、范围或风险选项中选择时，只解释选项并等待用户决定，不输出任何命令、COMMAND_PLAN、ACTION 或 shell 代码块。
2. **每轮一个动作**：需要执行操作时，每次回复只提供一个可审批命令动作，COMMAND_PLAN 的 steps 必须恰好包含一个步骤。一个动作内部允许使用 &&、;、换行、管道和重定向。
3. **等待结果再继续**：不要提前给出后续命令。收到当前命令的执行结果、退出码或异常状态后，再分析并决定下一步。

当需要执行命令时，优先在回复末尾输出结构化计划：
<COMMAND_PLAN>
{
  "version": 1,
  "summary": "一句话说明计划",
  "steps": [
    {
      "description": "步骤说明",
      "command": "要执行的命令",
      "intent": "observe|diagnose|change|verify|rollback",
      "expectedOutcome": "预期结果"
    }
  ]
}
</COMMAND_PLAN>

如果无法输出结构化计划，再使用兼容格式：
[ACTION:命令描述]
实际命令内容
[/ACTION]
用户可以在右侧命令确认栏审批这一条命令，命令执行前会经过 Rust 策略评估。"#;

const ANALYZE_PROMPT: &str = r#"你是 OpsBatch 终端的 AI 运维助手。用户框选了终端中的一部分输出，请分析其中可能存在的错误原因，并给出诊断建议和修复方案。

## 分析要求
1. 识别错误类型和错误原因
2. 分析可能的根因
3. 给出具体的修复建议和命令

## 命令格式
当需要执行命令时，在回复末尾用以下格式提议：
[ACTION:命令描述]
实际命令内容
[/ACTION]
可以提议多个命令，每个都用 ACTION 块包裹。用户可以在右侧命令确认栏审批这些命令。批准后命令将通过 bracketed paste 注入终端。"#;

const INLINE_PROMPT: &str = "你是一个终端命令生成器。根据用户描述，生成可直接在终端执行的命令。只输出命令本身，不要解释。如果需要多条命令，用 && 或 ; 连接。";

/// `[终端缓冲区（最近输出）]` block with the last 80 lines, or empty when there is no buffer.
pub fn terminal_buffer_text(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }
    let lines: Vec<&str> = raw.split('\n').collect();
    let tail = &lines[lines.len().saturating_sub(MAX_BUFFER_LINES)..];
    format!("[终端缓冲区（最近输出）]\n{}", tail.join("\n"))
}

/// The system message of a normal chat turn.
pub fn build_context(host: Option<(&str, &str)>, sftp_path: Option<&str>, terminal_buffer: &str) -> String {
    let mut parts = vec![ASSISTANT_PROMPT.to_string()];
    if let Some((name, ip)) = host.filter(|(n, i)| !n.is_empty() && !i.is_empty()) {
        parts.push(format!("当前连接: {name} ({ip})"));
    }
    if let Some(path) = sftp_path.filter(|p| !p.is_empty()) {
        parts.push(format!("SFTP 当前路径: {path}"));
    }
    let buffer = terminal_buffer_text(terminal_buffer);
    if !buffer.is_empty() {
        parts.push(buffer);
    }
    parts.join("\n\n")
}

/// (system context, user message) for "AI analyze error" on a terminal selection.
pub fn analyze_selection(selection: &str, host: Option<(&str, &str)>) -> (String, String) {
    let host_info = match host.filter(|(n, i)| !n.is_empty() && !i.is_empty()) {
        Some((name, ip)) => format!("\n\n当前连接: {name} ({ip})"),
        None => String::new(),
    };
    let truncated = if selection.chars().count() > MAX_SELECTION_CHARS {
        format!("{}\n…(已截断)", selection.chars().take(MAX_SELECTION_CHARS).collect::<String>())
    } else {
        selection.to_string()
    };
    (format!("{ANALYZE_PROMPT}{host_info}"), format!("请分析以下终端输出中的错误原因：\n\n```\n{truncated}\n```"))
}

/// System message of the ⌘I command generator (last 30 lines of the terminal as context).
pub fn inline_context(terminal_buffer: &str) -> String {
    let mut parts = vec![INLINE_PROMPT.to_string()];
    if !terminal_buffer.is_empty() {
        let lines: Vec<&str> = terminal_buffer.split('\n').collect();
        let tail = &lines[lines.len().saturating_sub(30)..];
        parts.push(format!("[终端最近输出]\n{}", tail.join("\n")));
    }
    parts.join("\n\n")
}

/// The follow-up user message after an approved command finished.
pub fn follow_up_message(output: &str) -> String {
    format!("[自动跟进] 当前命令已执行完毕，请分析这条命令的输出和退出状态，判断是否需要下一步操作：\n\n{output}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_contains_host_path_and_buffer_tail() {
        let buffer: String = (0..100).map(|i| format!("l{i}\n")).collect();
        let ctx = build_context(Some(("web", "10.0.0.1")), Some("/var/log"), &buffer);
        assert!(ctx.contains("当前连接: web (10.0.0.1)"));
        assert!(ctx.contains("SFTP 当前路径: /var/log"));
        assert!(ctx.contains("l99") && !ctx.contains("l10\n"));
        assert!(ctx.contains("steps 必须恰好包含一个步骤"));
        let bare = build_context(None, None, "");
        assert!(!bare.contains("当前连接") && !bare.contains("终端缓冲区"));
    }

    #[test]
    fn selection_is_truncated() {
        let (sys, user) = analyze_selection(&"x".repeat(5000), Some(("web", "1.2.3.4")));
        assert!(sys.ends_with("当前连接: web (1.2.3.4)"));
        assert!(user.contains("…(已截断)"));
        assert_eq!(user.matches('x').count(), 4000);
    }

    #[test]
    fn inline_context_keeps_last_thirty_lines() {
        let buffer: String = (0..50).map(|i| format!("row{i}\n")).collect();
        let ctx = inline_context(&buffer);
        assert!(ctx.contains("row49") && ctx.contains("row21") && !ctx.contains("row19\n"));
    }
}
