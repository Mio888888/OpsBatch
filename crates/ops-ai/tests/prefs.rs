//! AI preferences against a real (temporary) database and local mock servers.

// The lock serialises the tests that share one database directory, so it is held for the whole test on purpose.
#![allow(clippy::await_holding_lock, clippy::field_reassign_with_default)]

use ops_ai::client::{
    ai_assess_action, apply_prefs_policy, get_ai_config, save_ai_config, AiActionDecision, AiConfig,
};
use ops_ai::prefs::provider::{self, ProbeHealth};
use ops_ai::prefs::safety::{self, PermissionGrantRule};
use ops_ai::prefs::search::{self, WebSearchConfig};
use ops_ai::prefs::{
    legacy_config_for, load_prefs, runtime_limits, save_prefs, AiPrefs, PermissionMode,
    RuntimeLimits,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// The keychain facade is one process-wide map, so tests that save preferences must not interleave.
static KEYCHAIN: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn db(name: &str) -> (ops_core::Core, std::sync::MutexGuard<'static, ()>) {
    let guard = KEYCHAIN.lock().unwrap_or_else(|e| e.into_inner());
    (ops_core::Core::open_temp(name).unwrap(), guard)
}

#[test]
fn a_fresh_database_yields_the_defaults() {
    let (core, _guard) = db("prefs-default");
    let prefs = load_prefs(&core.db).unwrap();
    assert_eq!(prefs, {
        let mut d = AiPrefs::default();
        d.normalize();
        d
    });
    assert_eq!(prefs.global_permission_mode, PermissionMode::Confirm);
    assert_eq!(prefs.default_agent_id, "catty");
    assert!(!prefs.command_blocklist.is_empty());
    assert_eq!(
        (
            prefs.command_timeout,
            prefs.response_idle_timeout,
            prefs.max_iterations
        ),
        (60, 120, 20)
    );
}

#[test]
fn preferences_round_trip_and_secrets_stay_out_of_the_json() {
    let (core, _guard) = db("prefs-roundtrip");
    let mut prefs = AiPrefs::default();
    let mut p = provider::new_provider("deepseek");
    p.api_key = Some("sk-secret-123456".into());
    p.custom_headers.insert("X-Org".into(), "org-secret".into());
    p.default_model = Some("deepseek-chat".into());
    let id = p.id.clone();
    prefs.providers.push(p);
    prefs.web_search = Some(WebSearchConfig {
        provider_id: "tavily".into(),
        api_key: Some("tvly-secret".into()),
        enabled: true,
        ..Default::default()
    });
    prefs.external_mcp.http.token = "tok-secret".into();
    prefs.global_permission_mode = PermissionMode::Auto;
    prefs.command_timeout = 90;
    save_prefs(&core.db, &prefs).unwrap();

    let raw: String = core
        .db
        .pool
        .get()
        .unwrap()
        .query_row("SELECT value FROM settings WHERE key='ai_prefs'", [], |r| {
            r.get(0)
        })
        .unwrap();
    for secret in [
        "sk-secret-123456",
        "org-secret",
        "tvly-secret",
        "tok-secret",
    ] {
        assert!(
            !raw.contains(secret),
            "{secret} leaked into the settings table"
        );
    }
    assert!(raw.contains("\"globalPermissionMode\":\"auto\""));

    let loaded = load_prefs(&core.db).unwrap();
    let p = loaded.providers.iter().find(|p| p.id == id).unwrap();
    assert_eq!(p.api_key.as_deref(), Some("sk-secret-123456"));
    assert_eq!(
        p.custom_headers.get("X-Org").map(String::as_str),
        Some("org-secret")
    );
    assert_eq!(
        loaded.web_search.unwrap().api_key.as_deref(),
        Some("tvly-secret")
    );
    assert_eq!(loaded.external_mcp.http.token, "tok-secret");
    assert_eq!(
        (loaded.global_permission_mode, loaded.command_timeout),
        (PermissionMode::Auto, 90)
    );
}

#[test]
fn clearing_a_key_removes_it() {
    let (core, _guard) = db("prefs-clear-key");
    let mut prefs = AiPrefs::default();
    let mut p = provider::new_provider("openai");
    p.api_key = Some("sk-one".into());
    let id = p.id.clone();
    prefs.providers.push(p);
    save_prefs(&core.db, &prefs).unwrap();
    prefs.provider_mut(&id).unwrap().api_key = None;
    save_prefs(&core.db, &prefs).unwrap();
    assert_eq!(load_prefs(&core.db).unwrap().providers[0].api_key, None);
}

#[tokio::test]
async fn the_enabled_provider_is_mirrored_into_the_legacy_ai_config() {
    let (core, _guard) = db("prefs-legacy");
    let mut prefs = AiPrefs::default();
    let mut a = provider::new_provider("openai");
    a.api_key = Some("sk-aaaaaaaaaaaa".into());
    let mut b = provider::new_provider("ollama");
    b.default_model = Some("qwen3:8b".into());
    let (a_id, b_id) = (a.id.clone(), b.id.clone());
    prefs.providers.extend([a, b]);
    prefs.provider_mut(&a_id).unwrap().default_model = Some("gpt-4o".into());
    prefs.set_provider_enabled(&a_id, true);
    save_prefs(&core.db, &prefs).unwrap();

    let legacy = get_ai_config(&core.db).await.unwrap();
    assert_eq!(
        (
            legacy.provider.as_str(),
            legacy.model.as_str(),
            legacy.enabled
        ),
        ("openai", "gpt-4o", true)
    );
    assert_eq!(legacy.api_url, "https://api.openai.com/v1");
    assert!(legacy.api_key.contains("****"));

    // enabling another provider disables the first one
    prefs.set_provider_enabled(&b_id, true);
    assert!(
        !prefs
            .providers
            .iter()
            .find(|p| p.id == a_id)
            .unwrap()
            .enabled
    );
    save_prefs(&core.db, &prefs).unwrap();
    let legacy = get_ai_config(&core.db).await.unwrap();
    assert_eq!(
        (legacy.provider.as_str(), legacy.model.as_str()),
        ("ollama", "qwen3:8b")
    );

    // nothing active: the assistant is switched off, the provider stays listed
    prefs.set_provider_enabled(&b_id, false);
    save_prefs(&core.db, &prefs).unwrap();
    assert!(!get_ai_config(&core.db).await.unwrap().enabled);
    assert_eq!(load_prefs(&core.db).unwrap().providers.len(), 2);
}

#[tokio::test]
async fn an_existing_single_provider_config_is_adopted() {
    let (core, _guard) = db("prefs-migrate");
    save_ai_config(
        &core.db,
        AiConfig {
            provider: "openai".into(),
            api_url: "https://x.test/v1".into(),
            api_key: "sk-legacy-123456".into(),
            model: "gpt-x".into(),
            enabled: true,
        },
    )
    .await
    .unwrap();
    let prefs = load_prefs(&core.db).unwrap();
    assert_eq!(prefs.providers.len(), 1);
    let p = &prefs.providers[0];
    assert_eq!(
        (
            p.provider_id.as_str(),
            p.base_url.as_deref(),
            p.api_key.as_deref(),
            p.enabled
        ),
        (
            "openai",
            Some("https://x.test/v1"),
            Some("sk-legacy-123456"),
            true
        )
    );
    assert_eq!(
        (
            prefs.active_provider_id.as_str(),
            prefs.active_model_id.as_str()
        ),
        (p.id.as_str(), "gpt-x")
    );
    assert_eq!(legacy_config_for(&prefs).unwrap().model, "gpt-x");
}

#[test]
fn removing_a_provider_clears_every_reference() {
    let mut prefs = AiPrefs::default();
    let p = provider::new_provider("openai");
    let id = p.id.clone();
    prefs.providers.push(p);
    prefs.set_provider_enabled(&id, true);
    prefs
        .agent_provider_map
        .insert("discovered_codex".into(), id.clone());
    prefs
        .agent_model_map
        .insert("discovered_codex".into(), "m".into());
    prefs.agent_model_map.insert("other".into(), "keep".into());
    prefs.remove_provider(&id);
    assert!(prefs.providers.is_empty());
    assert_eq!(
        (
            prefs.active_provider_id.as_str(),
            prefs.active_model_id.as_str()
        ),
        ("", "")
    );
    assert!(prefs.agent_provider_map.is_empty());
    assert_eq!(
        prefs.agent_model_map.keys().collect::<Vec<_>>(),
        vec!["other"]
    );
}

#[test]
fn out_of_range_values_are_normalised_on_save() {
    let (core, _guard) = db("prefs-normalise");
    let mut prefs = AiPrefs::default();
    prefs.command_timeout = 0;
    prefs.max_iterations = 9999;
    prefs.default_agent_id = "  ".into();
    save_prefs(&core.db, &prefs).unwrap();
    let loaded = load_prefs(&core.db).unwrap();
    assert_eq!(
        (
            loaded.command_timeout,
            loaded.max_iterations,
            loaded.default_agent_id.as_str()
        ),
        (1, 100, "catty")
    );
}

#[test]
fn the_runtime_reads_limits_without_touching_secrets() {
    let (core, _guard) = db("prefs-runtime");
    assert_eq!(
        runtime_limits(&core.db).permission_mode,
        PermissionMode::Confirm
    );
    let mut prefs = AiPrefs::default();
    prefs.global_permission_mode = PermissionMode::Observer;
    prefs.response_idle_timeout = 30;
    save_prefs(&core.db, &prefs).unwrap();
    let limits = runtime_limits(&core.db);
    assert_eq!(
        (limits.permission_mode, limits.response_idle_timeout),
        (PermissionMode::Observer, 30)
    );
}

#[test]
fn assessment_honours_observer_mode_blocklist_and_grants() {
    let (core, _guard) = db("prefs-policy");
    // defaults: a harmless command waits for confirmation
    let plain = ai_assess_action(&core.db, "uptime".into()).unwrap();
    assert_eq!(plain.decision, AiActionDecision::Confirm);

    let mut limits = RuntimeLimits::default();
    limits.permission_mode = PermissionMode::Observer;
    let observer = apply_prefs_policy(plain.clone(), "uptime", &limits);
    assert_eq!(
        (observer.decision, observer.matched_rule.as_str()),
        (AiActionDecision::Block, "AI_OBSERVER_MODE")
    );

    let mut limits = RuntimeLimits::default();
    limits.command_blocklist = vec!["^cat /etc/shadow".into()];
    let blocked = apply_prefs_policy(plain.clone(), "cat /etc/shadow", &limits);
    assert_eq!(
        (blocked.decision, blocked.matched_rule.as_str()),
        (AiActionDecision::Block, "AI_COMMAND_BLOCKLIST")
    );
    assert!(blocked.reason.contains("^cat /etc/shadow"));
    assert_eq!(
        apply_prefs_policy(plain.clone(), "uptime", &limits).decision,
        AiActionDecision::Confirm
    );

    let mut limits = RuntimeLimits::default();
    limits.permission_grants = vec![PermissionGrantRule {
        id: "g".into(),
        capability_id: "terminal.execute".into(),
        session_pattern: "*".into(),
        command_pattern: Some("uptime".into()),
        ..Default::default()
    }];
    let granted = apply_prefs_policy(plain.clone(), "uptime", &limits);
    assert_eq!(
        (granted.decision, granted.matched_rule.as_str()),
        (AiActionDecision::Safe, "PERMISSION_GRANT")
    );
    assert_eq!(
        apply_prefs_policy(plain, "uptime -p", &limits).decision,
        AiActionDecision::Confirm
    );

    // built-in blocks and riskier confirmations are never lifted by a grant
    let rm = ai_assess_action(&core.db, "rm -rf /".into()).unwrap();
    assert_eq!(
        apply_prefs_policy(rm, "rm -rf /", &limits).decision,
        AiActionDecision::Block
    );
    let svc = ai_assess_action(&core.db, "systemctl restart nginx".into()).unwrap();
    limits.permission_grants[0].command_pattern = Some("systemctl *".into());
    assert_eq!(
        apply_prefs_policy(svc, "systemctl restart nginx", &limits).decision,
        AiActionDecision::Confirm
    );
}

#[test]
fn the_default_blocklist_reaches_assessment_through_the_database() {
    let (core, _guard) = db("prefs-blocklist-db");
    let mut prefs = AiPrefs::default();
    save_prefs(&core.db, &prefs).unwrap();
    assert_eq!(
        ai_assess_action(&core.db, "echo hello | base64 | sh".into())
            .unwrap()
            .decision,
        AiActionDecision::Block
    );
    prefs.command_blocklist.clear();
    save_prefs(&core.db, &prefs).unwrap();
    assert_ne!(
        ai_assess_action(&core.db, "echo hello | base64 | sh".into())
            .unwrap()
            .decision,
        AiActionDecision::Block
    );
    let _ = safety::DEFAULT_MAX_ITERATIONS;
}

// ---------------------------------------------------------------------------
// Mock HTTP servers
// ---------------------------------------------------------------------------

/// Serves `responses` in order to successive connections and records `(request line + headers)`.
async fn serve(
    responses: Vec<(u16, String)>,
) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        for (status, body) in responses {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let mut buf = vec![0u8; 16384];
            let mut len = 0;
            loop {
                let n = sock.read(&mut buf[len..]).await.unwrap_or(0);
                len += n;
                let text = String::from_utf8_lossy(&buf[..len]).to_string();
                if n == 0 || text.contains("\r\n\r\n") && body_complete(&text) {
                    log.lock().unwrap().push(text);
                    break;
                }
            }
            let reply = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let _ = sock.write_all(reply.as_bytes()).await;
        }
    });
    (base, seen)
}

fn body_complete(text: &str) -> bool {
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text, ""));
    let len = head
        .to_lowercase()
        .lines()
        .find_map(|l| {
            l.strip_prefix("content-length:")
                .and_then(|v| v.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    body.len() >= len
}

#[tokio::test]
async fn discovery_sends_the_style_specific_credentials() {
    let (base, seen) = serve(vec![
        (
            200,
            r#"{"data":[{"id":"gpt-b"},{"id":"gpt-a","context_length":128000}]}"#.into(),
        ),
        (200, r#"{"data":[{"id":"claude-x"}]}"#.into()),
        (200, r#"{"models":[{"name":"models/gemini-pro"}]}"#.into()),
    ])
    .await;
    let mut p = provider::new_provider("openai");
    p.base_url = Some(format!("{base}/v1"));
    p.api_key = Some("sk-test".into());
    p.custom_headers.insert("X-Extra".into(), "yes".into());
    let found = provider::discover_models(&p).await.unwrap();
    assert_eq!(found.ids, vec!["gpt-a", "gpt-b"]);
    assert_eq!(found.context_windows.get("gpt-a"), Some(&128000));

    let mut a = provider::new_provider("anthropic");
    a.base_url = Some(base.clone());
    a.api_key = Some("ant-key".into());
    assert_eq!(
        provider::discover_models(&a).await.unwrap().ids,
        vec!["claude-x"]
    );

    let mut g = provider::new_provider("google");
    g.base_url = Some(format!("{base}/v1beta"));
    g.api_key = Some("g-key".into());
    assert_eq!(
        provider::discover_models(&g).await.unwrap().ids,
        vec!["gemini-pro"]
    );

    let requests = seen.lock().unwrap().clone();
    let lower: Vec<String> = requests.iter().map(|r| r.to_lowercase()).collect();
    assert!(lower[0].starts_with("get /v1/models "));
    assert!(
        lower[0].contains("authorization: bearer sk-test") && lower[0].contains("x-extra: yes")
    );
    assert!(
        lower[1].starts_with("get /v1/models ")
            && lower[1].contains("x-api-key: ant-key")
            && lower[1].contains("anthropic-version")
    );
    assert!(
        lower[2].starts_with("get /v1beta/models ") && lower[2].contains("x-goog-api-key: g-key")
    );
}

#[tokio::test]
async fn discovery_and_probe_report_failures() {
    let (base, _) = serve(vec![
        (401, r#"{"error":"bad key"}"#.into()),
        (200, r#"{"data":[{"id":"m"}]}"#.into()),
        (500, "oops".into()),
    ])
    .await;
    let mut p = provider::new_provider("custom");
    p.base_url = Some(base.clone());
    p.api_key = Some("k".into());
    let err = provider::discover_models(&p).await.err().unwrap();
    assert!(err.contains("401"), "{err}");
    let ok = provider::probe(&p).await;
    assert_eq!(
        (ok.health, ok.model_count, ok.status),
        (ProbeHealth::Ok, Some(1), 200)
    );
    let bad = provider::probe(&p).await;
    assert_eq!((bad.health, bad.status), (ProbeHealth::Error, 500));
    p.api_key = None;
    assert!(provider::discover_models(&p).await.is_err());
    // unreachable endpoint
    p.api_key = Some("k".into());
    p.base_url = Some("http://127.0.0.1:1".into());
    assert_eq!(provider::probe(&p).await.health, ProbeHealth::Error);
}

#[tokio::test]
async fn web_search_hits_the_configured_provider() {
    let (base, seen) = serve(vec![(
        200,
        r#"{"results":[{"title":"Rust","url":"https://rust-lang.org","content":"lang"}]}"#.into(),
    )])
    .await;
    let cfg = WebSearchConfig {
        provider_id: "tavily".into(),
        api_key: Some("tvly-k".into()),
        api_host: Some(base),
        enabled: true,
        max_results: Some(3),
    };
    let rows = search::web_search(&cfg, "rust").await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].url, "https://rust-lang.org");
    let request = seen.lock().unwrap()[0].to_lowercase();
    assert!(
        request.starts_with("post /search ") && request.contains("authorization: bearer tvly-k")
    );
    assert!(request.contains("\"max_results\":3") && request.contains("\"query\":\"rust\""));
    let mut off = cfg.clone();
    off.enabled = false;
    assert!(search::web_search(&off, "rust").await.is_err());
}

// ---------------------------------------------------------------------------
// Privacy protection through the streaming client
// ---------------------------------------------------------------------------

/// A model that repeats the first placeholder it was given, split across SSE chunks, and records the request body.
async fn echo_model() -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let bodies = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = bodies.clone();
    let ip_token = regex::Regex::new(r"\[OB_IP_\d+\]").unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let mut buf = vec![0u8; 65536];
            let mut len = 0;
            loop {
                let n = sock.read(&mut buf[len..]).await.unwrap_or(0);
                len += n;
                let text = String::from_utf8_lossy(&buf[..len]).to_string();
                if n == 0 || text.contains("\r\n\r\n") && body_complete(&text) {
                    break;
                }
            }
            let text = String::from_utf8_lossy(&buf[..len]).to_string();
            let body = text
                .split_once("\r\n\r\n")
                .map(|(_, b)| b.to_string())
                .unwrap_or_default();
            log.lock().unwrap().push(body.clone());
            let token = ip_token
                .find(&body)
                .map(|m| m.as_str().to_string())
                .unwrap_or_else(|| "NONE".into());
            let (head, tail) = token.split_at(token.len() / 2);
            let _ = sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").await;
            for delta in ["ssh to ", head, tail, " now"] {
                let line = format!(
                    "data: {}\n\n",
                    serde_json::json!({"model": "m", "choices": [{"delta": {"content": delta}}]})
                );
                let _ = sock.write_all(line.as_bytes()).await;
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            let _ = sock.write_all(b"data: [DONE]\n\n").await;
        }
    });
    (url, bodies)
}

#[tokio::test(flavor = "multi_thread")]
async fn private_addresses_never_reach_the_model_and_come_back_restored() {
    let (core, _guard) = db("prefs-privacy-stream");
    let (url, bodies) = echo_model().await;
    let mut prefs = AiPrefs::default();
    prefs.privacy.enabled = true;
    prefs.privacy.redact_private_ips = true;
    prefs.privacy.patterns = vec!["INTERNAL_[A-Z]+".into()];
    let mut p = provider::new_provider("ollama");
    p.base_url = Some(url);
    p.default_model = Some("m".into());
    let id = p.id.clone();
    prefs.providers.push(p);
    prefs.set_provider_enabled(&id, true);
    save_prefs(&core.db, &prefs).unwrap();

    let rx = core.events.subscribe("ai-stream-chunk");
    let request: ops_ai::client::StreamChatRequest = serde_json::from_value(serde_json::json!({
        "messages": [{"role": "user", "content": "restart nginx on 10.20.30.40 (INTERNAL_DB)"}],
        "scope": "ssh_host", "scopeId": "h1", "clientRequestId": "r1"
    }))
    .unwrap();
    let res = ops_ai::client::ai_chat_stream(&core, &core.db, request)
        .await
        .unwrap();

    let sent = bodies.lock().unwrap().join("\n");
    assert!(
        !sent.contains("10.20.30.40") && !sent.contains("INTERNAL_DB"),
        "{sent}"
    );
    assert!(
        sent.contains("[OB_IP_1]") && sent.contains("[OB_TEXT_1]"),
        "{sent}"
    );

    let mut text = String::new();
    while let Ok(ev) = rx.try_recv() {
        text.push_str(ev.payload["delta"].as_str().unwrap_or(""));
    }
    assert_eq!(text, "ssh to 10.20.30.40 now");
    let (_, messages) = ops_ai::client::ai_get_conversation(
        &core.db,
        res["conversation_id"].as_str().unwrap().to_string(),
    )
    .await
    .unwrap();
    assert_eq!(messages.last().unwrap().content, "ssh to 10.20.30.40 now");
    // the user's own message is stored as typed
    assert_eq!(
        messages[0].content,
        "restart nginx on 10.20.30.40 (INTERNAL_DB)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_active_providers_headers_and_parameters_go_into_requests() {
    let (core, _guard) = db("prefs-request-options");
    let (base, seen) = serve(vec![
        (
            200,
            r#"{"model":"m","choices":[{"message":{"content":"ok"}}]}"#.into(),
        ),
        (200, r#"{"data":[{"id":"m1"}]}"#.into()),
    ])
    .await;
    let mut prefs = AiPrefs::default();
    let mut p = provider::new_provider("custom");
    p.base_url = Some(format!("{base}/v1"));
    p.api_key = Some("sk-k".into());
    p.default_model = Some("m".into());
    p.custom_headers.insert("X-Org".into(), "org-1".into());
    p.advanced_params = Some(provider::AdvancedParams {
        temperature: Some(0.2),
        max_tokens: Some(777),
        top_p: Some(0.9),
        reasoning_effort: Some("high".into()),
        ..Default::default()
    });
    let id = p.id.clone();
    prefs.providers.push(p);
    prefs.set_provider_enabled(&id, true);
    save_prefs(&core.db, &prefs).unwrap();

    let reply = ops_ai::client::ai_chat(
        &core.db,
        serde_json::from_value(
            serde_json::json!({"messages": [{"role": "user", "content": "hi"}]}),
        )
        .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(reply.content, "ok");
    let models = ops_ai::client::ai_list_models(&core.db, None, None, None)
        .await
        .unwrap();
    assert_eq!(models.len(), 1);

    let requests = seen.lock().unwrap().clone();
    let chat = requests[0].to_lowercase();
    assert!(chat.contains("x-org: org-1"), "{chat}");
    assert!(
        chat.contains("\"temperature\":0.2")
            && chat.contains("\"max_tokens\":777")
            && chat.contains("\"top_p\":0.9")
            && chat.contains("\"reasoning_effort\":\"high\""),
        "{chat}"
    );
    assert!(requests[1].to_lowercase().contains("x-org: org-1"));

    // without an active provider nothing extra is sent
    prefs.set_provider_enabled(&id, false);
    save_prefs(&core.db, &prefs).unwrap();
    assert_eq!(
        ops_ai::prefs::request_options(&core.db),
        ops_ai::prefs::RequestOptions::default()
    );
}
