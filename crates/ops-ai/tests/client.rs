//! AI client against a local OpenAI-compatible mock server (no network).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ops_ai::client::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

struct Mock {
    url: String,
    requests: Arc<AtomicUsize>,
}

async fn read_request(sock: &mut tokio::net::TcpStream) -> (String, String) {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let mut header_end = None;
    loop {
        let n = sock.read(&mut tmp).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if header_end.is_none() {
            header_end = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4);
        }
        if let Some(he) = header_end {
            let head = String::from_utf8_lossy(&buf[..he]).to_lowercase();
            let len = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
            if buf.len() >= he + len {
                break;
            }
        }
    }
    let text = String::from_utf8_lossy(&buf).to_string();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    (head.lines().next().unwrap_or("").to_string(), body.to_string())
}

async fn start_mock() -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let counter = requests.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { break };
            let counter = counter.clone();
            tokio::spawn(async move {
                let (line, body) = read_request(&mut sock).await;
                counter.fetch_add(1, Ordering::SeqCst);
                if line.contains("/models") {
                    let b = r#"{"data":[{"id":"m-b","owned_by":"x"},{"id":"m-a"}]}"#;
                    let _ = sock.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}", b.len()).as_bytes()).await;
                } else if body.contains("UNAUTHORIZED") {
                    let b = r#"{"error":{"message":"bad key sk-secret123"}}"#;
                    let _ = sock.write_all(format!("HTTP/1.1 401 Unauthorized\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}", b.len()).as_bytes()).await;
                } else if body.contains("\"stream\":true") {
                    let slow = body.contains("SLOW");
                    let _ = sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").await;
                    let n = if slow { 100 } else { 3 };
                    for i in 0..n {
                        let delta = if slow { format!("w{i} ") } else { ["Hel", "lo ", "世界"][i].to_string() };
                        let line = format!("data: {}\n\n", serde_json::json!({"model":"mock-1","choices":[{"delta":{"content":delta}}]}));
                        if sock.write_all(line.as_bytes()).await.is_err() {
                            return;
                        }
                        tokio::time::sleep(Duration::from_millis(if slow { 100 } else { 10 })).await;
                    }
                    let _ = sock.write_all(b"data: [DONE]\n\n").await;
                } else {
                    let b = r#"{"model":"mock-1","choices":[{"message":{"content":"plain reply"}}],"usage":{"total_tokens":7}}"#;
                    let _ = sock.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}", b.len()).as_bytes()).await;
                }
            });
        }
    });
    Mock { url, requests }
}

async fn configured(core: &ops_core::Core, mock: &Mock) {
    save_ai_config(&core.db, AiConfig { provider: "ollama".into(), api_url: mock.url.clone(), api_key: String::new(), model: "mock-1".into(), enabled: true }).await.unwrap();
}

fn msg(role: &str, content: &str) -> ChatMessage {
    ChatMessage { role: role.into(), content: content.into(), images: None }
}

fn stream_req(content: &str, conv: Option<&str>, scope: &str, scope_id: &str) -> StreamChatRequest {
    StreamChatRequest {
        messages: vec![msg("user", content)],
        model: None,
        temperature: None,
        max_tokens: None,
        conversation_id: conv.map(str::to_string),
        scope: Some(scope.into()),
        scope_id: Some(scope_id.into()),
        client_request_id: Some("req-1".into()),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn config_roundtrip_masks_keys_and_normalizes_urls() {
    let core = ops_core::Core::open_temp("ai-config").unwrap();
    let defaults = get_ai_config(&core.db).await.unwrap();
    assert!(!defaults.enabled && defaults.provider == "openai");
    save_ai_config(&core.db, AiConfig { provider: "openai".into(), api_url: "https://example.test/v1/chat/completions/".into(), api_key: "sk-abcdef123456".into(), model: "gpt-x".into(), enabled: true }).await.unwrap();
    let got = get_ai_config(&core.db).await.unwrap();
    assert_eq!(got.api_url, "https://example.test/v1");
    assert_eq!(got.api_key, "sk-a****3456", "masked for display");
    // Saving the masked value keeps the stored key.
    save_ai_config(&core.db, AiConfig { model: "gpt-y".into(), ..got.clone() }).await.unwrap();
    let again = get_ai_config(&core.db).await.unwrap();
    assert_eq!((again.api_key.as_str(), again.model.as_str()), ("sk-a****3456", "gpt-y"));
    // Ollama needs no key.
    save_ai_config(&core.db, AiConfig { provider: "ollama".into(), api_url: String::new(), api_key: String::new(), model: "llama3".into(), enabled: true }).await.unwrap();
    assert_eq!(get_ai_config(&core.db).await.unwrap().api_url, "http://localhost:11434/v1");
}

#[tokio::test(flavor = "multi_thread")]
async fn streaming_emits_chunks_persists_the_conversation_and_lists_models() {
    let core = ops_core::Core::open_temp("ai-stream").unwrap();
    let mock = start_mock().await;
    configured(&core, &mock).await;
    let rx = core.events.subscribe("ai-stream-chunk");

    let res = ai_chat_stream(&core, &core.db, stream_req("say hello", None, "ssh_host", "h1")).await.unwrap();
    let conv_id = res["conversation_id"].as_str().unwrap().to_string();
    let mut text = String::new();
    let mut done = false;
    while let Ok(ev) = rx.try_recv() {
        let c = &ev.payload;
        assert_eq!(c["client_request_id"], "req-1");
        text.push_str(c["delta"].as_str().unwrap_or(""));
        done |= c["done"].as_bool().unwrap_or(false);
    }
    assert_eq!(text, "Hello 世界");
    assert!(done, "a final done chunk is sent");

    let (conv, messages) = ai_get_conversation(&core.db, conv_id.clone()).await.unwrap();
    assert_eq!((conv.scope.as_str(), conv.scope_id.as_str(), conv.title.as_str()), ("ssh_host", "h1", "say hello"));
    assert_eq!(messages.iter().map(|m| (m.role.as_str(), m.content.as_str())).collect::<Vec<_>>(), [("user", "say hello"), ("assistant", "Hello 世界")]);

    // Listing filters by scope; reusing a conversation for another host is refused.
    assert_eq!(ai_list_conversations(&core.db, Some("ssh_host".into()), Some("h1".into())).await.unwrap().len(), 1);
    assert!(ai_list_conversations(&core.db, Some("ssh_host".into()), Some("h2".into())).await.unwrap().is_empty());
    let err = ai_chat_stream(&core, &core.db, stream_req("again", Some(&conv_id), "ssh_host", "h2")).await.unwrap_err();
    assert!(err.contains("scope"), "{err}");
    ai_delete_conversation(&core.db, conv_id.clone()).await.unwrap();
    assert!(ai_get_conversation(&core.db, conv_id).await.is_err());

    let models = ai_list_models(&core.db, None, None, None).await.unwrap();
    assert_eq!(models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>().len(), 2);
    assert!(mock.requests.load(Ordering::SeqCst) >= 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_slow_stream_stops_it_and_keeps_the_partial_answer() {
    let core = ops_core::Core::open_temp("ai-cancel").unwrap();
    let mock = start_mock().await;
    configured(&core, &mock).await;
    let rx = core.events.subscribe("ai-stream-chunk");
    let core2 = core.clone();
    let task = tokio::spawn(async move { ai_chat_stream(&core2, &core2.db, stream_req("SLOW please", Some("conv-slow"), "global", "")).await });
    tokio::time::sleep(Duration::from_millis(600)).await;
    ai_chat_cancel("conv-slow".into()).unwrap();
    let started = std::time::Instant::now();
    let res = tokio::time::timeout(Duration::from_secs(5), task).await.expect("stream ended after cancel").unwrap().unwrap();
    assert!(started.elapsed() < Duration::from_secs(3), "did not wait for the 10 s the server would take");
    assert_eq!(res["conversation_id"], "conv-slow");
    let mut chunks = 0;
    let mut done = false;
    while let Ok(ev) = rx.try_recv() {
        chunks += 1;
        done |= ev.payload["done"].as_bool().unwrap_or(false);
    }
    assert!(chunks >= 2 && done);
    let (_, messages) = ai_get_conversation(&core.db, "conv-slow".into()).await.unwrap();
    let answer = &messages.last().unwrap().content;
    assert!(answer.starts_with("w0 ") && answer.split(' ').count() < 50, "partial answer saved: {answer:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn provider_errors_are_reported_without_leaking_keys_and_disabled_ai_refuses() {
    let core = ops_core::Core::open_temp("ai-errors").unwrap();
    let mock = start_mock().await;
    // Disabled -> refused before any request.
    let err = ai_chat_stream(&core, &core.db, stream_req("hi", None, "global", "")).await.unwrap_err();
    assert!(err.contains("未启用") || err.to_lowercase().contains("enabled"), "{err}");
    assert_eq!(mock.requests.load(Ordering::SeqCst), 0);
    configured(&core, &mock).await;
    let err = ai_chat_stream(&core, &core.db, stream_req("UNAUTHORIZED", None, "global", "")).await.unwrap_err();
    assert!(err.contains("401") && !err.contains("sk-secret123"), "{err}");
    // The non-streaming call works through the same config.
    let r = ai_chat(&core.db, serde_json::from_value(serde_json::json!({"messages":[{"role":"user","content":"hi"}]})).unwrap()).await.unwrap();
    assert_eq!(r.content, "plain reply");
}
