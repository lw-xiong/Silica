//! The tool table: what a model may ask the local model to do.
//!
//! Three tools, and the split between them is about whose context pays. The
//! caller's own context pays for `llm_generate`, because the prompt and the
//! answer both pass through it. It does not pay for `llm_digest`: the file is
//! read here, inside this process, and only the summary crosses back. That is
//! the whole point of the tool — a 40,000-line `cargo test` log can be reduced
//! to a paragraph without ever entering the conversation.

use crate::model::{Answer, Ask, Backend};
use crate::rpc::{Fault, INVALID_PARAMS};
use serde_json::{json, Map, Value};

enum Run {
    Models,
    Generate,
    Digest,
}

struct Tool {
    name: &'static str,
    description: &'static str,
    schema: fn() -> Value,
    run: Run,
}

const TOOLS: &[Tool] = &[
    Tool {
        name: "llm_models",
        description: "\
List the presets the local llama-server router is serving, and say where it \
is. Call this first, and whenever a generate fails: it is what separates \
`nothing is listening on that port` from `no such preset`. The router keeps \
one model resident (`--models-max 1`) and sleeps it after 30 minutes idle, so \
naming a different preset than the resident one costs a 10-30 s reload and \
evicts the other. Takes no arguments.",
        schema: || json!({"type": "object", "properties": {}}),
        run: Run::Models,
    },
    Tool {
        name: "llm_generate",
        description: "\
Send one prompt to the local model and return its answer. Use it for bulk, \
low-judgment text work that would otherwise cost the caller's own tokens: \
generating candidate test fixtures, drafting many variations of a string, \
classifying a list, rewriting prose. Do NOT use it for anything that must be \
exact — a command id, a config key, a file path, an API signature — because a \
small local model will produce a plausible wrong string and nothing in the \
answer will mark it as a guess. The answer is prefixed with a one-line \
provenance note giving the model, the token counts and the rate. A \
`<think>` block is stripped before the answer is returned.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "prompt": {"type": "string",
                        "description": "The instruction, with everything the model needs: it sees no other context."},
                    "system": {"type": "string",
                        "description": "System message; sent only when given."},
                    "model": {"type": "string",
                        "description": "Override SILICA_LLM_MODEL for this one call. Use llm_models for the names."},
                    "temperature": {"type": "number",
                        "description": "0 for a deterministic answer, 0.8-1.0 when you want variety (generating fixtures). The server's default when omitted."},
                    "max_tokens": {"type": "number",
                        "description": "Cap the completion. Worth setting: an unbounded local generation can run for minutes."},
                    "json": {"type": "boolean",
                        "description": "Ask the server for a single JSON object. llama-server enforces this with a grammar, so the shape is guaranteed."},
                    "enable_thinking": {"type": "boolean",
                        "description": "Qwen's thinking mode. The `-code` presets have it off already; pass false for `qwen3.8-27b` or `qwen3.6-35b-a3b`, which think by default and will otherwise spend most of the budget on it."},
                },
                "required": ["prompt"],
            })
        },
        run: Run::Generate,
    },
    Tool {
        name: "llm_digest",
        description: "\
Read a file here and return only the local model's summary of it, so the file \
itself never enters the caller's context. This is the tool to reach for when \
something has produced far more output than is worth reading: a `cargo test` \
failure wall, a clippy flood, a corpus diff, an Obsidian probe's raw JSON. \
`instruction` says what to pull out — be specific, because a small model \
summarizing without a question gives back a vague paragraph. `contains` \
pre-filters to matching lines before the model sees anything, which is often \
what makes a large log fit at all; prefer it over asking the model to ignore \
the noise. Remember that a summary can drop the one line that mattered: when \
the answer names a specific failure, read that part of the file directly.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string",
                        "description": "File to read, absolute or relative to this server's working directory."},
                    "instruction": {"type": "string",
                        "description": "What to extract, e.g. `list each distinct test failure once, with the assertion and the file:line`."},
                    "contains": {"type": "string",
                        "description": "Keep only lines containing this substring (case-insensitive) before the model sees the file."},
                    "tail": {"type": "boolean",
                        "description": "When the file is larger than the byte cap, keep the end rather than the beginning. True for a log whose failure summary is last."},
                    "model": {"type": "string", "description": "Override SILICA_LLM_MODEL for this call."},
                    "max_tokens": {"type": "number", "description": "Cap the summary's length."},
                },
                "required": ["path", "instruction"],
            })
        },
        run: Run::Digest,
    },
];

pub fn list() -> Value {
    let tools: Vec<Value> = TOOLS
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": (tool.schema)(),
            })
        })
        .collect();
    json!({ "tools": tools })
}

/// Run one `tools/call`. The `Err` arm is for a malformed request only: a
/// model server that refused is an `isError` result the caller can read.
pub fn call(backend: &Backend, params: &Value) -> Result<Value, Fault> {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return Err(Fault::new(INVALID_PARAMS, "tools/call needs a `name`"));
    };
    let Some(tool) = TOOLS.iter().find(|tool| tool.name == name) else {
        return Err(Fault::new(INVALID_PARAMS, format!("unknown tool: {name}")));
    };
    let arguments = match params.get("arguments") {
        None | Some(Value::Null) => Value::Object(Map::new()),
        Some(arguments) if arguments.is_object() => arguments.clone(),
        Some(_) => return Err(Fault::new(INVALID_PARAMS, "`arguments` must be an object")),
    };
    Ok(match run(backend, tool, &arguments) {
        Ok(text_out) => json!({"content": [text(text_out)], "isError": false}),
        Err(message) => json!({"content": [text(message)], "isError": true}),
    })
}

fn run(backend: &Backend, tool: &Tool, arguments: &Value) -> Result<String, String> {
    match tool.run {
        Run::Models => {
            let names = backend.models()?;
            let origin = backend.config.url.origin();
            if names.is_empty() {
                return Ok(format!(
                    "{origin} is up but serving no models; check \
                     `modelsPreset` in hosts/misco/llm.nix and that the GGUFs \
                     are in /var/lib/llm-models (wanted `{}`).",
                    backend.config.model
                ));
            }
            Ok(format!(
                "{origin}, default `{}`{}\n{}",
                backend.config.model,
                if names.contains(&backend.config.model) {
                    String::new()
                } else {
                    format!(
                        " (WARNING: `{}` is not in this list; every call will fail \
                         until SILICA_LLM_MODEL names one that is)",
                        backend.config.model
                    )
                },
                names
                    .iter()
                    .map(|name| format!("  {name}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ))
        }
        Run::Generate => {
            let prompt = required_str(arguments, "prompt")?;
            let answer = backend.chat(&Ask {
                prompt,
                system: str_arg(arguments, "system"),
                model: str_arg(arguments, "model"),
                temperature: arguments.get("temperature").and_then(Value::as_f64),
                max_tokens: arguments.get("max_tokens").and_then(Value::as_u64),
                json: arguments
                    .get("json")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                enable_thinking: arguments.get("enable_thinking").and_then(Value::as_bool),
            })?;
            Ok(with_provenance(&answer))
        }
        Run::Digest => {
            let path = required_str(arguments, "path")?;
            let instruction = required_str(arguments, "instruction")?;
            let excerpt = read_excerpt(
                backend,
                path,
                str_arg(arguments, "contains"),
                arguments
                    .get("tail")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            )?;
            let prompt = format!(
                "{instruction}\n\nBelow is the content of `{path}`{}. Answer only \
                 from it; say so if it does not contain what was asked for. Quote \
                 identifiers, paths and numbers exactly as they appear — never \
                 reconstruct one from memory.\n\n----- begin {path} -----\n{}\n\
                 ----- end {path} -----",
                excerpt.note, excerpt.body,
            );
            let answer = backend.chat(&Ask {
                prompt: &prompt,
                system: Some(
                    "You summarize developer output. Be terse and concrete. \
                     Prefer a list of distinct findings over prose. Copy every \
                     identifier, path and number verbatim.",
                ),
                model: str_arg(arguments, "model"),
                temperature: Some(0.0),
                max_tokens: arguments.get("max_tokens").and_then(Value::as_u64),
                json: false,
                // A digest wants the answer, not the deliberation, and the
                // general presets think by default.
                enable_thinking: Some(false),
            })?;
            Ok(format!(
                "{}\n{}",
                excerpt.note.trim(),
                with_provenance(&answer)
            ))
        }
    }
}

fn with_provenance(answer: &Answer) -> String {
    if answer.text.is_empty() {
        return format!(
            "{}\nThe model returned nothing. If the whole budget went to a \
             reasoning block, raise max_tokens or pick a model without one.",
            answer.provenance()
        );
    }
    format!("{}\n{}", answer.provenance(), answer.text)
}

#[derive(Debug)]
struct Excerpt {
    body: String,
    /// What was done to the file, for the caller: a summary of a filtered,
    /// truncated file must never look like a summary of the whole thing.
    note: String,
}

fn read_excerpt(
    backend: &Backend,
    path: &str,
    contains: Option<&str>,
    tail: bool,
) -> Result<Excerpt, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let whole = String::from_utf8_lossy(&bytes);
    let total_lines = whole.lines().count();
    let mut note = format!("[{path}: {} bytes, {total_lines} lines", bytes.len());

    let filtered = match contains.filter(|needle| !needle.is_empty()) {
        Some(needle) => {
            let needle = needle.to_lowercase();
            let kept: Vec<&str> = whole
                .lines()
                .filter(|line| line.to_lowercase().contains(&needle))
                .collect();
            if kept.is_empty() {
                return Err(format!(
                    "{path} has {total_lines} lines and none contain {needle:?}; \
                     nothing to summarize"
                ));
            }
            note.push_str(&format!(
                "; {} lines contain {:?}",
                kept.len(),
                contains.unwrap_or_default()
            ));
            kept.join("\n")
        }
        None => whole.into_owned(),
    };

    let max = backend.config.max_bytes;
    let body = if filtered.len() <= max {
        filtered
    } else {
        note.push_str(&format!(
            "; truncated to the {} {max} bytes",
            if tail { "last" } else { "first" }
        ));
        if tail {
            let mut start = filtered.len() - max;
            while start < filtered.len() && !filtered.is_char_boundary(start) {
                start += 1;
            }
            filtered[start..].to_string()
        } else {
            let mut end = max;
            while end > 0 && !filtered.is_char_boundary(end) {
                end -= 1;
            }
            filtered[..end].to_string()
        }
    };
    note.push(']');
    Ok(Excerpt { body, note })
}

fn required_str<'a>(arguments: &'a Value, name: &str) -> Result<&'a str, String> {
    str_arg(arguments, name).ok_or_else(|| format!("`{name}` is required and must be a string"))
}

fn str_arg<'a>(arguments: &'a Value, name: &str) -> Option<&'a str> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

fn text(body: impl Into<String>) -> Value {
    json!({"type": "text", "text": body.into()})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Config;

    fn backend_with(max_bytes: usize) -> Backend {
        let mut config = Config::from_env().expect("the default config parses");
        config.max_bytes = max_bytes;
        Backend::new(config)
    }

    #[test]
    fn every_tool_has_a_schema_and_a_description() {
        let listed = list();
        let tools = listed["tools"].as_array().unwrap();
        assert_eq!(tools.len(), TOOLS.len());
        for tool in tools {
            assert!(!tool["description"].as_str().unwrap().is_empty());
            assert_eq!(tool["inputSchema"]["type"], "object");
        }
    }

    #[test]
    fn an_unknown_tool_and_bad_arguments_are_faults() {
        let backend = backend_with(1 << 10);
        assert_eq!(
            call(&backend, &json!({"name": "llm_dance"}))
                .unwrap_err()
                .code,
            INVALID_PARAMS
        );
        assert_eq!(call(&backend, &json!({})).unwrap_err().code, INVALID_PARAMS);
        assert_eq!(
            call(&backend, &json!({"name": "llm_generate", "arguments": 7}))
                .unwrap_err()
                .code,
            INVALID_PARAMS
        );
    }

    #[test]
    fn a_missing_prompt_is_a_readable_tool_error_not_a_fault() {
        let backend = backend_with(1 << 10);
        let result = call(&backend, &json!({"name": "llm_generate", "arguments": {}})).unwrap();
        assert_eq!(result["isError"], json!(true));
        assert!(result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("`prompt` is required"));
    }

    #[test]
    fn digest_filters_and_truncates_and_says_so() {
        let dir = std::env::temp_dir().join("silica-llm-mcp-digest");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("log.txt");
        std::fs::write(&path, "ok one\nFAILED two\nok three\nfailed four\n").unwrap();
        let path = path.to_string_lossy().into_owned();

        let whole = read_excerpt(&backend_with(1 << 20), &path, None, false).unwrap();
        assert!(whole.note.contains("4 lines"), "{}", whole.note);
        assert!(whole.body.contains("ok one"));

        // The filter is case-insensitive, and the note records the count.
        let only = read_excerpt(&backend_with(1 << 20), &path, Some("failed"), false).unwrap();
        assert_eq!(only.body, "FAILED two\nfailed four");
        assert!(only.note.contains("2 lines contain"), "{}", only.note);

        // Truncation says which end was kept.
        let head = read_excerpt(&backend_with(10), &path, None, false).unwrap();
        assert!(head.note.contains("first 10 bytes"), "{}", head.note);
        assert_eq!(head.body.len(), 10);
        let tail = read_excerpt(&backend_with(10), &path, None, true).unwrap();
        assert!(tail.note.contains("last 10 bytes"), "{}", tail.note);
        assert!(tail.body.ends_with("four\n"));

        // A filter that matches nothing is an error, not an empty summary.
        let none = read_excerpt(&backend_with(1 << 20), &path, Some("zzz"), false);
        assert!(none.unwrap_err().contains("none contain"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_file_is_reported_with_its_path() {
        let e =
            read_excerpt(&backend_with(1 << 10), "/nonexistent/log.txt", None, false).unwrap_err();
        assert!(e.contains("/nonexistent/log.txt"), "{e}");
    }
}
