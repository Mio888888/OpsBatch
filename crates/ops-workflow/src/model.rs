//! Workflow data model (port of `stores/workflow.ts` + `components/nodeTypes.ts`). The JSON shapes are the ones the
//! Tauri build stored, so existing databases keep working.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanvasNode {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub config: String,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_width")]
    pub width: f64,
    #[serde(default = "default_height")]
    pub height: f64,
}

fn default_true() -> bool {
    true
}
fn default_width() -> f64 {
    160.0
}
fn default_height() -> f64 {
    56.0
}

impl CanvasNode {
    pub fn new(id: impl Into<String>, kind: &str, name: &str, config: &str) -> Self {
        Self { id: id.into(), kind: kind.into(), name: name.into(), config: config.into(), x: 0.0, y: 0.0, enabled: true, width: 160.0, height: 56.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: String,
    pub from_id: String,
    pub to_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_handle: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSettings {
    pub default_node_interval_seconds: f64,
}

impl Default for WorkflowSettings {
    fn default() -> Self {
        Self { default_node_interval_seconds: 0.0 }
    }
}

impl WorkflowSettings {
    fn normalized(self) -> Self {
        let v = self.default_node_interval_seconds;
        Self { default_node_interval_seconds: if v.is_finite() && v > 0.0 { v } else { 0.0 } }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowPayload {
    pub nodes: Vec<CanvasNode>,
    pub settings: WorkflowSettings,
}

/// The `nodes` column holds either a bare node array (old builds) or `{nodes, settings}`.
pub fn parse_nodes_payload(raw: &str) -> Result<WorkflowPayload, String> {
    let raw = if raw.trim().is_empty() { "[]" } else { raw };
    let value: serde_json::Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    match value {
        serde_json::Value::Array(_) => Ok(WorkflowPayload { nodes: serde_json::from_value(value).map_err(|e| e.to_string())?, settings: WorkflowSettings::default() }),
        serde_json::Value::Object(mut map) => {
            let nodes = match map.remove("nodes") {
                Some(v @ serde_json::Value::Array(_)) => serde_json::from_value(v).map_err(|e| e.to_string())?,
                _ => Vec::new(),
            };
            let settings = map.remove("settings").and_then(|v| serde_json::from_value::<WorkflowSettings>(v).ok()).unwrap_or_default().normalized();
            Ok(WorkflowPayload { nodes, settings })
        }
        _ => Ok(WorkflowPayload { nodes: Vec::new(), settings: WorkflowSettings::default() }),
    }
}

pub fn stringify_nodes_payload(nodes: &[CanvasNode], settings: WorkflowSettings) -> String {
    serde_json::json!({ "nodes": nodes, "settings": settings.normalized() }).to_string()
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowRecord {
    pub id: String,
    pub name: String,
    pub description: String,
    pub nodes: Vec<CanvasNode>,
    pub connections: Vec<Connection>,
    pub settings: WorkflowSettings,
    pub status: WorkflowStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowStatus {
    Draft,
    Ready,
}

impl WorkflowStatus {
    pub fn parse(s: &str) -> Self {
        if s == "ready" {
            Self::Ready
        } else {
            Self::Draft
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Ready => "ready",
        }
    }
}

/// Every host selected by any `selectHost` node (unique, in first-seen order). Broken configs are ignored.
pub fn collect_workflow_host_ids(nodes: &[CanvasNode]) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for node in nodes.iter().filter(|n| n.kind == "selectHost") {
        let Ok(cfg) = serde_json::from_str::<serde_json::Value>(if node.config.is_empty() { "{}" } else { &node.config }) else { continue };
        if let Some(list) = cfg.get("hostIds").and_then(|v| v.as_array()) {
            for id in list.iter().filter_map(|v| v.as_str()).filter(|s| !s.trim().is_empty()) {
                if !ids.iter().any(|x| x == id) {
                    ids.push(id.to_string());
                }
            }
        }
    }
    ids
}

pub struct NodeTypeEntry {
    pub value: &'static str,
    pub label_key: &'static str,
    pub fallback_label: &'static str,
    pub color: u32,
}

pub struct NodeGroup {
    pub label_key: &'static str,
    pub fallback_label: &'static str,
    pub types: &'static [NodeTypeEntry],
}

const fn nt(value: &'static str, label_key: &'static str, fallback_label: &'static str, color: u32) -> NodeTypeEntry {
    NodeTypeEntry { value, label_key, fallback_label, color }
}

pub static NODE_GROUPS: &[NodeGroup] = &[
    NodeGroup { label_key: "nodePalette.flowControl", fallback_label: "流程控制", types: &[nt("start", "nodePalette.start", "开始", 0x52c41a), nt("end", "nodePalette.end", "结束", 0xff4d4f)] },
    NodeGroup { label_key: "nodePalette.target", fallback_label: "目标", types: &[nt("selectHost", "nodePalette.selectHost", "选择主机", 0x1677ff)] },
    NodeGroup {
        label_key: "nodePalette.executeOps",
        fallback_label: "执行操作",
        types: &[
            nt("command", "nodePalette.executeCommand", "执行命令", 0x1677ff),
            nt("script", "nodePalette.executeScript", "执行脚本", 0x52c41a),
            nt("quickAction", "nodePalette.quickAction", "快捷指令", 0xfa8c16),
            nt("transfer", "nodePalette.fileTransfer", "文件传输", 0x722ed1),
        ],
    },
    NodeGroup {
        label_key: "nodePalette.logicControl",
        fallback_label: "逻辑控制",
        types: &[
            nt("condition", "nodePalette.condition", "条件判断", 0x13c2c2),
            nt("switch", "nodePalette.switch", "多分支判断", 0x722ed1),
            nt("delay", "nodePalette.delay", "等待延时", 0x8c8c8c),
            nt("confirm", "nodePalette.manualConfirm", "人工确认", 0xeb2f96),
        ],
    },
    NodeGroup { label_key: "nodePalette.rollback", fallback_label: "回滚", types: &[nt("rollback", "nodePalette.rollbackOp", "回滚操作", 0xff4d4f)] },
];

pub fn node_type_entry(kind: &str) -> Option<&'static NodeTypeEntry> {
    NODE_GROUPS.iter().flat_map(|g| g.types.iter()).find(|t| t.value == kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_round_trips_and_accepts_the_legacy_array_form() {
        let nodes = vec![CanvasNode::new("a", "command", "跑命令", "ls")];
        let raw = stringify_nodes_payload(&nodes, WorkflowSettings { default_node_interval_seconds: 3.0 });
        let back = parse_nodes_payload(&raw).unwrap();
        assert_eq!(back.nodes, nodes);
        assert_eq!(back.settings.default_node_interval_seconds, 3.0);
        let legacy = parse_nodes_payload(r#"[{"id":"x","type":"start","name":"s","config":"","x":1,"y":2,"enabled":true,"width":160,"height":56}]"#).unwrap();
        assert_eq!(legacy.nodes[0].kind, "start");
        assert_eq!(legacy.settings, WorkflowSettings::default());
        assert!(parse_nodes_payload("").unwrap().nodes.is_empty());
        assert!(parse_nodes_payload("not json").is_err());
    }

    #[test]
    fn settings_are_normalised() {
        let p = parse_nodes_payload(r#"{"nodes":[],"settings":{"defaultNodeIntervalSeconds":-4}}"#).unwrap();
        assert_eq!(p.settings.default_node_interval_seconds, 0.0);
        let p = parse_nodes_payload(r#"{"nodes":"oops"}"#).unwrap();
        assert!(p.nodes.is_empty());
    }

    #[test]
    fn connection_json_matches_the_tauri_shape() {
        let c = Connection { id: "e1".into(), from_id: "a".into(), to_id: "b".into(), source_handle: Some("true".into()) };
        assert_eq!(serde_json::to_string(&c).unwrap(), r#"{"id":"e1","fromId":"a","toId":"b","sourceHandle":"true"}"#);
        let plain: Connection = serde_json::from_str(r#"{"id":"e","fromId":"a","toId":"b"}"#).unwrap();
        assert_eq!(plain.source_handle, None);
        assert_eq!(serde_json::to_string(&plain).unwrap(), r#"{"id":"e","fromId":"a","toId":"b"}"#);
    }

    #[test]
    fn host_ids_come_from_select_host_nodes_only() {
        let nodes = vec![
            CanvasNode::new("1", "selectHost", "h", r#"{"hostIds":["h1","h2"," ",3]}"#),
            CanvasNode::new("2", "selectHost", "h", r#"{"hostIds":["h2","h3"]}"#),
            CanvasNode::new("3", "selectHost", "broken", "{nope"),
            CanvasNode::new("4", "command", "c", r#"{"hostIds":["zzz"]}"#),
            CanvasNode::new("5", "selectHost", "empty", ""),
        ];
        assert_eq!(collect_workflow_host_ids(&nodes), vec!["h1", "h2", "h3"]);
    }

    #[test]
    fn node_palette_has_all_original_types() {
        let all: Vec<&str> = NODE_GROUPS.iter().flat_map(|g| g.types.iter().map(|t| t.value)).collect();
        assert_eq!(all, ["start", "end", "selectHost", "command", "script", "quickAction", "transfer", "condition", "switch", "delay", "confirm", "rollback"]);
        assert_eq!(node_type_entry("delay").unwrap().color, 0x8c8c8c);
    }
}
