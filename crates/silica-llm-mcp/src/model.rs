//! The model behind the tools: where it lives, and one blocking chat call.
//!
//! The wire format is OpenAI's `/v1/chat/completions`, because that is the one
//! shape Ollama, `llama-server` and vLLM all serve — so moving from one to
//! another is `SILICA_LLM_URL`, not a new code path. Streaming is deliberately
//! off: a tool call returns once, and a partial completion is of no use to a
//! caller that cannot see it arrive.

use crate::http::{self, Url};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// Everything read from the environment, once, at startup. An MCP server is
/// configured in a `.mcp.json` entry, and that is the only place a user can
/// reach it.
pub struct Config {
    /// `SILICA_LLM_URL`: the OpenAI-compatible base, default Ollama's.
    pub url: Url,
    /// `SILICA_LLM_MODEL`: the default model, overridable per call.
    pub model: String,
    /// `SILICA_LLM_KEY`: a bearer token, for a server that wants one.
    pub key: Option<String>,
    /// `SILICA_LLM_TIMEOUT_MS`: generous, because a long completion on one
    /// consumer GPU is minutes, not seconds.
    pub timeout: Duration,
    /// `SILICA_LLM_MAX_BYTES`: how much of a file `llm_digest` may read.
    pub max_bytes: usize,
}

pub const DEFAULT_URL: &str = "http://127.0.0.1:8080/v1";
pub const DEFAULT_MODEL: &str = "qwen3.6-35b-a3b-code";

impl Config {
    pub fn from_env() -> Result<Config, String> {
        Ok(Config {
            url: Url::parse(&env_text("SILICA_LLM_URL").unwrap_or_else(|| DEFAULT_URL.into()))?,
            model: env_text("SILICA_LLM_MODEL").unwrap_or_else(|| DEFAULT_MODEL.into()),
            key: env_text("SILICA_LLM_KEY"),
            timeout: Duration::from_millis(
                env_text("SILICA_LLM_TIMEOUT_MS")
                    .and_then(|ms| ms.parse().ok())
                    .unwrap_or(300_000),
            ),
            max_bytes: env_text("SILICA_LLM_MAX_BYTES")
                .and_then(|n| n.parse().ok())
                .unwrap_or(256 << 10),
        })
    }
}

fn env_text(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// One completion request.
pub struct Ask<'a> {
    pub prompt: &'a str,
    pub system: Option<&'a str>,
    pub model: Option<&'a str>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u64>,
    /// Ask the server for a JSON object. `llama-server` turns this into a
    /// grammar, so the shape is enforced rather than requested.
    pub json: bool,
    /// Qwen's thinking mode, through `chat_template_kwargs`. The coding
    /// presets already set `reasoning = off`, so this is for the general
    /// presets (`qwen3.8-27b`, `qwen3.6-35b-a3b`), which think by default.
    pub enable_thinking: Option<bool>,
}

/// What came back, with the numbers that say whether the local model was
/// worth the wall clock.
pub struct Answer {
    pub text: String,
    pub model: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub elapsed: Duration,
    /// A reasoning block that was stripped off the front of `text`, if any.
    pub thought_bytes: usize,
}

impl Answer {
    /// The one-line provenance that goes above every answer, so a caller can
    /// see which model spoke and how long it took.
    pub fn provenance(&self) -> String {
        let seconds = self.elapsed.as_secs_f64();
        // Below a few tens of milliseconds the rate is division noise, not a
        // measurement, so it is left off rather than printed as six figures.
        let rate = if seconds >= 0.05 {
            format!(" · {:.1} tok/s", self.completion_tokens as f64 / seconds)
        } else {
            String::new()
        };
        let thought = if self.thought_bytes > 0 {
            format!(", {} bytes of <think> dropped", self.thought_bytes)
        } else {
            String::new()
        };
        format!(
            "[{} · {} prompt + {} completion tokens · {seconds:.1} s{rate}{thought}]",
            self.model, self.prompt_tokens, self.completion_tokens,
        )
    }
}

pub struct Backend {
    pub config: Config,
}

impl Backend {
    pub fn new(config: Config) -> Backend {
        Backend { config }
    }

    /// What the server is serving. The first thing to call when a generate
    /// fails: it separates "nothing is listening" from "that model is not
    /// pulled".
    pub fn models(&self) -> Result<Vec<String>, String> {
        let route = self.config.url.route("/models");
        let response = http::request(
            &self.config.url,
            &route,
            None,
            self.config.key.as_deref(),
            self.config.timeout,
        )?;
        if !response.ok() {
            return Err(format!(
                "{} answered {} for {route}: {}",
                self.config.url.origin(),
                response.status,
                response.body.trim()
            ));
        }
        let value: Value = serde_json::from_str(&response.body)
            .map_err(|e| format!("{route} did not answer JSON: {e}"))?;
        let mut names: Vec<String> = value["data"]
            .as_array()
            .map(|models| {
                models
                    .iter()
                    .filter_map(|model| model["id"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        Ok(names)
    }

    pub fn chat(&self, ask: &Ask) -> Result<Answer, String> {
        let model = ask.model.unwrap_or(&self.config.model);
        let mut messages = Vec::new();
        if let Some(system) = ask.system.filter(|s| !s.trim().is_empty()) {
            messages.push(json!({"role": "system", "content": system}));
        }
        messages.push(json!({"role": "user", "content": ask.prompt}));
        let mut body = json!({
            "model": model,
            "messages": messages,
            "stream": false,
        });
        // Only send what the caller set: a server's own default is a better
        // guess than one invented here.
        if let Some(temperature) = ask.temperature {
            body["temperature"] = json!(temperature);
        }
        if let Some(max_tokens) = ask.max_tokens {
            body["max_tokens"] = json!(max_tokens);
        }
        if ask.json {
            body["response_format"] = json!({"type": "json_object"});
        }
        if let Some(enable_thinking) = ask.enable_thinking {
            body["chat_template_kwargs"] = json!({"enable_thinking": enable_thinking});
        }

        let route = self.config.url.route("/chat/completions");
        let started = Instant::now();
        let response = http::request(
            &self.config.url,
            &route,
            Some(&body.to_string()),
            self.config.key.as_deref(),
            self.config.timeout,
        )?;
        let elapsed = started.elapsed();
        let value: Value = serde_json::from_str(&response.body).map_err(|e| {
            format!(
                "{route} answered {} but not JSON ({e}): {}",
                response.status,
                truncate(response.body.trim(), 400)
            )
        })?;
        if !response.ok() {
            // Every one of the three servers puts its complaint somewhere
            // different; show whichever is there.
            let detail = value["error"]["message"]
                .as_str()
                .or_else(|| value["error"].as_str())
                .or_else(|| value["message"].as_str())
                .unwrap_or_else(|| response.body.trim());
            return Err(format!(
                "{} answered {}: {}",
                self.config.url.origin(),
                response.status,
                truncate(detail, 400)
            ));
        }
        let raw = value["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| {
                format!(
                    "no completion in the answer: {}",
                    truncate(&response.body, 400)
                )
            })?;
        let (text, thought_bytes) = strip_thought(raw);
        Ok(Answer {
            text,
            model: value["model"].as_str().unwrap_or(model).to_string(),
            prompt_tokens: value["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
            completion_tokens: value["usage"]["completion_tokens"].as_u64().unwrap_or(0),
            elapsed,
            thought_bytes,
        })
    }
}

/// Drop a leading `<think>…</think>` block, which the Qwen3 family emits
/// inline. The reasoning is not the answer, and leaving it in makes every
/// result the caller parses start with a paragraph of deliberation.
fn strip_thought(raw: &str) -> (String, usize) {
    let trimmed = raw.trim_start();
    if !trimmed.starts_with("<think>") {
        return (raw.trim().to_string(), 0);
    }
    match trimmed.find("</think>") {
        Some(at) => (
            trimmed[at + "</think>".len()..].trim().to_string(),
            at + "</think>".len(),
        ),
        // An unterminated block means the model spent its whole budget
        // thinking: say so rather than returning an empty answer.
        None => (String::new(), trimmed.len()),
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… ({} bytes)", &text[..end], text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_a_reasoning_block() {
        let (text, dropped) = strip_thought("<think>hmm, let me see</think>\n\nthe answer");
        assert_eq!(text, "the answer");
        assert!(dropped > 0);

        let (text, dropped) = strip_thought("  plain answer  ");
        assert_eq!((text.as_str(), dropped), ("plain answer", 0));

        // `<think>` that never closes leaves nothing to return.
        let (text, dropped) = strip_thought("<think>and on and on");
        assert!(text.is_empty());
        assert_eq!(dropped, "<think>and on and on".len());
    }

    #[test]
    fn a_think_block_later_in_the_text_is_left_alone() {
        let (text, dropped) = strip_thought("use <think> in HTML</think> like so");
        assert_eq!(text, "use <think> in HTML</think> like so");
        assert_eq!(dropped, 0);
    }

    #[test]
    fn truncate_keeps_char_boundaries() {
        assert_eq!(truncate("abc", 10), "abc");
        assert!(truncate("é".repeat(100).as_str(), 7).starts_with("ééé"));
    }

    #[test]
    fn provenance_reports_the_rate() {
        let answer = Answer {
            text: String::new(),
            model: "qwen3:8b".into(),
            prompt_tokens: 100,
            completion_tokens: 50,
            elapsed: Duration::from_secs(2),
            thought_bytes: 0,
        };
        let line = answer.provenance();
        assert!(line.contains("qwen3:8b"), "{line}");
        assert!(line.contains("25.0 tok/s"), "{line}");

        // A call too fast to measure reports no rate rather than six figures.
        let instant = Answer {
            elapsed: Duration::from_millis(1),
            ..answer
        };
        assert!(
            !instant.provenance().contains("tok/s"),
            "{}",
            instant.provenance()
        );
    }
}
