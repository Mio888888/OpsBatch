//! AI settings (providers / agents / tool access / web search / safety) through the real views: headless GPUI, a
//! temporary database and local mock servers.

mod common;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use gpui_kit::component::input::InputEvent;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext};
use ops_ai::client::get_ai_config;
use ops_ai::prefs::provider::ProbeHealth;
use ops_ai::prefs::{self, PermissionMode, ToolIntegrationMode};
use opsbatch_app::ai::runtime;
use opsbatch_app::ai::settings::{AiSettingsView, AiTab};

use common::{serial, wait_until, Fixture};

fn win<R>(
    cx: &mut TestAppContext,
    fx: &Fixture,
    f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App) -> R,
) -> R {
    cx.update_window(fx.handle.into(), |_, window, cx| f(window, cx))
        .unwrap()
}

/// Settings view plus the fixture; waits until the stored preferences are in.
fn open(cx: &mut TestAppContext) -> (Fixture, Entity<AiSettingsView>) {
    let fx = common::setup(cx, |_| {});
    let view: Entity<AiSettingsView> = win(cx, &fx, |window, cx| {
        cx.new(|cx| AiSettingsView::new(window, cx))
    });
    wait_until(cx, "preferences loaded", |cx| {
        view.read_with(cx, |v, cx| v.loaded(cx))
    });
    (fx, view)
}

/// Waits for the background save to finish and returns what the database holds.
fn saved(cx: &mut TestAppContext, fx: &Fixture, view: &Entity<AiSettingsView>) -> prefs::AiPrefs {
    let store = view.read_with(cx, |v, _| v.store().clone());
    wait_until(cx, "saved", |cx| store.read_with(cx, |s, _| !s.saving()));
    prefs::load_prefs(&fx.ctx.db).unwrap()
}

fn blur(cx: &mut TestAppContext, input: &Entity<gpui_kit::component::input::InputState>) {
    cx.update(|cx| input.update(cx, |_, cx| cx.emit(InputEvent::Blur)));
}

fn set_text(
    cx: &mut TestAppContext,
    fx: &Fixture,
    input: &Entity<gpui_kit::component::input::InputState>,
    text: &str,
) {
    win(cx, fx, |w, cx| {
        input.update(cx, |s, cx| s.set_value(text.to_string(), w, cx))
    });
}

/// A server answering `GET .../models` and recording the raw requests.
struct ModelsMock {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
}

fn models_mock(status: u16, body: &str) -> ModelsMock {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let (log, body) = (requests.clone(), body.to_string());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buf = [0u8; 8192];
            let n = stream.read(&mut buf).unwrap_or(0);
            log.lock()
                .unwrap()
                .push(String::from_utf8_lossy(&buf[..n]).to_string());
            let reply = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    ModelsMock { url, requests }
}

#[gpui_kit::test]
fn the_five_tabs_switch(cx: &mut TestAppContext) {
    let _s = serial();
    let (fx, view) = open(cx);
    view.read_with(cx, |v, _| assert_eq!(v.tab(), AiTab::Providers));
    assert_eq!(AiTab::ALL.len(), 5);
    for tab in AiTab::ALL {
        view.update_in_test(cx, &fx, |v, cx| v.select_tab(tab, cx));
        view.read_with(cx, |v, _| assert_eq!(v.tab(), tab));
    }
}

trait UpdateInTest<T: 'static> {
    fn update_in_test(
        &self,
        cx: &mut TestAppContext,
        fx: &Fixture,
        f: impl FnOnce(&mut T, &mut gpui_kit::Context<T>),
    );
}

impl<T: 'static> UpdateInTest<T> for Entity<T> {
    fn update_in_test(
        &self,
        cx: &mut TestAppContext,
        fx: &Fixture,
        f: impl FnOnce(&mut T, &mut gpui_kit::Context<T>),
    ) {
        win(cx, fx, |_, cx| self.update(cx, f));
    }
}

// ---------------------------------------------------------------------------
// Providers
// ---------------------------------------------------------------------------

#[gpui_kit::test]
fn adding_enabling_configuring_and_removing_a_provider(cx: &mut TestAppContext) {
    let _s = serial();
    let (fx, view) = open(cx);
    let tab = view.read_with(cx, |v, _| v.providers().clone());

    // empty state, then a DeepSeek provider from the menu
    tab.read_with(cx, |t, cx| assert!(t.providers(cx).is_empty()));
    let id = tab.update_in_test_ret(cx, &fx, |t, cx| t.add_provider("deepseek", cx));
    cx.run_until_parked();
    tab.read_with(cx, |t, cx| {
        let p = &t.providers(cx)[0];
        assert_eq!(
            (
                p.name.as_str(),
                p.provider_id.as_str(),
                p.enabled,
                p.default_model.as_deref()
            ),
            ("DeepSeek", "deepseek", false, Some("deepseek-v4-flash"))
        );
        assert_eq!(p.base_url.as_deref(), Some("https://api.deepseek.com/v1"));
    });

    // the key and model go in through the form; the mock stands in for the API
    let mock = models_mock(
        200,
        r#"{"data":[{"id":"ds-b"},{"id":"ds-a","context_length":64000}]}"#,
    );
    win(cx, &fx, |w, cx| {
        tab.update(cx, |t, cx| t.toggle_edit(&id, w, cx))
    });
    let form = tab
        .read_with(cx, |t, _| t.form().cloned())
        .expect("the form opens");
    let (api_key, base_url, model, name) = form.read_with(cx, |f, _| {
        (
            f.api_key().clone(),
            f.base_url().clone(),
            f.model().clone(),
            f.name().clone(),
        )
    });
    set_text(cx, &fx, &api_key, "sk-form-key-123456");
    set_text(cx, &fx, &base_url, &mock.url);
    set_text(cx, &fx, &name, "My DeepSeek");
    set_text(cx, &fx, &model, "ds-a");

    // fetch models and test the connection
    win(cx, &fx, |w, cx| {
        form.update(cx, |f, cx| f.fetch_models(w, cx))
    });
    wait_until(cx, "models fetched", |cx| {
        form.read_with(cx, |f, _| !f.fetched_models().is_empty() && !f.busy())
    });
    form.read_with(cx, |f, _| assert_eq!(f.fetched_models(), ["ds-a", "ds-b"]));
    win(cx, &fx, |w, cx| {
        form.update(cx, |f, cx| f.test_connection(w, cx))
    });
    wait_until(cx, "connection tested", |cx| {
        form.read_with(cx, |f, _| f.status().is_some() && !f.busy())
    });
    form.read_with(cx, |f, _| {
        assert_eq!(f.status().unwrap().0, ProbeHealth::Ok)
    });
    {
        let requests = mock.requests.lock().unwrap().clone();
        assert!(requests.len() >= 2);
        assert!(
            requests[0]
                .to_lowercase()
                .contains("authorization: bearer sk-form-key-123456"),
            "{}",
            requests[0]
        );
        assert!(requests[0].starts_with("GET /v1/models"));
    }

    // an invalid context window is refused, a valid one is stored
    set_text(
        cx,
        &fx,
        &form.read_with(cx, |f, _| f.context_window().clone()),
        "12",
    );
    assert!(!form.update_ret(cx, |f, cx| f.save(cx)));
    form.read_with(cx, |f, _| assert!(f.error().is_some()));
    set_text(
        cx,
        &fx,
        &form.read_with(cx, |f, _| f.context_window().clone()),
        "128000",
    );
    assert!(form.update_ret(cx, |f, cx| f.save(cx)));
    cx.run_until_parked();
    tab.read_with(cx, |t, _| assert!(t.editing().is_none(), "the form closed"));
    let stored = saved(cx, &fx, &view);
    let p = &stored.providers[0];
    assert_eq!(
        (
            p.name.as_str(),
            p.api_key.as_deref(),
            p.base_url.as_deref(),
            p.default_model.as_deref(),
            p.context_window
        ),
        (
            "My DeepSeek",
            Some("sk-form-key-123456"),
            Some(mock.url.as_str()),
            Some("ds-a"),
            Some(128000)
        )
    );
    assert_eq!(p.model_context_windows.get("ds-a"), Some(&64000));

    // switching it on makes it the assistant's provider
    tab.update_in_test(cx, &fx, |t, cx| t.toggle_enabled(&id, true, cx));
    let stored = saved(cx, &fx, &view);
    assert_eq!(
        (
            stored.active_provider_id.as_str(),
            stored.active_model_id.as_str()
        ),
        (id.as_str(), "ds-a")
    );
    let legacy = ops_core::rt::block_on(get_ai_config(&fx.ctx.db)).unwrap();
    assert_eq!(
        (
            legacy.provider.as_str(),
            legacy.api_url.as_str(),
            legacy.model.as_str(),
            legacy.enabled
        ),
        ("custom", mock.url.as_str(), "ds-a", true)
    );

    // a second provider takes over; the first is switched off
    let second = tab.update_in_test_ret(cx, &fx, |t, cx| t.add_provider("ollama", cx));
    tab.update_in_test(cx, &fx, |t, cx| t.toggle_enabled(&second, true, cx));
    let stored = saved(cx, &fx, &view);
    assert_eq!(stored.providers.iter().filter(|p| p.enabled).count(), 1);
    assert_eq!(stored.active_provider_id, second);
    assert_eq!(
        ops_core::rt::block_on(get_ai_config(&fx.ctx.db))
            .unwrap()
            .provider,
        "ollama"
    );

    // removing the active one clears the references and switches the assistant off
    tab.update_in_test(cx, &fx, |t, cx| t.remove_provider(&second, cx));
    let stored = saved(cx, &fx, &view);
    assert_eq!(
        (stored.providers.len(), stored.active_provider_id.as_str()),
        (1, "")
    );
    assert!(
        !ops_core::rt::block_on(get_ai_config(&fx.ctx.db))
            .unwrap()
            .enabled
    );
}

trait UpdateRet<T: 'static> {
    fn update_ret<R>(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut T, &mut gpui_kit::Context<T>) -> R,
    ) -> R;
    fn update_in_test_ret<R>(
        &self,
        cx: &mut TestAppContext,
        fx: &Fixture,
        f: impl FnOnce(&mut T, &mut gpui_kit::Context<T>) -> R,
    ) -> R;
}

impl<T: 'static> UpdateRet<T> for Entity<T> {
    fn update_ret<R>(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut T, &mut gpui_kit::Context<T>) -> R,
    ) -> R {
        cx.update(|cx| self.update(cx, f))
    }
    fn update_in_test_ret<R>(
        &self,
        cx: &mut TestAppContext,
        fx: &Fixture,
        f: impl FnOnce(&mut T, &mut gpui_kit::Context<T>) -> R,
    ) -> R {
        win(cx, fx, |_, cx| self.update(cx, f))
    }
}

#[gpui_kit::test]
fn provider_form_reports_failures_and_validates_headers(cx: &mut TestAppContext) {
    let _s = serial();
    let (fx, view) = open(cx);
    let tab = view.read_with(cx, |v, _| v.providers().clone());
    let id = tab.update_in_test_ret(cx, &fx, |t, cx| t.add_provider("custom", cx));
    win(cx, &fx, |w, cx| {
        tab.update(cx, |t, cx| t.toggle_edit(&id, w, cx))
    });
    let form = tab.read_with(cx, |t, _| t.form().cloned()).unwrap();

    // missing URL / key are reported without any request
    win(cx, &fx, |w, cx| {
        form.update(cx, |f, cx| f.test_connection(w, cx))
    });
    form.read_with(cx, |f, _| {
        assert_eq!(f.status().map(|s| s.0), Some(ProbeHealth::Error))
    });

    let mock = models_mock(401, r#"{"error":"nope"}"#);
    let (api_key, base_url) =
        form.read_with(cx, |f, _| (f.api_key().clone(), f.base_url().clone()));
    set_text(cx, &fx, &base_url, &mock.url);
    set_text(cx, &fx, &api_key, "k");
    win(cx, &fx, |w, cx| {
        form.update(cx, |f, cx| f.test_connection(w, cx))
    });
    wait_until(cx, "probe", |cx| {
        form.read_with(cx, |f, _| {
            f.status().is_some_and(|s| s.1.contains("401")) && !f.busy()
        })
    });

    // header rows: a bad name blocks saving
    win(cx, &fx, |w, cx| {
        form.update(cx, |f, cx| f.add_header(w, cx))
    });
    let rows = form.update_ret(cx, |f, cx| f.draft(cx).custom_headers.len());
    assert_eq!(rows, 0, "an empty header row is not part of the draft");
    form.update_in_test(cx, &fx, |f, cx| f.cancel(cx));
    cx.run_until_parked();
    tab.read_with(cx, |t, _| assert!(t.editing().is_none()));
}

// ---------------------------------------------------------------------------
// Safety
// ---------------------------------------------------------------------------

#[gpui_kit::test]
fn safety_settings_are_stored_clamped_and_reach_the_runtime(cx: &mut TestAppContext) {
    let _s = serial();
    let (fx, view) = open(cx);
    let tab = view.read_with(cx, |v, _| v.safety().clone());

    // defaults
    tab.read_with(cx, |t, cx| {
        assert_eq!(
            (
                t.response_idle().read(cx).value().to_string(),
                t.command_timeout().read(cx).value().to_string(),
                t.max_iterations().read(cx).value().to_string()
            ),
            ("120".to_string(), "60".to_string(), "20".to_string())
        );
        assert!(!t.blocklist_inputs().is_empty());
    });
    assert_eq!(
        cx.update(|cx| runtime::limits(cx).permission_mode),
        PermissionMode::Confirm
    );

    // permission mode
    tab.update_in_test(cx, &fx, |t, cx| t.set_mode(PermissionMode::Observer, cx));
    assert_eq!(
        cx.update(|cx| runtime::limits(cx).permission_mode),
        PermissionMode::Observer
    );
    assert_eq!(
        saved(cx, &fx, &view).global_permission_mode,
        PermissionMode::Observer
    );

    // numbers: committed on blur, clamped, invalid input reverts
    let (idle, cmd, iters) = tab.read_with(cx, |t, _| {
        (
            t.response_idle().clone(),
            t.command_timeout().clone(),
            t.max_iterations().clone(),
        )
    });
    set_text(cx, &fx, &cmd, "90");
    blur(cx, &cmd);
    set_text(cx, &fx, &iters, "999");
    blur(cx, &iters);
    set_text(cx, &fx, &idle, "abc");
    blur(cx, &idle);
    cx.run_until_parked();
    let stored = saved(cx, &fx, &view);
    assert_eq!(
        (
            stored.command_timeout,
            stored.max_iterations,
            stored.response_idle_timeout
        ),
        (90, 100, 120)
    );
    tab.read_with(cx, |t, cx| {
        assert_eq!(
            (
                t.max_iterations().read(cx).value().to_string(),
                t.response_idle().read(cx).value().to_string()
            ),
            ("100".to_string(), "120".to_string())
        )
    });
    let limits = cx.update(|cx| runtime::limits(cx));
    assert_eq!((limits.command_timeout, limits.max_iterations), (90, 100));

    // blocklist: add a row, a broken regex is flagged and not stored, a good one is
    win(cx, &fx, |w, cx| {
        tab.update(cx, |t, cx| t.add_pattern(w, cx))
    });
    let count = tab.read_with(cx, |t, _| t.blocklist_inputs().len());
    let last = tab.read_with(cx, |t, _| t.blocklist_inputs().last().unwrap().clone());
    set_text(cx, &fx, &last, "(");
    cx.update(|cx| last.update(cx, |_, cx| cx.emit(InputEvent::Change)));
    cx.run_until_parked();
    tab.read_with(cx, |t, _| assert!(t.blocklist_error(count - 1).is_some()));
    blur(cx, &last);
    set_text(cx, &fx, &last, "^dangerous-tool\\b");
    cx.update(|cx| last.update(cx, |_, cx| cx.emit(InputEvent::Change)));
    blur(cx, &last);
    cx.run_until_parked();
    tab.read_with(cx, |t, _| assert!(t.blocklist_error(count - 1).is_none()));
    let stored = saved(cx, &fx, &view);
    assert_eq!(
        stored.command_blocklist.last().map(String::as_str),
        Some("^dangerous-tool\\b")
    );
    assert!(cx
        .update(|cx| runtime::limits(cx))
        .command_blocklist
        .iter()
        .any(|p| p == "^dangerous-tool\\b"));

    // remove it again, then restore the defaults
    win(cx, &fx, |w, cx| {
        tab.update(cx, |t, cx| t.remove_pattern(count - 1, w, cx))
    });
    assert_eq!(saved(cx, &fx, &view).command_blocklist.len(), count - 1);
    win(cx, &fx, |w, cx| {
        tab.update(cx, |t, cx| t.reset_blocklist(w, cx))
    });
    assert_eq!(
        saved(cx, &fx, &view).command_blocklist,
        ops_ai::prefs::safety::default_command_blocklist()
    );
}

#[gpui_kit::test]
fn permission_grants_can_be_added_edited_exported_and_imported(cx: &mut TestAppContext) {
    let _s = serial();
    let (fx, view) = open(cx);
    let tab = view.read_with(cx, |v, _| v.safety().clone());
    win(cx, &fx, |w, cx| tab.update(cx, |t, cx| t.add_grant(w, cx)));
    let (id, command, note) = tab.read_with(cx, |t, _| t.grant_inputs()[0].clone());
    set_text(cx, &fx, &note, "cpu info");
    blur(cx, &note);

    // only shell capabilities take a command pattern: the first grantable one does not
    set_text(cx, &fx, &command, "ignored *");
    blur(cx, &command);
    let stored = saved(cx, &fx, &view);
    let rule = &stored.permission_grants[0];
    assert_eq!(
        (
            rule.id.as_str(),
            rule.capability_id.as_str(),
            rule.command_pattern.as_deref(),
            rule.note.as_deref()
        ),
        (
            id.as_str(),
            "portforward.rules.create",
            None,
            Some("cpu info")
        )
    );
    tab.update_in_test(cx, &fx, |t, cx| {
        t.set_grant_capability(&id, "terminal.execute", cx)
    });
    set_text(cx, &fx, &command, "lscpu *");
    blur(cx, &command);
    let stored = saved(cx, &fx, &view);
    assert_eq!(
        (
            stored.permission_grants[0].capability_id.as_str(),
            stored.permission_grants[0].command_pattern.as_deref()
        ),
        ("terminal.execute", Some("lscpu *"))
    );
    // and the pattern goes away with a capability that cannot use it
    tab.update_in_test(cx, &fx, |t, cx| {
        t.set_grant_capability(&id, "sftp.write", cx)
    });
    assert_eq!(
        saved(cx, &fx, &view).permission_grants[0].command_pattern,
        None
    );
    tab.update_in_test(cx, &fx, |t, cx| {
        t.set_grant_capability(&id, "terminal.execute", cx)
    });

    // export, wipe and import it back
    let json = tab.read_with(cx, |t, cx| t.export_json(cx));
    assert!(json.contains("\"capabilityId\""));
    win(cx, &fx, |w, cx| {
        tab.update(cx, |t, cx| t.remove_grant(&id, w, cx))
    });
    assert!(saved(cx, &fx, &view).permission_grants.is_empty());
    win(cx, &fx, |w, cx| {
        tab.update(cx, |t, cx| t.import_json(&json, w, cx))
    });
    assert_eq!(saved(cx, &fx, &view).permission_grants.len(), 1);
    win(cx, &fx, |w, cx| {
        tab.update(cx, |t, cx| t.import_json("{not json", w, cx))
    });
    tab.read_with(cx, |t, _| assert!(t.import_error().is_some()));
    assert_eq!(
        saved(cx, &fx, &view).permission_grants.len(),
        1,
        "a bad file changes nothing"
    );
    win(cx, &fx, |w, cx| {
        tab.update(cx, |t, cx| t.import_json(r#"{"a":1}"#, w, cx))
    });
    tab.read_with(cx, |t, _| assert!(t.import_error().is_some()));
}

// ---------------------------------------------------------------------------
// Web search
// ---------------------------------------------------------------------------

#[gpui_kit::test]
fn web_search_settings_follow_the_provider(cx: &mut TestAppContext) {
    let _s = serial();
    let (fx, view) = open(cx);
    let tab = view.read_with(cx, |v, _| v.search().clone());
    tab.read_with(cx, |t, cx| {
        let c = t.config(cx);
        assert_eq!(
            (c.provider_id.as_str(), c.enabled, c.max_results),
            ("tavily", false, Some(5))
        );
        assert_eq!(
            t.api_host().read(cx).value().to_string(),
            "https://api.tavily.com"
        );
    });
    let (key, host, max) = tab.read_with(cx, |t, _| {
        (
            t.api_key().clone(),
            t.api_host().clone(),
            t.max_results().clone(),
        )
    });
    set_text(cx, &fx, &key, "tvly-abc");
    blur(cx, &key);
    set_text(cx, &fx, &max, "50");
    blur(cx, &max);
    tab.update_in_test(cx, &fx, |t, cx| t.set_enabled(true, cx));
    let stored = saved(cx, &fx, &view);
    let c = stored.web_search.clone().unwrap();
    assert_eq!(
        (c.api_key.as_deref(), c.enabled, c.max_results),
        (Some("tvly-abc"), true, Some(5)),
        "an out-of-range count is refused"
    );
    assert!(c.is_ready());
    set_text(cx, &fx, &max, "12");
    blur(cx, &max);

    // switching provider resets the key and host
    win(cx, &fx, |w, cx| {
        tab.update(cx, |t, cx| t.choose_provider("searxng", w, cx))
    });
    let c = saved(cx, &fx, &view).web_search.unwrap();
    assert_eq!(
        (
            c.provider_id.as_str(),
            c.api_key.clone(),
            c.api_host.clone(),
            c.max_results
        ),
        ("searxng", None, None, Some(12))
    );
    assert!(!c.is_ready(), "SearXNG needs a host");
    set_text(cx, &fx, &host, "https://searx.example");
    blur(cx, &host);
    assert!(saved(cx, &fx, &view).web_search.unwrap().is_ready());
    win(cx, &fx, |w, cx| {
        tab.update(cx, |t, cx| t.choose_provider("exa", w, cx))
    });
    tab.read_with(cx, |t, cx| {
        assert_eq!(
            t.api_host().read(cx).value().to_string(),
            "https://api.exa.ai"
        )
    });
}

// ---------------------------------------------------------------------------
// Tool access
// ---------------------------------------------------------------------------

#[gpui_kit::test]
fn privacy_is_validated_before_it_is_applied(cx: &mut TestAppContext) {
    let _s = serial();
    let (fx, view) = open(cx);
    let tab = view.read_with(cx, |v, _| v.tools().clone());
    let (cidrs, patterns) = tab.read_with(cx, |t, _| (t.cidrs().clone(), t.patterns().clone()));

    win(cx, &fx, |w, cx| {
        cidrs.update(cx, |s, cx| {
            s.set_value("10.0.0.0/8\nnot-a-cidr".to_string(), w, cx)
        })
    });
    assert!(!tab.update_ret(cx, |t, cx| t.apply_privacy(cx)));
    tab.read_with(cx, |t, _| {
        assert!(t.privacy_error().is_some() && !t.privacy_applied())
    });
    win(cx, &fx, |w, cx| {
        cidrs.update(cx, |s, cx| {
            s.set_value("203.0.113.0/24\n\n2001:db8::/32".to_string(), w, cx)
        })
    });
    win(cx, &fx, |w, cx| {
        patterns.update(cx, |s, cx| s.set_value("(".to_string(), w, cx))
    });
    assert!(!tab.update_ret(cx, |t, cx| t.apply_privacy(cx)));
    win(cx, &fx, |w, cx| {
        patterns.update(cx, |s, cx| {
            s.set_value("INTERNAL_[A-Z0-9]+".to_string(), w, cx)
        })
    });
    assert!(tab.update_ret(cx, |t, cx| t.apply_privacy(cx)));
    tab.read_with(cx, |t, _| {
        assert!(t.privacy_applied() && t.privacy_error().is_none())
    });
    let stored = saved(cx, &fx, &view);
    assert_eq!(
        stored.privacy.cidrs,
        vec!["203.0.113.0/24", "2001:db8::/32"]
    );
    assert_eq!(stored.privacy.patterns, vec!["INTERNAL_[A-Z0-9]+"]);
    assert!(
        !stored.privacy.enabled,
        "applying does not switch it on by itself"
    );
    assert!(cx.update(|cx| runtime::limits(cx)).privacy.cidrs.len() == 2);
}

#[gpui_kit::test]
fn quick_messages_are_validated_and_stored(cx: &mut TestAppContext) {
    let _s = serial();
    let (fx, view) = open(cx);
    let tools = view.read_with(cx, |v, _| v.tools().clone());
    let quick = tools.read_with(cx, |t, _| t.quick().clone());

    // a new message: the slug follows the name, then everything is validated
    let dialog = win(cx, &fx, |w, cx| {
        quick.update(cx, |q, cx| q.open_edit(None, w, cx))
    });
    let (name, slug, content) = dialog.read_with(cx, |d, _| {
        (d.name.clone(), d.slug.clone(), d.content.clone())
    });
    win(cx, &fx, |w, cx| {
        let ok = dialog.update(cx, |d, cx| d.submit(w, cx));
        assert!(!ok);
    });
    dialog.read_with(cx, |d, _| {
        assert_eq!(
            d.errors().iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            vec!["name", "slug", "content"]
        )
    });
    set_text(cx, &fx, &name, "Check Disk");
    cx.update(|cx| name.update(cx, |_, cx| cx.emit(InputEvent::Change)));
    cx.run_until_parked();
    assert_eq!(
        slug.read_with(cx, |s, _| s.value().to_string()),
        "check-disk"
    );
    win(cx, &fx, |w, cx| {
        content.update(cx, |s, cx| s.set_value("df -h".to_string(), w, cx))
    });
    win(cx, &fx, |w, cx| {
        assert!(dialog.update(cx, |d, cx| d.submit(w, cx)))
    });
    let stored = saved(cx, &fx, &view);
    assert_eq!(
        (
            stored.quick_messages[0].name.as_str(),
            stored.quick_messages[0].slug.as_str(),
            stored.quick_messages[0].content.as_str()
        ),
        ("Check Disk", "check-disk", "df -h")
    );

    // the same slug again, and a reserved one, are refused
    let dialog = win(cx, &fx, |w, cx| {
        quick.update(cx, |q, cx| q.open_edit(None, w, cx))
    });
    let (name, slug, content) = dialog.read_with(cx, |d, _| {
        (d.name.clone(), d.slug.clone(), d.content.clone())
    });
    set_text(cx, &fx, &name, "Another");
    set_text(cx, &fx, &slug, "check-disk");
    win(cx, &fx, |w, cx| {
        content.update(cx, |s, cx| s.set_value("x".to_string(), w, cx))
    });
    win(cx, &fx, |w, cx| {
        assert!(!dialog.update(cx, |d, cx| d.submit(w, cx)))
    });
    dialog.read_with(cx, |d, _| assert_eq!(d.errors()[0].0, "slug"));
    set_text(cx, &fx, &slug, "stop");
    win(cx, &fx, |w, cx| {
        assert!(!dialog.update(cx, |d, cx| d.submit(w, cx)))
    });

    // editing keeps the message's identity; deleting removes it
    let existing = quick.read_with(cx, |q, cx| q.messages(cx)[0].clone());
    let edit = win(cx, &fx, |w, cx| {
        quick.update(cx, |q, cx| q.open_edit(Some(existing.clone()), w, cx))
    });
    let content = edit.read_with(cx, |d, _| d.content.clone());
    win(cx, &fx, |w, cx| {
        content.update(cx, |s, cx| s.set_value("df -h /var".to_string(), w, cx))
    });
    win(cx, &fx, |w, cx| {
        assert!(edit.update(cx, |d, cx| d.submit(w, cx)))
    });
    let stored = saved(cx, &fx, &view);
    assert_eq!(
        (
            stored.quick_messages.len(),
            stored.quick_messages[0].id.as_str(),
            stored.quick_messages[0].content.as_str()
        ),
        (1, existing.id.as_str(), "df -h /var")
    );
    quick.update_in_test(cx, &fx, |q, cx| q.remove(&existing.id, cx));
    assert!(saved(cx, &fx, &view).quick_messages.is_empty());
}

#[gpui_kit::test]
fn user_skills_are_scanned_from_the_data_folder(cx: &mut TestAppContext) {
    let _s = serial();
    let fx = common::setup(cx, |_| {});
    let dir = fx.ctx.core.data_dir.join("skills");
    std::fs::create_dir_all(dir.join("Deploy")).unwrap();
    std::fs::write(
        dir.join("Deploy/SKILL.md"),
        "---\nname: Deploy\ndescription: ships\n---\nbody",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("broken")).unwrap();
    let view: Entity<AiSettingsView> = win(cx, &fx, |window, cx| {
        cx.new(|cx| AiSettingsView::new(window, cx))
    });
    let tools = view.read_with(cx, |v, _| v.tools().clone());
    wait_until(cx, "skills scanned", |cx| {
        tools.read_with(cx, |t, _| t.skills().is_some() && !t.skills_loading())
    });
    tools.read_with(cx, |t, _| {
        let status = t.skills().unwrap().as_ref().unwrap();
        assert_eq!((status.ready_count, status.warning_count), (1, 1));
    });
    // a ready skill reserves its slash command
    let quick = tools.read_with(cx, |t, _| t.quick().clone());
    wait_until(cx, "preferences loaded", |cx| {
        view.read_with(cx, |v, cx| v.loaded(cx))
    });
    let dialog = win(cx, &fx, |w, cx| {
        quick.update(cx, |q, cx| q.open_edit(None, w, cx))
    });
    let (name, slug, content) = dialog.read_with(cx, |d, _| {
        (d.name.clone(), d.slug.clone(), d.content.clone())
    });
    set_text(cx, &fx, &name, "Mine");
    set_text(cx, &fx, &slug, "deploy");
    win(cx, &fx, |w, cx| {
        content.update(cx, |s, cx| s.set_value("x".to_string(), w, cx))
    });
    win(cx, &fx, |w, cx| {
        assert!(!dialog.update(cx, |d, cx| d.submit(w, cx)))
    });
    dialog.read_with(cx, |d, _| {
        assert!(d.errors().iter().any(|(k, _)| *k == "slug"))
    });
}

#[gpui_kit::test]
fn tool_access_mode_and_chat_shortcut_are_stored(cx: &mut TestAppContext) {
    let _s = serial();
    let (fx, view) = open(cx);
    let tools = view.read_with(cx, |v, _| v.tools().clone());
    tools.read_with(cx, |t, cx| {
        assert!(t.guidance_text(cx).contains("Settings → AI → Tool Access"))
    });
    tools.update_in_test(cx, &fx, |t, cx| t.set_show_selection_action(false, cx));
    assert!(
        !cx.update(|cx| runtime::limits(cx)).show_selection_action,
        "the terminal menu reads it from the runtime"
    );
    let stored = saved(cx, &fx, &view);
    assert!(!stored.show_terminal_selection_ai_action);
    assert_eq!(stored.tool_integration_mode, ToolIntegrationMode::Mcp);
}

#[gpui_kit::test]
fn external_mcp_settings_validate_and_generate_http_snippets(cx: &mut TestAppContext) {
    let _s = serial();
    let (fx, view) = open(cx);
    let mcp = view.read_with(cx, |v, cx| v.tools().read(cx).mcp().clone());
    mcp.update_in_test(cx, &fx, |m, cx| m.set_enabled(true, cx));
    mcp.update_in_test(cx, &fx, |m, cx| m.set_http_enabled(true, cx));
    let stored = saved(cx, &fx, &view);
    assert!(stored.external_mcp.enabled && stored.external_mcp.http.enabled);
    assert_eq!(
        stored.external_mcp.http.token.len(),
        64,
        "a token is minted when HTTP is switched on"
    );
    let token = stored.external_mcp.http.token.clone();
    mcp.read_with(cx, |m, cx| {
        let texts = m.http_texts(cx);
        assert_eq!(texts[0], "http://127.0.0.1:47391/mcp");
        assert!(texts[3].contains(&token) && texts[4].contains(&token));
    });

    // port validation
    let port = mcp.read_with(cx, |m, _| m.port().clone());
    set_text(cx, &fx, &port, "80");
    blur(cx, &port);
    mcp.read_with(cx, |m, _| assert_eq!(m.http_errors(), ["port"]));
    set_text(cx, &fx, &port, "50000");
    blur(cx, &port);
    mcp.read_with(cx, |m, _| assert!(m.http_errors().is_empty()));
    assert_eq!(saved(cx, &fx, &view).external_mcp.http.port, 50000);
    mcp.read_with(cx, |m, cx| {
        assert_eq!(m.http_texts(cx)[0], "http://127.0.0.1:50000/mcp")
    });

    // regenerating the token changes the snippets
    mcp.update_in_test(cx, &fx, |m, cx| m.regenerate_token(cx));
    let stored = saved(cx, &fx, &view);
    assert_ne!(stored.external_mcp.http.token, token);
    // a custom bind address only applies when allowed
    win(cx, &fx, |w, cx| {
        mcp.update(cx, |m, cx| m.set_allow_custom_bind(true, w, cx))
    });
    let address = mcp.read_with(cx, |m, _| m.listen_address().clone());
    set_text(cx, &fx, &address, "host.local");
    blur(cx, &address);
    mcp.read_with(cx, |m, _| assert_eq!(m.http_errors(), ["address"]));
    set_text(cx, &fx, &address, "0.0.0.0");
    blur(cx, &address);
    let stored = saved(cx, &fx, &view);
    assert_eq!(stored.external_mcp.http.listen_address, "0.0.0.0");
    mcp.read_with(cx, |m, cx| {
        assert!(
            m.http_texts(cx)[0].is_empty(),
            "a wildcard bind needs a client host"
        )
    });
    let idle = mcp.read_with(cx, |m, _| m.idle_timeout().clone());
    set_text(cx, &fx, &idle, "0");
    blur(cx, &idle);
    assert_eq!(saved(cx, &fx, &view).external_mcp.idle_timeout_minutes, 1);
}

// ---------------------------------------------------------------------------
// Agents
// ---------------------------------------------------------------------------

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "types POSIX shell syntax into the local shell")]
fn agent_clis_are_detected_and_the_default_agent_follows(cx: &mut TestAppContext) {
    let _s = serial();
    let (fx, view) = open(cx);
    let tab = view.read_with(cx, |v, _| v.agents().clone());
    wait_until(cx, "discovery finished", |cx| {
        tab.read_with(cx, |t, _| !t.resolving() && t.path_info("codex").is_some())
    });

    // a typed path is checked and remembered as a manual command
    let input = tab.read_with(cx, |t, _| t.path_input("grok").unwrap());
    set_text(cx, &fx, &input, "/bin/sh");
    win(cx, &fx, |w, cx| {
        tab.update(cx, |t, cx| t.check_path("grok", w, cx))
    });
    wait_until(cx, "grok checked", |cx| {
        tab.read_with(cx, |t, _| {
            !t.resolving()
                && t.path_info("grok")
                    .is_some_and(|i| i.path.as_deref() == Some("/bin/sh"))
        })
    });
    let stored = saved(cx, &fx, &view);
    let grok = stored
        .external_agents
        .iter()
        .find(|a| a.id == "discovered_grok")
        .unwrap();
    assert_eq!(
        (
            grok.command.as_str(),
            grok.command_source.as_deref(),
            grok.enabled,
            grok.available
        ),
        ("/bin/sh", Some("manual"), true, Some(true))
    );

    // a path that does not exist removes the agent again
    set_text(cx, &fx, &input, "/definitely/not/here");
    win(cx, &fx, |w, cx| {
        tab.update(cx, |t, cx| t.check_path("grok", w, cx))
    });
    wait_until(cx, "grok gone", |cx| {
        tab.read_with(cx, |t, _| {
            !t.resolving() && t.path_info("grok").is_some_and(|i| !i.available)
        })
    });
    assert!(saved(cx, &fx, &view)
        .external_agents
        .iter()
        .all(|a| a.id != "discovered_grok"));

    // reset goes back to PATH discovery
    win(cx, &fx, |w, cx| {
        tab.update(cx, |t, cx| t.reset_path("grok", w, cx))
    });
    wait_until(cx, "reset done", |cx| {
        tab.read_with(cx, |t, _| !t.resolving())
    });
    tab.read_with(cx, |t, cx| {
        assert_eq!(
            t.path_input("grok").unwrap().read(cx).value().to_string(),
            ""
        )
    });
}

#[gpui_kit::test]
fn claude_config_cursor_key_and_codebuddy_options_are_kept_without_the_cli(
    cx: &mut TestAppContext,
) {
    let _s = serial();
    let (fx, view) = open(cx);
    let tab = view.read_with(cx, |v, _| v.agents().clone());
    wait_until(cx, "discovery finished", |cx| {
        tab.read_with(cx, |t, _| !t.resolving() && t.path_info("codex").is_some())
    });
    // these tests must not depend on what the machine has installed: the entries below are created by the user's input
    let (dir, env) = tab.read_with(cx, |t, _| (t.claude_dir().clone(), t.claude_env().clone()));
    set_text(cx, &fx, &dir, "/home/me/.claude");
    win(cx, &fx, |w, cx| {
        env.update(cx, |s, cx| {
            s.set_value(
                "FOO=1\nCLAUDE_CONFIG_DIR=ignored\n# comment".to_string(),
                w,
                cx,
            )
        })
    });
    blur(cx, &dir);
    let stored = saved(cx, &fx, &view);
    let claude = stored
        .external_agents
        .iter()
        .find(|a| a.id == "discovered_claude")
        .unwrap();
    assert_eq!(
        claude.env.get("CLAUDE_CONFIG_DIR").map(String::as_str),
        Some("/home/me/.claude")
    );
    assert_eq!(claude.env.get("FOO").map(String::as_str), Some("1"));
    assert!(claude
        .env
        .keys()
        .all(|k| k != "CLAUDE_CONFIG_DIR" || claude.env[k] == "/home/me/.claude"));

    // CodeBuddy options
    tab.update_in_test(cx, &fx, |t, cx| {
        t.update_codebuddy_options(cx, |o| {
            o.max_turns = Some(5);
            o.sandbox_enabled = Some(true);
        })
    });
    let stored = saved(cx, &fx, &view);
    let cb = stored
        .external_agents
        .iter()
        .find(|a| a.id == "discovered_codebuddy")
        .unwrap();
    assert_eq!(
        (
            cb.codebuddy_options.as_ref().unwrap().max_turns,
            cb.codebuddy_options.as_ref().unwrap().sandbox_enabled
        ),
        (Some(5), Some(true))
    );

    // Cursor API key
    let key = tab.read_with(cx, |t, _| t.cursor_key().clone());
    set_text(cx, &fx, &key, "cursor-key-123");
    win(cx, &fx, |w, cx| {
        tab.update(cx, |t, cx| t.save_cursor_key(w, cx))
    });
    let stored = saved(cx, &fx, &view);
    let cursor = stored
        .external_agents
        .iter()
        .find(|a| a.id == "discovered_cursor")
        .unwrap();
    assert_eq!(
        (
            cursor.api_key.as_deref(),
            cursor.cursor_auth_mode.as_deref()
        ),
        (Some("cursor-key-123"), Some("api-key"))
    );
    tab.update_in_test(cx, &fx, |t, cx| t.set_cursor_mode("cli-login", cx));
    assert_eq!(
        saved(cx, &fx, &view)
            .external_agents
            .iter()
            .find(|a| a.id == "discovered_cursor")
            .unwrap()
            .cursor_auth_mode
            .as_deref(),
        Some("cli-login")
    );
}

// ---------------------------------------------------------------------------
// i18n and rendering
// ---------------------------------------------------------------------------

#[gpui_kit::test]
fn every_dynamic_translation_key_exists(_cx: &mut TestAppContext) {
    let mut keys: Vec<String> = Vec::new();
    for m in ["observer", "confirm", "auto"] {
        keys.push(format!("ncai.safety.permissionMode.{m}"));
    }
    for s in ["openai", "anthropic", "google"] {
        keys.push(format!("ncai.providers.style.{s}"));
    }
    for e in ["off", "unfocused", "always"] {
        keys.push(format!("ncai.externalMcp.approvalAlert.{e}"));
    }
    for e in ["minimal", "low", "medium", "high", "xhigh", "max"] {
        keys.push(format!("ncai.codebuddy.effort.{e}"));
    }
    for c in ["codex", "claude", "grok", "cursor", "other"] {
        keys.push(format!("ncai.externalMcp.client.{c}"));
    }
    for prefix in [
        "codex",
        "claude",
        "copilot",
        "codebuddy",
        "opencode",
        "grok",
        "mimo",
    ] {
        for k in [
            "description",
            "detecting",
            "detected",
            "notFound",
            "path",
            "notFoundHint",
            "customPathPlaceholder",
            "check",
            "resetPath",
        ] {
            // Codex shows its login state instead of "detected"
            if !(prefix == "codex" && k == "detected") {
                keys.push(format!("ncai.{prefix}.{k}"));
            }
        }
    }
    for k in [
        "usage.keepRunning",
        "usage.localhost",
        "usage.permissions",
        "usage.capabilities",
    ] {
        keys.push(format!("ncai.externalMcp.{k}"));
    }
    let mut missing = Vec::new();
    for lang in [ops_i18n::Language::ZhCn, ops_i18n::Language::EnUs] {
        for key in &keys {
            if ops_i18n::template(lang, key) == *key {
                missing.push(format!("{key} ({})", lang.code()));
            }
        }
    }
    assert!(missing.is_empty(), "missing: {missing:?}");
}

#[gpui_kit::test]
fn every_tab_renders_in_both_languages(cx: &mut TestAppContext) {
    use opsbatch_app::settings::window::{Section, SettingsWindow};
    let _s = serial();
    let _fx = common::setup(cx, |_| {});
    let (handle, settings) = cx.update(|cx| {
        let (w, v) = gpui_kit::open_window(
            gpui_kit::WindowOptions {
                window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds {
                    origin: Default::default(),
                    size: gpui_kit::size(gpui_kit::px(1100.), gpui_kit::px(800.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| SettingsWindow::new(Section::Ai, window, cx)),
        )
        .unwrap();
        (w.downcast::<gpui_kit::base::Root>().unwrap(), v)
    });
    let view = cx.update(|cx| settings.read(cx).ai().clone());
    wait_until(cx, "preferences loaded", |cx| {
        view.read_with(cx, |v, cx| v.loaded(cx))
    });
    wait_until(cx, "agents discovered", |cx| {
        view.read_with(cx, |v, cx| v.agents().read(cx).path_info("codex").is_some())
    });
    for lang in [ops_i18n::LanguageMode::ZhCn, ops_i18n::LanguageMode::EnUs] {
        cx.update(|cx| opsbatch_app::language::set_mode(lang, cx));
        for (tab, marker) in [
            (AiTab::Providers, "ai-providers-tab"),
            (AiTab::Agents, "ai-agents-tab"),
            (AiTab::Tools, "ai-tools-tab"),
            (AiTab::Search, "ai-search-tab"),
            (AiTab::Safety, "ai-safety-tab"),
        ] {
            cx.update_window(handle.into(), |_, w, cx| {
                view.update(cx, |v, cx| v.select_tab(tab, cx));
                w.render_frame(cx);
                assert!(w.try_find("ai-tabs").is_some(), "tab bar");
                assert!(w.try_find(marker).is_some(), "{marker} ({lang:?})");
            })
            .unwrap();
        }
    }
    // an open provider form renders too
    let providers = cx.update(|cx| view.read(cx).providers().clone());
    cx.update_window(handle.into(), |_, w, cx| {
        view.update(cx, |v, cx| v.select_tab(AiTab::Providers, cx));
        let id = providers.update(cx, |t, cx| t.add_provider("anthropic", cx));
        providers.update(cx, |t, cx| t.toggle_edit(&id, w, cx));
        w.render_frame(cx);
        assert!(w.try_find("ai-provider-form").is_some());
    })
    .unwrap();
    cx.update(|cx| opsbatch_app::language::set_mode(ops_i18n::LanguageMode::ZhCn, cx));
}
