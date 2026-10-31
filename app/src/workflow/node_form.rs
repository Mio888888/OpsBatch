//! The configuration form of one workflow node (port of `ConfigForm` in `NodeEditModal.tsx`). Every edit is written
//! back as the node's `config` string in exactly the shapes the Tauri build used.

use crate::scale::px;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::select::Select;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ops_i18n::t;
use serde_json::{json, Value};

use crate::stores::assets::AssetsStore;
use crate::widgets::choice::{confirmed_value, new_select, Choice, ChoiceSelect};

pub const REMOTE_PATH_QUICK_FILLS: &[(&str, &str)] = &[
    ("主机名", "/home/{host}/"),
    ("第一个目录", "/home/{firstdir:/home}/"),
    ("/tmp/", "/tmp/"),
    ("/opt/", "/opt/"),
    ("/home/", "/home/"),
    ("/var/log/", "/var/log/"),
    ("/usr/local/bin/", "/usr/local/bin/"),
    ("/etc/", "/etc/"),
    ("/data/", "/data/"),
    ("/www/wwwroot/", "/www/wwwroot/"),
    ("/var/www/", "/var/www/"),
    ("/usr/local/src/", "/usr/local/src/"),
];

// ---------------------------------------------------------------------------
// Config shapes (pure, tested)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferConfig {
    pub direction: String,
    pub local_paths: Vec<String>,
    pub remote_path: String,
}

impl TransferConfig {
    pub fn parse(config: &str) -> Self {
        let v: Value = serde_json::from_str(config).unwrap_or(Value::Null);
        let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let paths: Vec<String> = match v.get("localPaths").and_then(Value::as_array) {
            Some(list) if !list.is_empty() => list
                .iter()
                .filter_map(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
            _ => Some(text("localPath"))
                .filter(|s| !s.is_empty())
                .into_iter()
                .collect(),
        };
        Self {
            direction: if text("direction") == "download" {
                "download".into()
            } else {
                "upload".into()
            },
            local_paths: paths,
            remote_path: text("remotePath"),
        }
    }

    pub fn to_config(&self) -> String {
        json!({
            "localPath": self.local_paths.first().cloned().unwrap_or_default(),
            "localPaths": self.local_paths,
            "remotePath": self.remote_path,
            "direction": self.direction,
        })
        .to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchConfig {
    pub expression: String,
    /// `(label, value)`; the value `*` is the default branch.
    pub cases: Vec<(String, String)>,
}

impl SwitchConfig {
    pub fn parse(config: &str) -> Self {
        let v: Value = serde_json::from_str(config).unwrap_or(Value::Null);
        let cases: Vec<(String, String)> = v
            .get("cases")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .map(|c| {
                        (
                            c.get("label")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                            c.get("value")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_else(|| vec![(t!("nodeEdit.branchDefault"), "*".into())]);
        Self {
            expression: v
                .get("expression")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            cases,
        }
    }

    pub fn to_config(&self) -> String {
        json!({ "expression": self.expression, "cases": self.cases.iter().map(|(l, v)| json!({"label": l, "value": v})).collect::<Vec<_>>() }).to_string()
    }
}

pub fn parse_host_ids(config: &str) -> Vec<String> {
    serde_json::from_str::<Value>(config)
        .ok()
        .and_then(|v| {
            v.get("hostIds").and_then(Value::as_array).map(|l| {
                l.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
        })
        .unwrap_or_default()
}

pub fn host_ids_config(ids: &[String]) -> String {
    json!({ "hostIds": ids }).to_string()
}

/// Template chips offered next to the input of a node type: `(label, text to insert)`.
pub fn snippets(kind: &str) -> Vec<(String, String)> {
    let mut chips = vec![
        (
            t!("nodeEdit.snippetOutput"),
            "{{lastResult.output}}".to_string(),
        ),
        (
            t!("nodeEdit.snippetExitCode"),
            "{{lastResult.exitCode}}".to_string(),
        ),
        (
            t!("nodeEdit.snippetSuccess"),
            "{{lastResult.success}}".to_string(),
        ),
    ];
    if matches!(
        kind,
        "command" | "script" | "quickAction" | "rollback" | "transfer"
    ) {
        chips.push(("IP".into(), "{{host.ip}}".into()));
        chips.push((t!("nodeEdit.snippetHostName"), "{{host.name}}".into()));
    }
    if matches!(kind, "condition" | "switch") {
        chips.push((
            t!("nodeEdit.snippetSucceeded"),
            "{{lastResult.exitCode}} === 0".into(),
        ));
        chips.push((
            t!("nodeEdit.snippetFailed"),
            "{{lastResult.exitCode}} !== 0".into(),
        ));
        chips.push((
            t!("nodeEdit.snippetContains"),
            "{{lastResult.output}}.includes(\"\")".into(),
        ));
    }
    chips
}

// ---------------------------------------------------------------------------
// The form view
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum NodeFormEvent {
    Config(String),
}

pub struct NodeForm {
    kind: String,
    assets: Entity<AssetsStore>,
    /// command / quickAction / script / rollback / confirm text.
    text: Entity<TextareaState>,
    /// delay seconds, condition expression, switch expression, transfer remote path.
    line: Entity<InputState>,
    direction: Entity<ChoiceSelect>,
    transfer: TransferConfig,
    switch: SwitchConfig,
    case_inputs: Vec<(Entity<InputState>, Entity<InputState>)>,
    host_ids: Vec<String>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<NodeFormEvent> for NodeForm {}

fn has_text_area(kind: &str) -> bool {
    matches!(
        kind,
        "command" | "quickAction" | "script" | "rollback" | "confirm"
    )
}

fn placeholder_for(kind: &str) -> String {
    match kind {
        "command" | "quickAction" => t!("nodeEdit.commandPlaceholder"),
        "script" => t!("nodeEdit.scriptPlaceholder"),
        "rollback" => t!("nodeEdit.rollbackCommand"),
        "confirm" => t!("nodeEdit.confirmPlaceholder"),
        "delay" => t!("nodeEdit.delaySeconds"),
        "condition" => t!("nodeEdit.conditionPlaceholder"),
        "switch" => t!("nodeEdit.expressionPlaceholder"),
        "transfer" => t!("nodeEdit.remoteFilePath"),
        _ => t!("nodeEdit.configContent"),
    }
}

impl NodeForm {
    pub fn new(
        kind: &str,
        config: &str,
        assets: Entity<AssetsStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let transfer = TransferConfig::parse(config);
        let switch = SwitchConfig::parse(config);
        let line_initial = match kind {
            "delay" | "condition" => config.to_string(),
            "switch" => switch.expression.clone(),
            "transfer" => transfer.remote_path.clone(),
            _ => String::new(),
        };
        let text_initial = if has_text_area(kind) {
            config.to_string()
        } else {
            String::new()
        };
        let ph = placeholder_for(kind);
        let text = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(ph.clone())
                .auto_grow(3, 8)
                .default_value(text_initial)
        });
        let line = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(ph)
                .default_value(line_initial)
        });
        let direction = cx.new(|cx| {
            new_select(
                vec![
                    Choice::new("upload", t!("nodeEdit.uploadLabel")),
                    Choice::new("download", t!("nodeEdit.downloadLabel")),
                ],
                &transfer.direction,
                window,
                cx,
            )
        });
        let mut this = Self {
            kind: kind.to_string(),
            assets,
            text: text.clone(),
            line: line.clone(),
            direction: direction.clone(),
            transfer,
            switch,
            case_inputs: Vec::new(),
            host_ids: if kind == "selectHost" {
                parse_host_ids(config)
            } else {
                Vec::new()
            },
            _subs: Vec::new(),
        };
        let subs = vec![
            cx.subscribe(&text, |this, state, ev: &InputEvent, cx| {
                if let InputEvent::Change = ev {
                    let v = state.read(cx).value().to_string();
                    cx.emit(NodeFormEvent::Config(v));
                    let _ = this;
                }
            }),
            cx.subscribe(&line, |this, state, ev: &InputEvent, cx| {
                if let InputEvent::Change = ev {
                    let v = state.read(cx).value().to_string();
                    this.line_changed(v, cx);
                }
            }),
            cx.subscribe(&direction, |this, _, ev, cx| {
                if let Some(v) = confirmed_value(ev) {
                    this.transfer.direction = v;
                    cx.emit(NodeFormEvent::Config(this.transfer.to_config()));
                    cx.notify();
                }
            }),
            cx.observe(&this.assets, |_, _, cx| cx.notify()),
        ];
        this._subs = subs;
        if kind == "switch" {
            this.rebuild_case_inputs(window, cx);
        }
        this
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }
    pub fn text_input(&self) -> &Entity<TextareaState> {
        &self.text
    }
    pub fn line_input(&self) -> &Entity<InputState> {
        &self.line
    }
    pub fn case_inputs(&self) -> &[(Entity<InputState>, Entity<InputState>)] {
        &self.case_inputs
    }

    fn line_changed(&mut self, value: String, cx: &mut Context<Self>) {
        match self.kind.as_str() {
            "delay" | "condition" => cx.emit(NodeFormEvent::Config(value)),
            "switch" => {
                self.switch.expression = value;
                cx.emit(NodeFormEvent::Config(self.switch.to_config()));
            }
            "transfer" => {
                self.transfer.remote_path = value;
                cx.emit(NodeFormEvent::Config(self.transfer.to_config()));
            }
            _ => {}
        }
    }

    fn rebuild_case_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.case_inputs = self
            .switch
            .cases
            .iter()
            .enumerate()
            .map(|(i, (label, value))| {
                let l = cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(t!("nodeEdit.labelPlaceholder"))
                        .default_value(label.clone())
                });
                let v = cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(t!("nodeEdit.valuePlaceholder"))
                        .default_value(value.clone())
                });
                self._subs
                    .push(cx.subscribe(&l, move |this, s, ev: &InputEvent, cx| {
                        if let InputEvent::Change = ev {
                            if let Some(c) = this.switch.cases.get_mut(i) {
                                c.0 = s.read(cx).value().to_string();
                                cx.emit(NodeFormEvent::Config(this.switch.to_config()));
                            }
                        }
                    }));
                self._subs
                    .push(cx.subscribe(&v, move |this, s, ev: &InputEvent, cx| {
                        if let InputEvent::Change = ev {
                            if let Some(c) = this.switch.cases.get_mut(i) {
                                c.1 = s.read(cx).value().to_string();
                                cx.emit(NodeFormEvent::Config(this.switch.to_config()));
                            }
                        }
                    }));
                (l, v)
            })
            .collect();
    }

    pub fn add_case(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.switch.cases.len() + 1;
        self.switch
            .cases
            .push((t!("nodeEdit.branchPlaceholder", count = n), String::new()));
        self.rebuild_case_inputs(window, cx);
        cx.emit(NodeFormEvent::Config(self.switch.to_config()));
        cx.notify();
    }

    pub fn remove_case(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index < self.switch.cases.len() {
            self.switch.cases.remove(index);
            self.rebuild_case_inputs(window, cx);
            cx.emit(NodeFormEvent::Config(self.switch.to_config()));
            cx.notify();
        }
    }

    pub fn toggle_host(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(i) = self.host_ids.iter().position(|h| h == id) {
            self.host_ids.remove(i);
        } else {
            self.host_ids.push(id.to_string());
        }
        cx.emit(NodeFormEvent::Config(host_ids_config(&self.host_ids)));
        cx.notify();
    }

    fn set_local_paths(&mut self, paths: Vec<String>, cx: &mut Context<Self>) {
        self.transfer.local_paths = paths;
        cx.emit(NodeFormEvent::Config(self.transfer.to_config()));
        cx.notify();
    }

    fn pick_local(&mut self, directory: bool, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: !directory,
            directories: directory,
            multiple: !directory,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let list: Vec<String> = paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            let _ = this.update(cx, |this, cx| this.set_local_paths(list, cx));
        })
        .detach();
    }

    fn insert_snippet(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let text = text.to_string();
        if has_text_area(&self.kind) {
            self.text.update(cx, |s, cx| s.insert(text, window, cx));
        } else {
            self.line.update(cx, |s, cx| s.insert(text, window, cx));
        }
    }

    pub fn set_remote_path(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let p = path.to_string();
        self.line.update(cx, |s, cx| s.set_value(p, window, cx));
        self.transfer.remote_path = path.to_string();
        cx.emit(NodeFormEvent::Config(self.transfer.to_config()));
    }

    fn chips(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        h_flex()
            .gap_1()
            .flex_wrap()
            .items_center()
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(theme.muted_foreground)
                    .child("{ }"),
            )
            .children(
                snippets(&self.kind)
                    .into_iter()
                    .enumerate()
                    .map(|(i, (label, value))| {
                        Button::new(ElementId::Name(format!("wf-snippet-{i}").into()))
                            .ghost()
                            .xsmall()
                            .label(label)
                            .tooltip(value.clone())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.insert_snippet(&value, window, cx)
                            }))
                    }),
            )
    }
}

impl Render for NodeForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let body: AnyElement = match self.kind.as_str() {
            "start" | "end" => div()
                .text_size(px(12.))
                .text_color(theme.muted_foreground)
                .child(t!("nodeEdit.noConfig"))
                .into_any_element(),
            "command" | "quickAction" | "script" | "rollback" | "confirm" => {
                let chips = self.chips(cx);
                v_flex()
                    .gap_1()
                    .child(Textarea::new(&self.text))
                    .child(chips)
                    .into_any_element()
            }
            "delay" | "condition" => {
                let chips = self.chips(cx);
                v_flex()
                    .gap_1()
                    .child(Input::new(&self.line))
                    .child(chips)
                    .into_any_element()
            }
            "switch" => {
                let chips = self.chips(cx);
                v_flex()
                    .gap_2()
                    .child(Input::new(&self.line))
                    .child(chips)
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme.muted_foreground)
                            .child(t!("nodeEdit.branchList")),
                    )
                    .children(self.case_inputs.iter().enumerate().map(|(i, (l, v))| {
                        h_flex()
                            .id(ElementId::Name(format!("wf-case-{i}").into()))
                            .test_support()
                            .gap_1()
                            .items_center()
                            .child(div().flex_1().child(Input::new(l).small()))
                            .child(div().flex_1().child(Input::new(v).small()))
                            .child(
                                Button::new(ElementId::Name(format!("wf-case-del-{i}").into()))
                                    .ghost()
                                    .xsmall()
                                    .label("x")
                                    .tooltip(t!("nodeEdit.deleteBranch"))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.remove_case(i, window, cx)
                                    })),
                            )
                    }))
                    .child(
                        Button::new("wf-case-add")
                            .small()
                            .label(t!("nodeEdit.addBranch"))
                            .on_click(cx.listener(|this, _, window, cx| this.add_case(window, cx))),
                    )
                    .into_any_element()
            }
            "transfer" => {
                let chips = self.chips(cx);
                v_flex()
                    .gap_2()
                    .child(Select::new(&self.direction))
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("wf-pick-files")
                                    .small()
                                    .label(t!("nodeEdit.selectFileButton"))
                                    .tooltip(t!("nodeEdit.selectLocalFiles"))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.pick_local(false, cx)),
                                    ),
                            )
                            .child(
                                Button::new("wf-pick-folder")
                                    .small()
                                    .label(t!("nodeEdit.selectFolderButton"))
                                    .tooltip(t!("nodeEdit.selectLocalFolder"))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.pick_local(true, cx)),
                                    ),
                            ),
                    )
                    .children(self.transfer.local_paths.iter().enumerate().map(|(i, p)| {
                        let name = p
                            .rsplit(['/', '\\'])
                            .find(|s| !s.is_empty())
                            .unwrap_or(p)
                            .to_string();
                        let full = p.clone();
                        let remove = p.clone();
                        h_flex()
                            .id(ElementId::Name(format!("wf-local-{i}").into()))
                            .test_support()
                            .gap_2()
                            .items_center()
                            .child(
                                v_flex()
                                    .min_w_0()
                                    .flex_1()
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(name),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(theme.muted_foreground)
                                            .truncate()
                                            .child(full),
                                    ),
                            )
                            .child(
                                Button::new(ElementId::Name(format!("wf-local-del-{i}").into()))
                                    .ghost()
                                    .xsmall()
                                    .label("x")
                                    .tooltip(t!("common.delete"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let rest: Vec<String> = this
                                            .transfer
                                            .local_paths
                                            .iter()
                                            .filter(|x| **x != remove)
                                            .cloned()
                                            .collect();
                                        this.set_local_paths(rest, cx);
                                    })),
                            )
                    }))
                    .child(Input::new(&self.line))
                    .child(chips)
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme.muted_foreground)
                            .child(t!("nodeEdit.remotePathQuickFill")),
                    )
                    .child(
                        h_flex().gap_1().flex_wrap().children(
                            REMOTE_PATH_QUICK_FILLS.iter().enumerate().map(
                                |(i, (label, value))| {
                                    Button::new(ElementId::Name(format!("wf-preset-{i}").into()))
                                        .ghost()
                                        .xsmall()
                                        .label(*label)
                                        .tooltip(*value)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.set_remote_path(value, window, cx)
                                        }))
                                },
                            ),
                        ),
                    )
                    .into_any_element()
            }
            "selectHost" => {
                let hosts: Vec<(String, String)> = self
                    .assets
                    .read(cx)
                    .hosts
                    .iter()
                    .map(|h| {
                        (
                            h.id.clone(),
                            if h.name.is_empty() {
                                h.ip.clone()
                            } else {
                                h.name.clone()
                            },
                        )
                    })
                    .collect();
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme.muted_foreground)
                            .child(t!("nodeEdit.selectTargetHosts")),
                    )
                    .child(
                        v_flex()
                            .id("wf-host-list")
                            .max_h(px(200.))
                            .overflow_y_scroll()
                            .border_1()
                            .border_color(theme.border)
                            .rounded(px(6.))
                            .p_1()
                            .when(hosts.is_empty(), |d| {
                                d.child(
                                    div()
                                        .p_3()
                                        .text_size(px(12.))
                                        .text_color(theme.muted_foreground)
                                        .child(t!("nodeEdit.noHosts")),
                                )
                            })
                            .children(hosts.into_iter().enumerate().map(|(i, (id, name))| {
                                let selected = self.host_ids.contains(&id);
                                h_flex()
                                    .id(ElementId::Name(format!("wf-host-{i}").into()))
                                    .test_support()
                                    .gap_2()
                                    .px_1p5()
                                    .py_1()
                                    .rounded(px(4.))
                                    .cursor_pointer()
                                    .when(selected, |d| d.bg(theme.accent))
                                    .child(div().child(if selected { "☑" } else { "☐" }))
                                    .child(div().text_size(px(12.)).child(name))
                                    .on_click(
                                        cx.listener(move |this, _, _, cx| {
                                            this.toggle_host(&id, cx)
                                        }),
                                    )
                            })),
                    )
                    .when(!self.host_ids.is_empty(), |d| {
                        d.child(
                            div()
                                .id("wf-host-count")
                                .test_support()
                                .text_size(px(11.))
                                .text_color(theme.muted_foreground)
                                .child(t!("nodeEdit.selectedCount", count = self.host_ids.len())),
                        )
                    })
                    .into_any_element()
            }
            _ => Input::new(&self.line).into_any_element(),
        };
        v_flex()
            .id("wf-node-form")
            .test_support()
            .gap_2()
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::{host_ids_config, parse_host_ids, snippets, SwitchConfig, TransferConfig};

    #[test]
    fn transfer_config_keeps_the_original_shape_and_reads_legacy_forms() {
        let c = TransferConfig::parse(
            r#"{"localPath":"/a","remotePath":"/tmp/","direction":"download"}"#,
        );
        assert_eq!(
            (
                c.direction.as_str(),
                c.local_paths.clone(),
                c.remote_path.as_str()
            ),
            ("download", vec!["/a".to_string()], "/tmp/")
        );
        let multi = TransferConfig::parse(
            r#"{"localPath":"/ignored","localPaths":["/x","","/y"],"remotePath":"/r"}"#,
        );
        assert_eq!(multi.local_paths, ["/x", "/y"]);
        assert_eq!(multi.direction, "upload", "default direction");
        let json: serde_json::Value = serde_json::from_str(&multi.to_config()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"localPath": "/x", "localPaths": ["/x", "/y"], "remotePath": "/r", "direction": "upload"})
        );
        assert_eq!(TransferConfig::parse("garbage").local_paths.len(), 0);
    }

    #[test]
    fn switch_config_round_trips_and_defaults_to_a_catch_all() {
        let d = SwitchConfig::parse("");
        assert_eq!(d.cases.len(), 1);
        assert_eq!(d.cases[0].1, "*");
        let c = SwitchConfig {
            expression: "{{lastResult.exitCode}}".into(),
            cases: vec![("成功".into(), "0".into()), ("其它".into(), "*".into())],
        };
        assert_eq!(SwitchConfig::parse(&c.to_config()), c);
        assert_eq!(
            SwitchConfig::parse(r#"{"expression":"x","cases":[]}"#)
                .cases
                .len(),
            0
        );
    }

    #[test]
    fn host_selection_config() {
        assert_eq!(parse_host_ids(r#"{"hostIds":["a","b"]}"#), ["a", "b"]);
        assert!(parse_host_ids("").is_empty());
        assert_eq!(host_ids_config(&["a".into()]), r#"{"hostIds":["a"]}"#);
    }

    #[test]
    fn snippets_depend_on_the_node_type() {
        let values = |k: &str| snippets(k).into_iter().map(|s| s.1).collect::<Vec<_>>();
        assert_eq!(values("delay").len(), 3);
        assert!(values("command").contains(&"{{host.ip}}".to_string()));
        assert!(values("condition").contains(&"{{lastResult.exitCode}} === 0".to_string()));
        assert!(!values("condition").contains(&"{{host.ip}}".to_string()));
    }
}
