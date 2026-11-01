use crate::buffer::Frame;
use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Clone, Default)]
pub struct Conversation {
    pub turns: Vec<(String, String)>,
}
impl Conversation {
    pub fn record(&mut self, q: String, a: String) {
        self.turns.push((q, a));
        if self.turns.len() > 6 {
            self.turns.remove(0);
        }
    }
}
pub fn request_body(
    model: &str,
    question: &str,
    frames: &[Frame],
    history: &Conversation,
) -> Value {
    let mut input = Vec::new();
    for (q, a) in &history.turns {
        input.push(json!({"role":"user","content":q}));
        input.push(json!({"role":"assistant","content":a}));
    }
    let mut content = vec![
        json!({"type":"input_text","text":format!("{question}\n\n{} recent distinct screenshots follow, oldest to newest. Each image is labeled with its age. Use the newest image as current context; earlier images show what changed. If there are no images, say you cannot see the screen.",frames.len())}),
    ];
    for frame in frames {
        content.push(json!({"type":"input_text","text":format!("Screenshot captured {} seconds ago",frame.captured.elapsed().as_secs())}));
        content.push(json!({"type":"input_image","detail":"low","image_url":format!("data:image/jpeg;base64,{}",STANDARD.encode(frame.jpeg.as_ref()))}));
    }
    input.push(json!({"role":"user","content":content}));
    json!({"model":model,"stream":true,"store":false,"max_output_tokens":1000,
        "instructions":crate::config::DEFAULT_PROMPT,"input":input})
}
#[cfg(test)]
pub fn provider_body(
    provider: &str,
    model: &str,
    question: &str,
    frames: &[Frame],
    history: &Conversation,
) -> Value {
    provider_body_with_prompt(provider, model, question, frames, history, crate::config::DEFAULT_PROMPT)
}
pub fn provider_body_with_prompt(
    provider: &str,
    model: &str,
    question: &str,
    frames: &[Frame],
    history: &Conversation,
    prompt: &str,
) -> Value {
    let mut body = request_body(model, question, frames, history);
    body["instructions"] = json!(prompt);
    if provider == "hermes" {
        body["optic_hermes"] = json!(true);
        return body;
    }
    if provider != "anthropic" {
        return body;
    }
    let mut messages = body["input"].as_array().unwrap().clone();
    for message in &mut messages {
        if let Some(parts) = message["content"].as_array_mut() {
            for part in parts {
                if part["type"] == "input_image" {
                    let url = part["image_url"].as_str().unwrap();
                    let data = url.split_once(',').unwrap().1;
                    *part = json!({"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":data}});
                } else {
                    part["type"] = json!("text");
                }
            }
        }
    }
    json!({"model":model,"stream":true,"max_tokens":1000,"system":body["instructions"],"messages":messages})
}
pub async fn provider_models(provider: &str, key: &str, local_url: &str) -> Result<Vec<String>> {
    if provider == "lmstudio" {
        return models_at(key, &format!("{}/models", local_url.trim_end_matches('/'))).await;
    }
    if provider == "hermes" {
        return models_at(key, "http://127.0.0.1:8642/v1/models").await;
    }
    if provider != "anthropic" {
        return models(key).await;
    }
    let client = reqwest::Client::new();
    let mut ids = Vec::new();
    let mut after = String::new();
    loop {
        let mut request = client
            .get("https://api.anthropic.com/v1/models")
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
            .timeout(Duration::from_secs(20))
            .query(&[("limit", "1000")]);
        if !after.is_empty() {
            request = request.query(&[("after_id", &after)]);
        }
        let response = request.send().await?;
        let status = response.status();
        let body: Value = response.json().await?;
        anyhow::ensure!(
            status.is_success(),
            "Anthropic {}: {}",
            status,
            body["error"]["message"]
                .as_str()
                .unwrap_or("Request failed")
        );
        ids.extend(
            body["data"]
                .as_array()
                .context("Missing model list")?
                .iter()
                .filter_map(|x| x["id"].as_str().map(str::to_owned)),
        );
        if body["has_more"] != true {
            break;
        }
        let next = body["last_id"]
            .as_str()
            .context("Missing pagination cursor")?
            .to_owned();
        anyhow::ensure!(next != after, "Repeated model cursor");
        after = next;
    }
    ids.sort();
    ids.dedup();
    Ok(ids)
}
pub async fn models(key: &str) -> Result<Vec<String>> {
    models_at(key, "https://api.openai.com/v1/models").await
}
async fn models_at(key: &str, url: &str) -> Result<Vec<String>> {
    let r = reqwest::Client::new()
        .get(url)
        .bearer_auth(key)
        .timeout(Duration::from_secs(20))
        .send()
        .await?;
    let status = r.status();
    let body: Value = r.json().await?;
    anyhow::ensure!(status.is_success(), "{}", api_error(&body, status.as_u16()));
    let mut ids: Vec<_> = body["data"]
        .as_array()
        .context("Missing model list")?
        .iter()
        .filter_map(|x| x["id"].as_str().map(str::to_owned))
        .collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}
fn api_error(body: &Value, status: u16) -> String {
    format!(
        "OpenAI {status}: {}",
        body["error"]["message"]
            .as_str()
            .unwrap_or("Request failed")
    )
}
#[derive(Default)]
pub struct SseDecoder {
    pending: Vec<u8>,
}
impl SseDecoder {
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Value>> {
        self.pending.extend_from_slice(chunk);
        let mut values = Vec::new();
        while let Some(end) = self.pending.iter().position(|x| *x == b'\n') {
            let line: Vec<_> = self.pending.drain(..=end).collect();
            let text = std::str::from_utf8(&line)?.trim_end_matches(['\r', '\n']);
            if let Some(data) = text.strip_prefix("data:") {
                let data = data.trim();
                if !data.is_empty() && data != "[DONE]" {
                    values.push(serde_json::from_str(data).context("Invalid streaming response")?);
                }
            }
        }
        anyhow::ensure!(self.pending.len() < 2_000_000, "Streaming event too large");
        Ok(values)
    }
}
pub async fn ask(
    key: String,
    body: Value,
    cancel: Arc<AtomicBool>,
    update: impl FnMut(String) + Send,
) -> Result<String> {
    let mut body = body;
    let local_url = body
        .as_object_mut()
        .unwrap()
        .remove("optic_lmstudio_url")
        .and_then(|v| v.as_str().map(str::to_owned));
    let local_endpoint = local_url.map(|url| format!("{}/responses", url.trim_end_matches('/')));
    let hermes = body
        .as_object_mut()
        .unwrap()
        .remove("optic_hermes")
        .is_some();
    let endpoint = if let Some(local) = &local_endpoint {
        local.as_str()
    } else if hermes {
        "http://127.0.0.1:8642/v1/responses"
    } else if body.get("messages").is_some() {
        "https://api.anthropic.com/v1/messages"
    } else {
        "https://api.openai.com/v1/responses"
    };
    ask_at(key, body, cancel, update, endpoint).await
}
async fn ask_at(
    key: String,
    body: Value,
    cancel: Arc<AtomicBool>,
    mut update: impl FnMut(String) + Send,
    endpoint: &str,
) -> Result<String> {
    let anthropic = body.get("messages").is_some();
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(120))
        .build()?;
    let request = client.post(endpoint);
    let request = if anthropic {
        request
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
    } else {
        request.bearer_auth(key)
    };
    let pending = request.json(&body).send();
    tokio::pin!(pending);
    let response = loop {
        anyhow::ensure!(!cancel.load(Ordering::Relaxed), "Response stopped");
        tokio::select! {
            response=&mut pending=>break response?,
            _=tokio::time::sleep(Duration::from_millis(100))=>continue,
        }
    };
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let value = response.json().await.unwrap_or(json!({}));
        anyhow::bail!(
            "{}",
            if anthropic {
                format!(
                    "Anthropic {status}: {}",
                    value["error"]["message"]
                        .as_str()
                        .unwrap_or("Request failed")
                )
            } else {
                api_error(&value, status)
            }
        );
    }
    let mut stream = response.bytes_stream();
    let mut decoder = SseDecoder::default();
    let mut answer = String::new();
    let mut completed = false;
    loop {
        if cancel.load(Ordering::Relaxed) {
            anyhow::bail!("Response stopped")
        }
        let chunk = tokio::select! {c=stream.next()=>c,_=tokio::time::sleep(Duration::from_millis(100))=>continue};
        let Some(chunk) = chunk else { break };
        for event in decoder.push(&chunk?)? {
            match event["type"].as_str().unwrap_or("") {
                "content_block_delta" => {
                    if event["delta"]["type"] == "text_delta" {
                        answer.push_str(event["delta"]["text"].as_str().unwrap_or(""));
                        update(answer.clone());
                    }
                }
                "message_stop" => completed = true,
                "message_delta" if event["delta"]["stop_reason"] == "max_tokens" => {
                    anyhow::bail!("Response was truncated. Ask a shorter question.")
                }
                "response.output_text.delta" => {
                    answer.push_str(event["delta"].as_str().unwrap_or(""));
                    update(answer.clone());
                }
                "response.completed" => completed = true,
                "response.failed" | "error" => anyhow::bail!(
                    "{}",
                    event
                        .pointer("/response/error/message")
                        .or_else(|| event.pointer("/error/message"))
                        .or_else(|| event.get("message"))
                        .and_then(Value::as_str)
                        .unwrap_or("OpenAI response failed")
                ),
                "response.incomplete" => {
                    anyhow::bail!("Response was truncated. Ask a shorter question.")
                }
                _ => {}
            }
        }
    }
    anyhow::ensure!(
        completed && !answer.is_empty(),
        "No complete text reply. Choose a model that supports text and image input."
    );
    Ok(answer)
}
#[cfg(test)]
mod tests {
    #[test]
    fn custom_prompt_reaches_every_provider() {
        let prompt = "Give practical steps in Spanish.";
        for provider in ["openai", "anthropic", "hermes", "lmstudio"] {
            let body = super::provider_body_with_prompt(provider, "test", "Help", &[], &super::Conversation::default(), prompt);
            let field = if provider == "anthropic" { "system" } else { "instructions" };
            assert_eq!(body[field], prompt, "{provider}");
        }
    }
    use super::*;
    #[tokio::test]
    async fn local_server_model_discovery_and_image_stream_routing() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for index in 0..2 {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0u8; 8192];
                loop {
                    let n = socket.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                        assert!(headers.contains("authorization: bearer lm-studio"));
                        let length: usize = headers
                            .lines()
                            .find_map(|s| s.strip_prefix("content-length:"))
                            .unwrap_or("0")
                            .trim()
                            .parse()
                            .unwrap();
                        if bytes.len() < end + 4 + length {
                            continue;
                        }
                        if index == 0 {
                            assert!(headers.starts_with("get /v1/models "));
                        } else {
                            assert!(headers.starts_with("post /v1/responses "));
                            let body: Value =
                                serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap();
                            assert!(body.get("optic_lmstudio_url").is_none());
                            assert_eq!(
                                body["input"][0]["content"]
                                    .as_array()
                                    .unwrap()
                                    .iter()
                                    .filter(|v| v["type"] == "input_image")
                                    .count(),
                                5
                            );
                        }
                        break;
                    }
                }
                let (kind, body) = if index == 0 {
                    ("application/json", r#"{"data":[{"id":"local-vision"}]}"#)
                } else {
                    (
                        "text/event-stream",
                        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"LOCAL_OK\"}\n\ndata: {\"type\":\"response.completed\"}\n\n",
                    )
                };
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
        });
        assert_eq!(
            provider_models("lmstudio", "lm-studio", &base)
                .await
                .unwrap(),
            vec!["local-vision"]
        );
        let frames: Vec<_> = (0..5)
            .map(|_| Frame {
                jpeg: Arc::new(vec![1, 2, 3]),
                captured: std::time::Instant::now(),
            })
            .collect();
        let mut body = provider_body(
            "lmstudio",
            "local-vision",
            "Help",
            &frames,
            &Conversation::default(),
        );
        body["optic_lmstudio_url"] = json!(base);
        assert_eq!(
            ask(
                "lm-studio".into(),
                body,
                Arc::new(AtomicBool::new(false)),
                |_| {}
            )
            .await
            .unwrap(),
            "LOCAL_OK"
        );
        server.join().unwrap();
    }
    #[tokio::test]
    #[ignore = "Requires the local authenticated Hermes bridge"]
    async fn live_hermes_subscription() {
        let key = crate::config::provider_key("hermes", "").unwrap();
        let body = provider_body(
            "hermes",
            "gpt-6.1-sol",
            "Reply with exactly OPTIC_RUST_OK",
            &[],
            &Conversation::default(),
        );
        assert_eq!(body["optic_hermes"], true);
        let reply = ask(key, body, Arc::new(AtomicBool::new(false)), |_| {})
            .await
            .unwrap();
        assert!(reply.contains("OPTIC_RUST_OK"), "{reply}");
    }
    #[test]
    fn sse_split_unicode() {
        let s="data: {\"type\":\"response.output_text.delta\",\"delta\":\"héllo\"}\r\n\r\ndata: [DONE]\n".as_bytes();
        let mut d = SseDecoder::default();
        let mut v = vec![];
        for b in s {
            v.extend(d.push(&[*b]).unwrap());
        }
        assert_eq!(v[0]["delta"], "héllo");
        assert_eq!(v.len(), 1);
    }
    #[test]
    fn no_images_replayed_from_history() {
        let mut h = Conversation::default();
        h.record("question".into(), "answer".into());
        let b = request_body("test", "new", &[], &h);
        assert_eq!(b["store"], false);
        assert_eq!(b["input"].as_array().unwrap().len(), 3);
        assert!(!b.to_string().contains("input_image"));
    }
    #[tokio::test]
    async fn model_list_auth_sort_and_errors() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/models", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for (status, body) in [
                (
                    "200 OK",
                    r#"{"data":[{"id":"z-model"},{"id":"a-model"},{"id":"z-model"}]}"#,
                ),
                (
                    "401 Unauthorized",
                    r#"{"error":{"message":"Invalid test credential"}}"#,
                ),
            ] {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buf = [0u8; 1024];
                while !request.windows(4).any(|s| s == b"\r\n\r\n") {
                    let n = socket.read(&mut buf).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buf[..n]);
                }
                let header = String::from_utf8(request).unwrap().to_lowercase();
                assert!(header.starts_with("get /v1/models "));
                assert!(header.contains("authorization: bearer fixture-key"));
                write!(socket,"HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
        });
        assert_eq!(
            models_at("fixture-key", &url).await.unwrap(),
            vec!["a-model", "z-model"]
        );
        assert!(
            models_at("fixture-key", &url)
                .await
                .unwrap_err()
                .to_string()
                .contains("401: Invalid test credential")
        );
        server.join().unwrap();
    }
    #[tokio::test]
    async fn streams_http_reply_and_checks_image_payload() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/responses", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0; 4096];
            loop {
                let n = socket.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(i) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..i]).to_lowercase();
                    let len: usize = header
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .unwrap()
                        .trim()
                        .parse()
                        .unwrap();
                    if bytes.len() >= i + 4 + len {
                        let body: Value =
                            serde_json::from_slice(&bytes[i + 4..i + 4 + len]).unwrap();
                        let content = body["input"][0]["content"].as_array().unwrap();
                        assert_eq!(
                            content
                                .iter()
                                .filter(|v| v["type"] == "input_image")
                                .count(),
                            5
                        );
                        assert_eq!(body["store"], false);
                        break;
                    }
                }
            }
            let data = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello \"}\n\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"world\"}\n\ndata: {\"type\":\"response.completed\"}\n\n";
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",data.len()).unwrap();
            for chunk in data.as_bytes().chunks(7) {
                socket.write_all(chunk).unwrap();
            }
        });
        let frames: Vec<_> = (0..5)
            .map(|_| Frame {
                jpeg: Arc::new(vec![1, 2, 3]),
                captured: std::time::Instant::now(),
            })
            .collect();
        let body = request_body("test-model", "Help", &frames, &Conversation::default());
        let mut updates = Vec::new();
        let result = ask_at(
            "test-key".into(),
            body,
            Arc::new(AtomicBool::new(false)),
            |s| updates.push(s),
            &url,
        )
        .await
        .unwrap();
        server.join().unwrap();
        assert_eq!(result, "Hello world");
        assert_eq!(updates, vec!["Hello ", "Hello world"]);
    }
    #[tokio::test]
    async fn anthropic_payload_and_stream() {
        use std::io::{Read, Write};
        let frames = vec![Frame {
            jpeg: Arc::new(vec![1, 2, 3]),
            captured: std::time::Instant::now(),
        }];
        let body = provider_body(
            "anthropic",
            "fixture-claude",
            "Help",
            &frames,
            &Conversation::default(),
        );
        assert_eq!(body["messages"][0]["content"][2]["source"]["data"], "AQID");
        assert!(body.get("store").is_none());
        assert_eq!(body["max_tokens"], 1000);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/messages", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = socket.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(i) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..i]).to_lowercase();
                    assert!(header.contains("x-api-key: fixture-key"));
                    assert!(header.contains("anthropic-version: 2023-06-01"));
                    assert!(!header.contains("authorization:"));
                    let len: usize = header
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .unwrap()
                        .trim()
                        .parse()
                        .unwrap();
                    if bytes.len() >= i + 4 + len {
                        break;
                    }
                }
            }
            let data = "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello Claude\"}}\n\ndata: {\"type\":\"message_stop\"}\n\n";
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{data}",data.len()).unwrap();
        });
        assert_eq!(
            ask_at(
                "fixture-key".into(),
                body,
                Arc::new(AtomicBool::new(false)),
                |_| {},
                &url
            )
            .await
            .unwrap(),
            "Hello Claude"
        );
        server.join().unwrap();
    }
}
