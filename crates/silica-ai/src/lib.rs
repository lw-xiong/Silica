//! A chat client for the in-app AI pane, against any OpenAI-compatible
//! `/v1/chat/completions` endpoint (llama-server, Ollama, vLLM, or a hosted
//! API when `SILICA_AI_URL` is `https://`).
//!
//! The crate is transport-plus-shaping only, no gpui: the pane builds a
//! [`ChatRequest`] (optionally with vault notes as context via
//! [`context_messages`]), calls [`chat`], and renders the reply with
//! [`parse_inline`]. The HTTP client lives in `http.rs` and mirrors
//! `silica-llm-mcp`'s: one blocking request per call, loopback-friendly,
//! no async runtime.

mod http;

use std::time::Duration;

use serde::Serialize;

pub const DEFAULT_URL: &str = "http://127.0.0.1:8080/v1";
pub const DEFAULT_MODEL: &str = "qwen3.6-35b-a3b-code";

/// Where to reach the model server and how to call it. The pane reads this
/// from the environment once per send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Base URL, e.g. `http://127.0.0.1:8080/v1`; `/chat/completions` is
    /// appended.
    pub url: String,
    pub model: String,
    pub key: Option<String>,
    pub timeout: Duration,
    /// Cap on the response body, so a runaway server cannot OOM the app.
    pub max_bytes: usize,
}

impl Config {
    /// `SILICA_AI_URL`, `SILICA_AI_MODEL`, `SILICA_AI_KEY`,
    /// `SILICA_AI_TIMEOUT_MS`, `SILICA_AI_MAX_BYTES` — the `SILICA_LLM_*`
    /// names of `silica-llm-mcp` with an `AI` infix, so the two tools can
    /// point at different servers.
    pub fn from_env() -> Result<Config, String> {
        Ok(Config {
            url: env_text("SILICA_AI_URL").unwrap_or_else(|| DEFAULT_URL.to_string()),
            model: env_text("SILICA_AI_MODEL").unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            key: env_text("SILICA_AI_KEY"),
            timeout: Duration::from_millis(
                env_text("SILICA_AI_TIMEOUT_MS")
                    .and_then(|ms| ms.parse().ok())
                    .unwrap_or(300_000),
            ),
            max_bytes: env_text("SILICA_AI_MAX_BYTES")
                .and_then(|n| n.parse().ok())
                .unwrap_or(4 << 20),
        })
    }
}

fn env_text(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

/// One `/v1/chat/completions` call.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

impl ChatRequest {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

/// The system message that grounds a chat in vault notes: one system turn
/// holding up to `max_chars` of `path`/`text` pairs.
pub fn context_messages(notes: &[(String, String)], max_chars: usize) -> Vec<ChatMessage> {
    let mut body = String::new();
    for (path, text) in notes {
        let chunk = format!("--- {path} ---\n{text}\n");
        let room = max_chars.saturating_sub(body.len());
        if chunk.len() <= room {
            body.push_str(&chunk);
        } else if room > 0 {
            body.extend(chunk.chars().take(room));
            break;
        } else {
            break;
        }
    }
    vec![ChatMessage::system(format!(
        "You are the assistant inside Silica, a note-taking app. Answer from the \
         notes below when they are relevant and cite them as [[Note]]. Keep answers \
         short.\n\nNotes:\n{body}"
    ))]
}

/// A parsed reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatReply {
    pub text: String,
    pub model: String,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
}

/// Parses a `/v1/chat/completions` response body; an `error` object is an
/// `Err` with the server's message.
pub fn parse_chat_response(body: &str) -> Result<ChatReply, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("response is not JSON: {e}"))?;
    if let Some(error) = value.get("error") {
        let message = error
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown server error");
        return Err(message.to_string());
    }
    let text = value
        .pointer("/choices/0/message/content")
        .and_then(|c| c.as_str())
        .ok_or_else(|| "response has no choices[0].message.content".to_string())?
        .to_string();
    let model = value
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or_default()
        .to_string();
    let usage = value.get("usage");
    Ok(ChatReply {
        text,
        model,
        prompt_tokens: usage
            .and_then(|u| u.get("prompt_tokens"))
            .and_then(|t| t.as_u64()),
        completion_tokens: usage
            .and_then(|u| u.get("completion_tokens"))
            .and_then(|t| t.as_u64()),
    })
}

/// Sends `request` to `config`'s `/chat/completions` and parses the reply.
/// Blocking; the pane runs it on the background executor.
pub fn chat(config: &Config, request: &ChatRequest) -> Result<ChatReply, String> {
    let url = http::Url::parse(&config.url)?;
    let body = serde_json::to_string(&request.to_json()).map_err(|e| e.to_string())?;
    let path = url.route("/chat/completions");
    let response = http::request(
        &url,
        &path,
        Some(&body),
        config.key.as_deref(),
        config.timeout,
        config.max_bytes,
    )?;
    if !(200..300).contains(&response.status) {
        let detail = String::from_utf8_lossy(&response.body);
        let detail: String = detail.chars().take(500).collect();
        return Err(format!("HTTP {}: {detail}", response.status));
    }
    parse_chat_response(&String::from_utf8_lossy(&response.body))
}

/// One inline run of an assistant reply, for the pane to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Text(String),
    /// `[[Note]]` → the link target.
    Link(String),
    /// `` `code` ``
    Code(String),
    /// `**bold**`
    Bold(String),
    /// `*italic*`
    Italic(String),
    /// ` ```lang …``` ` → (lang, code)
    CodeBlock(String, String),
}

/// Splits a reply into inline segments: code fences first, then `[[links]]`,
/// `` `code` ``, `**bold**`, `*italic*`. An unclosed marker is plain text.
pub fn parse_inline(text: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("```") {
        push_inline(&mut out, &rest[..start]);
        let after = &rest[start + 3..];
        let (lang, body) = match after.find('\n') {
            Some(nl) => (after[..nl].trim().to_string(), &after[nl + 1..]),
            None => (String::new(), after),
        };
        match body.find("```") {
            Some(end) => {
                out.push(Segment::CodeBlock(lang, body[..end].trim_end().to_string()));
                rest = &body[end + 3..];
            }
            None => {
                out.push(Segment::CodeBlock(lang, body.to_string()));
                rest = "";
            }
        }
    }
    push_inline(&mut out, rest);
    out
}

const INLINE_MARKERS: [(&str, &str); 4] = [
    ("[[", "]]"),
    ("`", "`"),
    ("**", "**"),
    ("*", "*"),
];

fn push_inline(out: &mut Vec<Segment>, text: &str) {
    let mut rest = text;
    while !rest.is_empty() {
        // * Earliest marker wins; `**` is listed before `*` so bold is not
        //   read as two italics.
        let mut best: Option<(usize, usize)> = None;
        for (ix, (open, _)) in INLINE_MARKERS.iter().enumerate() {
            if let Some(pos) = rest.find(open) {
                if best.is_none_or(|(best_pos, _)| pos < best_pos) {
                    best = Some((pos, ix));
                }
            }
        }
        let Some((pos, kind)) = best else {
            out.push(Segment::Text(rest.to_string()));
            return;
        };
        if pos > 0 {
            out.push(Segment::Text(rest[..pos].to_string()));
        }
        let (open, close) = INLINE_MARKERS[kind];
        let after_open = &rest[pos + open.len()..];
        match after_open.find(close) {
            Some(end) if end > 0 => {
                let inner = after_open[..end].to_string();
                out.push(match kind {
                    0 => Segment::Link(inner),
                    1 => Segment::Code(inner),
                    2 => Segment::Bold(inner),
                    _ => Segment::Italic(inner),
                });
                rest = &after_open[end + close.len()..];
            }
            _ => {
                out.push(Segment::Text(rest[pos..].to_string()));
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn request_serializes_to_the_openai_shape() {
        let request = ChatRequest {
            model: "m".to_string(),
            messages: vec![
                ChatMessage::system("sys"),
                ChatMessage::user("hi"),
            ],
            temperature: Some(0.2),
            max_tokens: None,
        };
        let json = request.to_json();
        assert_eq!(json["model"], "m");
        assert_eq!(json["messages"][0]["role"], "system");
        assert_eq!(json["messages"][1]["content"], "hi");
        assert!((json["temperature"].as_f64().unwrap() - 0.2).abs() < 1e-6);
        assert!(json.get("max_tokens").is_none(), "None fields are skipped");
    }

    #[test]
    fn parses_a_reply_with_usage() {
        let body = r#"{"model":"m","choices":[{"message":{"role":"assistant","content":"pong"}}],"usage":{"prompt_tokens":3,"completion_tokens":1}}"#;
        let reply = parse_chat_response(body).expect("parses");
        assert_eq!(reply.text, "pong");
        assert_eq!(reply.model, "m");
        assert_eq!(reply.prompt_tokens, Some(3));
        assert_eq!(reply.completion_tokens, Some(1));
    }

    #[test]
    fn an_error_object_is_an_err_with_the_message() {
        let body = r#"{"error":{"message":"model not loaded"}}"#;
        assert_eq!(parse_chat_response(body), Err("model not loaded".to_string()));
        assert!(parse_chat_response("not json").is_err());
        assert!(parse_chat_response("{}").is_err());
    }

    #[test]
    fn inline_segments_cover_links_code_bold_italic_and_fences() {
        let segments = parse_inline("See [[Note A]] and `x = 1` plus **bold** and *it*, then:\n```rust\nlet a = 1;\n```\ndone");
        assert_eq!(
            segments,
            vec![
                Segment::Text("See ".to_string()),
                Segment::Link("Note A".to_string()),
                Segment::Text(" and ".to_string()),
                Segment::Code("x = 1".to_string()),
                Segment::Text(" plus ".to_string()),
                Segment::Bold("bold".to_string()),
                Segment::Text(" and ".to_string()),
                Segment::Italic("it".to_string()),
                Segment::Text(", then:\n".to_string()),
                Segment::CodeBlock("rust".to_string(), "let a = 1;".to_string()),
                Segment::Text("\ndone".to_string()),
            ]
        );
    }

    #[test]
    fn an_unclosed_marker_stays_plain_text() {
        assert_eq!(
            parse_inline("a **b"),
            vec![Segment::Text("a ".to_string()), Segment::Text("**b".to_string())]
        );
    }

    #[test]
    fn context_caps_at_max_chars() {
        let notes = vec![
            ("A.md".to_string(), "x".repeat(100)),
            ("B.md".to_string(), "y".repeat(100)),
        ];
        let messages = context_messages(&notes, 150);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, Role::System);
        assert!(messages[0].content.contains("--- A.md ---"));
        assert!(messages[0].content.contains("--- B.md ---"), "the cap fits a prefix of the second note");
        assert!(messages[0].content.len() <= 150 + 200, "system text plus at most max_chars of notes");
    }

    /// A loopback server that answers one chat call with a canned reply and
    /// records the request body it saw.
    fn loopback(reply_status: &str, reply_body: &str) -> (u16, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let status = reply_status.to_string();
        let body = reply_body.to_string();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            let head_end = loop {
                if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break pos;
                }
                let n = stream.read(&mut chunk).expect("read");
                if n == 0 {
                    panic!("closed before headers");
                }
                buf.extend_from_slice(&chunk[..n]);
            };
            let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
            let length = head
                .lines()
                .find_map(|l| l.split_once(':').filter(|(n, _)| n.trim().eq_ignore_ascii_case("content-length")).map(|(_, v)| v.trim().to_string()))
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0);
            let mut body_bytes = buf[head_end + 4..].to_vec();
            while body_bytes.len() < length {
                let n = stream.read(&mut chunk).expect("read body");
                if n == 0 {
                    break;
                }
                body_bytes.extend_from_slice(&chunk[..n]);
            }
            let request_body = String::from_utf8_lossy(&body_bytes).to_string();
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .expect("write response");
            format!("{head}\n{request_body}")
        });
        (port, handle)
    }

    fn test_config(port: u16) -> Config {
        Config {
            url: format!("http://127.0.0.1:{port}/v1"),
            model: "test-model".to_string(),
            key: Some("secret".to_string()),
            timeout: Duration::from_secs(5),
            max_bytes: 1 << 20,
        }
    }

    #[test]
    fn chat_posts_and_parses_against_a_loopback_server() {
        let (port, server) = loopback(
            "200 OK",
            r#"{"model":"test-model","choices":[{"message":{"role":"assistant","content":"pong"}}],"usage":{"prompt_tokens":3,"completion_tokens":1}}"#,
        );
        let request = ChatRequest {
            model: "test-model".to_string(),
            messages: vec![
                ChatMessage::system("sys"),
                ChatMessage::user("hello"),
            ],
            temperature: None,
            max_tokens: None,
        };
        let reply = chat(&test_config(port), &request).expect("chat succeeds");
        assert_eq!(reply.text, "pong");
        assert_eq!(reply.prompt_tokens, Some(3));
        let seen = server.join().expect("server thread");
        assert!(seen.contains("POST /v1/chat/completions"), "the path: {seen}");
        assert!(seen.contains("\"model\":\"test-model\""), "the model: {seen}");
        assert!(seen.contains("hello"), "the message: {seen}");
        assert!(seen.contains("Authorization: Bearer secret"), "the key: {seen}");
    }

    #[test]
    fn chat_surfaces_a_server_error_body() {
        let (port, server) = loopback("500 Internal Server Error", r#"{"error":{"message":"boom"}}"#);
        let request = ChatRequest {
            model: "test-model".to_string(),
            messages: vec![ChatMessage::user("hello")],
            temperature: None,
            max_tokens: None,
        };
        let err = chat(&test_config(port), &request).expect_err("a 500 is an error");
        assert!(err.contains("500"), "the status is in the error: {err}");
        server.join().expect("server thread");
    }
}
