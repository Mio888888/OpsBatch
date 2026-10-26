//! Verification of one RDP assistant step: the model looks at the screen before and after the operations and says
//! whether the step did what it should, or whether something went wrong (wrong click, text in the wrong window, an
//! unexpected dialog, an error message).

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepVerdict {
    pub ok: bool,
    /// What the model sees on the screen afterwards (one sentence).
    pub observation: String,
    /// Why the step is considered wrong; empty when `ok`.
    pub reason: String,
}

pub fn verification_system_prompt() -> String {
    [
        "你是 Windows 远程桌面操作的结果校验器。",
        "你会收到两张远程桌面截图：第一张是执行操作之前，第二张是执行操作并等待画面稳定之后；以及步骤描述、实际执行的操作和预期结果。",
        "请判断这一步是否达到预期，并检查是否有误操作：点错位置、文字输入到了错误的窗口或输入框、弹出了意外的对话框、出现错误提示、程序没有启动、窗口被关闭等。",
        "只有明显没有达到预期、或确认出现误操作/错误提示时才判定失败；画面与预期基本一致（允许细节不同）就判定成功。",
        "只输出下面这一个块，不要输出其他内容：",
        "<RDP_VERIFY>{\"ok\": true或false, \"observation\": \"操作后画面里看到的关键事实，一句话\", \"reason\": \"失败或误操作的原因；成功时留空\"}</RDP_VERIFY>",
    ]
    .join("\n")
}

pub fn build_verification_prompt(description: &str, operations: &[String], expected_outcome: Option<&str>) -> String {
    let mut lines = vec![format!("步骤：{description}")];
    if !operations.is_empty() {
        lines.push("已执行的操作：".to_string());
        lines.extend(operations.iter().map(|o| format!("- {o}")));
    }
    match expected_outcome.map(str::trim).filter(|e| !e.is_empty()) {
        Some(expected) => lines.push(format!("预期结果：{expected}")),
        None => lines.push("预期结果：步骤描述所说的事情已经发生。".to_string()),
    }
    lines.push("第一张图是操作前，第二张图是操作后。请校验。".to_string());
    lines.join("\n")
}

fn truthy(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(b) => Some(*b),
        Value::String(s) => match s.trim().to_lowercase().as_str() {
            "true" | "yes" | "ok" | "success" | "passed" | "是" | "成功" => Some(true),
            "false" | "no" | "fail" | "failed" | "否" | "失败" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn text_of(value: Option<&Value>) -> String {
    value.and_then(Value::as_str).map(|s| s.trim().to_string()).unwrap_or_default()
}

/// Read the verdict out of the model's reply (a tagged block, a fenced block or a bare JSON object).
pub fn parse_verdict(reply: &str) -> Result<StepVerdict, String> {
    let lower = reply.to_lowercase();
    let body = match (lower.find("<rdp_verify>"), lower.find("</rdp_verify>")) {
        (Some(start), Some(end)) if end > start => &reply[start + "<rdp_verify>".len()..end],
        (Some(start), None) => &reply[start + "<rdp_verify>".len()..],
        _ => reply,
    };
    let (open, close) = (body.find('{'), body.rfind('}'));
    let json = match (open, close) {
        (Some(a), Some(b)) if b > a => &body[a..=b],
        _ => return Err("校验结果不是 JSON".to_string()),
    };
    let value: Value = serde_json::from_str(json).map_err(|e| format!("校验结果无法解析: {e}"))?;
    let ok = ["ok", "success", "passed", "pass"].iter().find_map(|k| value.get(*k).and_then(truthy)).ok_or_else(|| "校验结果缺少 ok".to_string())?;
    Ok(StepVerdict { ok, observation: text_of(value.get("observation")), reason: text_of(value.get("reason")) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdicts_are_read_from_tagged_fenced_or_bare_json() {
        let tagged = r#"好的。<RDP_VERIFY>{"ok":true,"observation":"记事本已打开","reason":""}</RDP_VERIFY>"#;
        assert_eq!(parse_verdict(tagged).unwrap(), StepVerdict { ok: true, observation: "记事本已打开".into(), reason: String::new() });
        let fenced = "```json\n{\"ok\": false, \"observation\": \"弹出了错误对话框\", \"reason\": \"找不到文件\"}\n```";
        let v = parse_verdict(fenced).unwrap();
        assert!(!v.ok && v.reason == "找不到文件");
        assert!(parse_verdict("{\"success\":\"yes\"}").unwrap().ok);
        assert!(!parse_verdict("<rdp_verify>{\"ok\":\"失败\",\"reason\":\"x\"}</rdp_verify>").unwrap().ok);
        // an unterminated block still works
        assert!(parse_verdict("<RDP_VERIFY>{\"ok\":true}").unwrap().ok);
        assert!(parse_verdict("看起来没问题").is_err());
        assert!(parse_verdict("{\"observation\":\"x\"}").is_err());
    }

    #[test]
    fn the_prompt_names_the_step_the_operations_and_the_expectation() {
        let p = build_verification_prompt("打开记事本", &["按 Win".into(), "输入 notepad".into()], Some("记事本窗口出现"));
        assert!(p.contains("步骤：打开记事本") && p.contains("- 输入 notepad") && p.contains("预期结果：记事本窗口出现"));
        assert!(build_verification_prompt("x", &[], None).contains("已经发生"));
        assert!(verification_system_prompt().contains("RDP_VERIFY"));
    }
}
