use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::Duration;

use ops_workflow::executor::*;
use ops_workflow::model::{CanvasNode, Connection};

type CommandFn = Box<dyn Fn(&str) -> Result<CommandOutput, String>>;

struct Mock {
    commands: RefCell<Vec<(Vec<String>, String, CommandSpec)>>,
    transfers: RefCell<Vec<(String, String, String, u64)>>,
    confirms: RefCell<Vec<(String, String)>>,
    sleeps: RefCell<Vec<Duration>>,
    answer_confirm: Cell<bool>,
    cancel_after_commands: Cell<Option<usize>>,
    transfer_error: RefCell<Option<String>>,
    on_command: CommandFn,
}

impl Mock {
    fn new() -> Self {
        Self {
            commands: RefCell::default(),
            transfers: RefCell::default(),
            confirms: RefCell::default(),
            sleeps: RefCell::default(),
            answer_confirm: Cell::new(true),
            cancel_after_commands: Cell::new(None),
            transfer_error: RefCell::default(),
            on_command: Box::new(|cmd| Ok(CommandOutput { output: format!("[web] ran {cmd}"), success: true, exit_code: 0 })),
        }
    }
    fn with_command(mut self, f: impl Fn(&str) -> Result<CommandOutput, String> + 'static) -> Self {
        self.on_command = Box::new(f);
        self
    }
    fn ran(&self) -> Vec<String> {
        self.commands.borrow().iter().map(|c| c.1.clone()).collect()
    }
}

impl WorkflowRuntime for Mock {
    async fn run_command(&self, host_ids: &[String], command: &str, spec: CommandSpec) -> Result<CommandOutput, String> {
        self.commands.borrow_mut().push((host_ids.to_vec(), command.to_string(), spec));
        (self.on_command)(command)
    }
    async fn transfer(&self, _hosts: &[String], direction: &str, local: &str, remote: &str, timeout: u64) -> Result<(), String> {
        self.transfers.borrow_mut().push((direction.into(), local.into(), remote.into(), timeout));
        match self.transfer_error.borrow().clone() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
    async fn confirm(&self, name: &str, description: &str) -> bool {
        self.confirms.borrow_mut().push((name.into(), description.into()));
        self.answer_confirm.get()
    }
    async fn sleep(&self, d: Duration) {
        self.sleeps.borrow_mut().push(d);
    }
    fn cancelled(&self) -> bool {
        self.cancel_after_commands.get().is_some_and(|n| self.commands.borrow().len() >= n)
    }
}

fn node(id: &str, kind: &str, config: &str) -> CanvasNode {
    CanvasNode::new(id, kind, &format!("node-{id}"), config)
}

fn edge(from: &str, to: &str, handle: Option<&str>) -> Connection {
    Connection { id: format!("{from}-{to}"), from_id: from.into(), to_id: to.into(), source_handle: handle.map(str::to_string) }
}

fn hosts() -> Vec<String> {
    vec!["h1".into(), "h2".into()]
}

struct Run {
    events: Vec<ProgressEvent>,
}

impl Run {
    fn completed(&self) -> Vec<(String, bool, String)> {
        self.events
            .iter()
            .filter_map(|e| if let ProgressEvent::NodeComplete { node_id, success, output, .. } = e { Some((node_id.clone(), *success, output.clone())) } else { None })
            .collect()
    }
    fn started(&self) -> Vec<String> {
        self.events.iter().filter_map(|e| if let ProgressEvent::NodeStart { node_id, .. } = e { Some(node_id.clone()) } else { None }).collect()
    }
    fn logs(&self) -> Vec<String> {
        self.events.iter().filter_map(|e| if let ProgressEvent::Log(m) = e { Some(m.clone()) } else { None }).collect()
    }
}

fn run(nodes: &[CanvasNode], edges: &[Connection], rt: &Mock, interval: f64) -> Run {
    let mut events = Vec::new();
    ops_core::rt::block_on(execute_workflow(nodes, edges, &hosts(), rt, &mut |e| events.push(e), interval));
    Run { events }
}

fn result(output: &str, success: bool, exit_code: i32) -> NodeResult {
    NodeResult { success, output: output.into(), exit_code }
}

// ---------------------------------------------------------------------------

#[test]
fn templates_resolve_last_and_named_results() {
    let mut map = HashMap::new();
    map.insert("a1".to_string(), result("alpha", true, 0));
    map.insert("b-2".to_string(), result("beta", false, 3));
    let last = result("last out", false, 7);
    let r = |t: &str| resolve_templates(t, Some(&last), &map);
    assert_eq!(r("{{lastResult}}|{{lastResult.output}}|{{lastResult.exitCode}}|{{lastResult.success}}"), "last out|last out|7|false");
    assert_eq!(r("{{result.a1}} {{result.a1.output}} {{result.b-2.exitCode}} {{result.b-2.success}}"), "alpha alpha 3 false");
    assert_eq!(r("{{result.nope}}|{{result.nope.exitCode}}"), "|");
    assert_eq!(resolve_templates("{{lastResult}}|{{lastResult.exitCode}}|{{lastResult.success}}", None, &map), "||");
    assert_eq!(r("no templates {{host.ip}}"), "no templates {{host.ip}}", "host placeholders are left to the backend");
}

#[test]
fn conditions_follow_the_original_rules() {
    let none = HashMap::new();
    let last = result("ok 5", true, 0);
    let c = |e: &str| evaluate_condition(e, Some(&last), &none);
    assert!(c("true") && !c("false"));
    assert!(c("{{lastResult.exitCode}} === 0") && !c("{{lastResult.exitCode}} !== 0"));
    assert!(c("1 < 2") && c("2 >= 2") && !c("3 <= 2") && c("10 > 9"), "numbers compare as numbers");
    assert!(c("'abc' == \"abc\"") && c("abc != abd") && !c("abc === abd"));
    assert!(!c("abc > abb"), "ordering of non-numbers never passes");
    assert!(c("\"7\" === 7"), "quotes are stripped and numbers compared");
    assert!(c("{{lastResult.success}} === true"));
    assert!(c("!false") && !c("!true"));
    assert!(c("hello") && c("1") && !c("0") && !c("") && !c("  "));
    assert!(c("0x10 === 16") && c(" === 0"), "JS Number('') is 0");
    // Without a previous result the template is empty, and JS `Number('') === 0`, so this passes.
    assert!(evaluate_condition("{{lastResult.exitCode}} === 0", None, &none));
}

#[test]
fn levels_group_by_dependency_depth() {
    let nodes = [node("a", "start", ""), node("b", "command", "x"), node("c", "command", "x"), node("d", "end", "")];
    let edges = [edge("a", "b", None), edge("a", "c", None), edge("b", "d", None), edge("c", "d", None), edge("a", "ghost", None)];
    let levels = build_execution_levels(&nodes, &edges);
    let ids: Vec<Vec<&str>> = levels.iter().map(|l| l.iter().map(|n| n.id.as_str()).collect()).collect();
    assert_eq!(ids, vec![vec!["a"], vec!["b", "c"], vec!["d"]]);

    let cyc = [node("x", "command", ""), node("y", "command", ""), node("z", "command", "")];
    let ids: Vec<Vec<&str>> = build_execution_levels(&cyc, &[edge("x", "y", None), edge("y", "x", None)]).iter().map(|l| l.iter().map(|n| n.id.as_str()).collect()).collect();
    assert_eq!(ids, vec![vec!["z"], vec!["x"], vec!["y"]], "nodes in a cycle come last, one per level");
}

#[test]
fn a_linear_workflow_passes_results_forward_and_reports_progress() {
    let nodes = [node("s", "start", ""), node("h", "selectHost", r#"{"hostIds":["h1","h2"]}"#), node("c1", "command", "echo one"), node("c2", "command", "echo got:{{lastResult.output}}|{{result.c1.exitCode}}")];
    let edges = [edge("s", "h", None), edge("h", "c1", None), edge("c1", "c2", None)];
    let rt = Mock::new();
    let r = run(&nodes, &edges, &rt, 0.0);
    assert_eq!(rt.ran(), vec!["echo one", "echo got:[web] ran echo one|0"]);
    assert_eq!(rt.commands.borrow()[0].0, hosts());
    assert_eq!(rt.commands.borrow()[0].2, COMMAND_SPEC);
    assert_eq!(r.started(), ["s", "h", "c1", "c2"]);
    assert_eq!(r.completed()[0], ("s".to_string(), true, "start".to_string()), "marker nodes report their type");
    assert_eq!(r.events.first(), Some(&ProgressEvent::Log("开始执行工作流（4 个步骤）".into())));
    assert_eq!(r.events[1], ProgressEvent::LevelStart { level: 1, total: 4, count: 1 });
    assert_eq!(r.events.last(), Some(&ProgressEvent::Done));
    assert!(r.logs().contains(&"工作流执行完成".to_string()));
    assert!(rt.sleeps.borrow().is_empty());
}

#[test]
fn failed_commands_are_reported_but_do_not_stop_the_flow() {
    let rt = Mock::new().with_command(|c| if c == "bad" { Ok(CommandOutput { output: "[web] boom".into(), success: false, exit_code: 2 }) } else { Err("ssh down".into()) });
    let nodes = [node("a", "command", "bad"), node("b", "command", "worse"), node("c", "command", "{{lastResult.exitCode}}")];
    let r = run(&nodes, &[edge("a", "b", None), edge("b", "c", None)], &rt, 0.0);
    let done = r.completed();
    assert_eq!(done[0], ("a".into(), false, "[web] boom".into()));
    assert_eq!(done[1], ("b".into(), false, "ssh down".into()));
    assert_eq!(rt.ran()[2], "1", "an error counts as exit code 1 for the next node");
}

#[test]
fn empty_output_gets_a_default_text() {
    let rt = Mock::new().with_command(|_| Ok(CommandOutput { output: String::new(), success: true, exit_code: 0 }));
    let r = run(&[node("a", "command", "true"), node("b", "rollback", "undo"), node("c", "script", "s")], &[edge("a", "b", None), edge("b", "c", None)], &rt, 0.0);
    let texts: Vec<String> = r.completed().into_iter().map(|c| c.2).collect();
    assert_eq!(texts, ["成功", "回滚成功", "成功"]);
    let specs: Vec<CommandSpec> = rt.commands.borrow().iter().map(|c| c.2).collect();
    assert_eq!(specs, [COMMAND_SPEC, ROLLBACK_SPEC, SCRIPT_SPEC]);
    assert_eq!((SCRIPT_SPEC.ssh_timeout_secs, SCRIPT_SPEC.wait_cap_secs), (60, 180));
}

#[test]
fn nodes_without_config_are_skipped() {
    let rt = Mock::new();
    let r = run(&[node("a", "command", ""), node("b", "rollback", ""), node("c", "transfer", ""), node("d", "mystery", "x")], &[edge("a", "b", None), edge("b", "c", None), edge("c", "d", None)], &rt, 0.0);
    let texts: Vec<String> = r.completed().into_iter().map(|c| c.2).collect();
    assert_eq!(texts, ["跳过(无配置)", "回滚(无配置)", "跳过(无配置)", "mystery"]);
    assert!(rt.commands.borrow().is_empty());
}

#[test]
fn condition_nodes_choose_between_true_and_false_branches() {
    let nodes = [node("first", "command", "check"), node("cond", "condition", "{{lastResult.exitCode}} === 0"), node("yes", "command", "on-true"), node("no", "command", "on-false"), node("always", "command", "any")];
    let edges = [edge("first", "cond", None), edge("cond", "yes", Some("true")), edge("cond", "no", Some("false")), edge("cond", "always", None)];
    let rt = Mock::new();
    let r = run(&nodes, &edges, &rt, 0.0);
    assert_eq!(rt.ran(), ["check", "on-true", "any"]);
    assert!(r.logs().contains(&"条件节点「node-cond」评估结果: 是 (true)".to_string()));

    let failing = Mock::new().with_command(|c| Ok(CommandOutput { output: "x".into(), success: c != "check", exit_code: if c == "check" { 1 } else { 0 } }));
    let r = run(&nodes, &edges, &failing, 0.0);
    assert_eq!(failing.ran(), ["check", "on-false", "any"]);
    let cond = r.completed().into_iter().find(|c| c.0 == "cond").unwrap();
    assert!(!cond.1, "a false condition is reported as a failed node");
    assert!(cond.2.starts_with("条件不满足: 1 === 0"));
    assert!(r.logs().iter().any(|l| l.contains("否 (false)")));
}

#[test]
fn a_node_behind_a_filtered_branch_waits_forever_like_the_original() {
    // `join` has two inputs; the filtered edge never decrements its in-degree, so it never becomes ready.
    let nodes = [node("cond", "condition", "false"), node("t", "command", "t"), node("f", "command", "f"), node("join", "command", "join")];
    let edges = [edge("cond", "t", Some("true")), edge("cond", "f", Some("false")), edge("t", "join", None), edge("f", "join", None)];
    let rt = Mock::new();
    run(&nodes, &edges, &rt, 0.0);
    assert_eq!(rt.ran(), ["f"]);
}

#[test]
fn switch_nodes_follow_the_matching_case_default_or_first() {
    let cfg = r#"{"expression":"{{lastResult.output}}","cases":[{"label":"A","value":"alpha"},{"label":"其它","value":"*"},{"label":"B","value":"beta"}]}"#;
    let nodes = [node("src", "command", "get"), node("sw", "switch", cfg), node("a", "command", "to-a"), node("d", "command", "to-default"), node("b", "command", "to-b")];
    let edges = [edge("src", "sw", None), edge("sw", "a", Some("case-0")), edge("sw", "d", Some("case-1")), edge("sw", "b", Some("case-2"))];
    let with = |out: &'static str| {
        let rt = Mock::new().with_command(move |c| Ok(CommandOutput { output: if c == "get" { out.into() } else { "x".into() }, success: true, exit_code: 0 }));
        let r = run(&nodes, &edges, &rt, 0.0);
        (rt.ran(), r)
    };
    assert_eq!(with("beta").0, ["get", "to-b"]);
    assert_eq!(with("alpha").0, ["get", "to-a"]);
    assert_eq!(with("zzz").0, ["get", "to-default"], "the * case catches everything else");
    let (_, r) = with("beta");
    let sw = r.completed().into_iter().find(|c| c.0 == "sw").unwrap();
    assert_eq!(sw.2, "匹配分支「B」: {{lastResult.output}} → beta");
    assert!(r.logs().contains(&"多分支节点「node-sw」匹配分支: case-2".to_string()));

    // No default: the first case.
    let no_default = r#"{"expression":"q","cases":[{"label":"x","value":"1"},{"label":"y","value":"2"}]}"#;
    let rt = Mock::new();
    run(&[node("sw", "switch", no_default), node("a", "command", "A"), node("b", "command", "B")], &[edge("sw", "a", Some("case-0")), edge("sw", "b", Some("case-1"))], &rt, 0.0);
    assert_eq!(rt.ran(), ["A"]);
    // Broken or empty switch config → case-0.
    for bad in ["", "{oops", r#"{"cases":[]}"#] {
        let rt = Mock::new();
        run(&[node("sw", "switch", bad), node("a", "command", "A")], &[edge("sw", "a", Some("case-0"))], &rt, 0.0);
        assert_eq!(rt.ran(), ["A"], "{bad:?}");
    }
}

#[test]
fn delay_uses_the_configured_seconds_or_five() {
    let rt = Mock::new();
    let r = run(&[node("a", "delay", "3"), node("b", "delay", ""), node("c", "delay", "abc"), node("d", "delay", "0"), node("e", "delay", "12 seconds")], &[edge("a", "b", None), edge("b", "c", None), edge("c", "d", None), edge("d", "e", None)], &rt, 0.0);
    assert_eq!(*rt.sleeps.borrow(), [3, 5, 5, 5, 12].map(Duration::from_secs));
    assert_eq!(r.completed()[0].2, "等待3s");
    assert_eq!(r.completed()[4].2, "等待12s");
}

#[test]
fn confirmation_continues_or_terminates() {
    let nodes = [node("a", "command", "before"), node("ok", "confirm", "上线 {{lastResult.output}}?"), node("b", "command", "after")];
    let edges = [edge("a", "ok", None), edge("ok", "b", None)];
    let rt = Mock::new();
    let r = run(&nodes, &edges, &rt, 0.0);
    assert_eq!(rt.confirms.borrow()[0], ("node-ok".to_string(), "上线 [web] ran before?".to_string()));
    assert_eq!(rt.ran(), ["before", "after"]);
    assert_eq!(r.completed()[1].2, "已确认");

    let denied = Mock::new();
    denied.answer_confirm.set(false);
    let r = run(&nodes, &edges, &denied, 0.0);
    assert_eq!(denied.ran(), ["before"], "terminated before the next node");
    assert_eq!(r.completed()[1], ("ok".into(), false, "用户取消".into()));
    assert!(r.logs().contains(&"工作流已被用户终止".to_string()));
    assert_eq!(r.events.last(), Some(&ProgressEvent::Done));
    assert!(!r.logs().contains(&"工作流执行完成".to_string()));

    // Default text.
    let rt = Mock::new();
    run(&[node("c", "confirm", "")], &[], &rt, 0.0);
    assert_eq!(rt.confirms.borrow()[0].1, "请确认是否继续执行");
}

#[test]
fn transfers_run_per_path_and_download_takes_the_first() {
    let up = r#"{"direction":"upload","localPaths":["/a","","/b"],"localPath":"/ignored","remotePath":"/tmp/"}"#;
    let rt = Mock::new();
    let r = run(&[node("t", "transfer", up)], &[], &rt, 0.0);
    assert_eq!(*rt.transfers.borrow(), [("upload".into(), "/a".into(), "/tmp/".into(), 120), ("upload".into(), "/b".into(), "/tmp/".into(), 120)]);
    assert_eq!(r.completed()[0].2, "传输成功(2项)");

    let down = r#"{"direction":"download","localPaths":["/x","/y"],"remotePath":"/var/log/a.log"}"#;
    let rt = Mock::new();
    let r = run(&[node("t", "transfer", down)], &[], &rt, 0.0);
    assert_eq!(rt.transfers.borrow().len(), 1);
    assert_eq!(r.completed()[0].2, "传输成功");

    // Legacy single path, direction defaults to upload, templates resolve inside the config.
    let rt = Mock::new();
    run(&[node("c", "command", "x"), node("t", "transfer", r#"{"localPath":"/f","remotePath":"/{{lastResult.exitCode}}/"}"#)], &[edge("c", "t", None)], &rt, 0.0);
    assert_eq!(rt.transfers.borrow()[0], ("upload".into(), "/f".into(), "/0/".into(), 120));

    for (cfg, msg) in [(r#"{"direction":"upload","remotePath":"/x"}"#, "本地路径为空"), ("not json", "expected")] {
        let rt = Mock::new();
        let r = run(&[node("t", "transfer", cfg)], &[], &rt, 0.0);
        let (_, ok, out) = r.completed().remove(0);
        assert!(!ok && out.contains(msg), "{out}");
    }
    let failing = Mock::new();
    *failing.transfer_error.borrow_mut() = Some("disk full".into());
    let r = run(&[node("t", "transfer", up)], &[], &failing, 0.0);
    assert_eq!(r.completed()[0], ("t".into(), false, "disk full".into()));
    assert_eq!(failing.transfers.borrow().len(), 1, "stops at the first failing path");
}

#[test]
fn the_default_interval_separates_nodes_but_not_the_first() {
    let rt = Mock::new();
    let r = run(&[node("a", "command", "1"), node("b", "command", "2"), node("c", "command", "3")], &[edge("a", "b", None), edge("b", "c", None)], &rt, 1.5);
    assert_eq!(*rt.sleeps.borrow(), [Duration::from_millis(1500), Duration::from_millis(1500)]);
    assert!(r.logs().contains(&"等待 1.5s 后执行下一个节点".to_string()));
}

#[test]
fn disabled_nodes_and_their_edges_are_ignored() {
    let mut off = node("off", "command", "never");
    off.enabled = false;
    let rt = Mock::new();
    let r = run(&[node("a", "command", "A"), off, node("b", "command", "B")], &[edge("a", "off", None), edge("off", "b", None)], &rt, 0.0);
    assert_eq!(rt.ran(), ["A", "B"], "b only had a disabled parent, so it is a root");
    assert_eq!(r.events[0], ProgressEvent::Log("开始执行工作流（2 个步骤）".into()));

    let mut all_off = node("x", "command", "x");
    all_off.enabled = false;
    let r = run(&[all_off], &[], &Mock::new(), 0.0);
    assert_eq!(r.events, vec![ProgressEvent::Log("工作流没有启用的步骤".into())], "no Done event in this case");
}

#[test]
fn a_stop_request_ends_the_run_between_nodes() {
    let rt = Mock::new();
    rt.cancel_after_commands.set(Some(1));
    let r = run(&[node("a", "command", "A"), node("b", "command", "B")], &[edge("a", "b", None)], &rt, 0.0);
    assert_eq!(rt.ran(), ["A"]);
    assert!(r.logs().contains(&"工作流已被用户终止".to_string()));
    assert_eq!(r.events.last(), Some(&ProgressEvent::Done));
}

#[test]
fn log_entries_summarise_progress() {
    let labels = LogLabels { start_execute: "开始".into(), execute_success: "成功".into(), execute_fail: "失败".into(), stage_info: Box::new(|l, t, c| format!("阶段 {l}/{t}（{c}）")) };
    let e = |ev: ProgressEvent| create_log_entry(&ev, &labels, 5);
    assert_eq!(e(ProgressEvent::Log("hi".into())).unwrap().status, Some(LogStatus::Info));
    let start = e(ProgressEvent::NodeStart { node_id: "1".into(), node_name: "n".into() }).unwrap();
    assert_eq!((start.message.as_str(), start.status, start.node_name.as_deref()), ("开始", Some(LogStatus::Running), Some("n")));
    let ok = e(ProgressEvent::NodeComplete { node_id: "1".into(), node_name: "n".into(), success: true, output: String::new(), exit_code: 0 }).unwrap();
    assert_eq!((ok.message.as_str(), ok.status), ("成功", Some(LogStatus::Success)));
    let bad = e(ProgressEvent::NodeComplete { node_id: "1".into(), node_name: "n".into(), success: false, output: String::new(), exit_code: 1 }).unwrap();
    assert_eq!(bad.message, "失败");
    let long = e(ProgressEvent::NodeComplete { node_id: "1".into(), node_name: "n".into(), success: true, output: "x".repeat(250), exit_code: 0 }).unwrap();
    assert_eq!(long.message.chars().count(), 203);
    assert!(long.message.ends_with("..."));
    assert_eq!(e(ProgressEvent::LevelStart { level: 2, total: 5, count: 3 }).unwrap().message, "阶段 2/5（3）");
    assert!(e(ProgressEvent::Done).is_none());
}
