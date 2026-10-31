//! `silica-llm-mcp`: a Model Context Protocol server in front of a local
//! model, so bulk text work can be done off the caller's own token budget.
//!
//! This is the binary a user registers in `.mcp.json`:
//!
//! ```json
//! {"mcpServers": {"silica-llm": {"command": "target/release/silica-llm-mcp"}}}
//! ```
//!
//! The transport is stdio, so **stdout carries the protocol and nothing else**
//! — one JSON object per line. Every log line goes to stderr. The tools are in
//! [`tools`], the framing in [`rpc`], the model in [`model`], and [`http`] is
//! the small HTTP client that reaches it.
//!
//! The model is misco's `llama-server` router (`hosts/misco/llm.nix` in the
//! nix flake): one OpenAI-compatible endpoint serving five Qwen presets, which
//! it loads on demand. The wire format is OpenAI's `/v1/chat/completions`, so
//! any other such server works by changing one variable:
//!
//! ```sh
//! SILICA_LLM_URL=http://127.0.0.1:8080/v1                 # on misco (the default)
//! SILICA_LLM_URL=http://misco.tail.ragtux.com:8080/v1     # from the tailnet
//! ```
//!
//! Configuration, all environment variables: `SILICA_LLM_URL`,
//! `SILICA_LLM_MODEL`, `SILICA_LLM_KEY`, `SILICA_LLM_TIMEOUT_MS`,
//! `SILICA_LLM_MAX_BYTES`.
//!
//! Two properties of that router shape how these tools behave, and both belong
//! in the code rather than only in a note. It keeps **one** model resident
//! (`--models-max 1`), so naming a preset other than the loaded one evicts it
//! and costs a 10-30 s reload: pick one model for a run. And it sleeps an idle
//! model after 30 minutes, so the first call after a quiet afternoon pays that
//! reload as well — which is why the default timeout is minutes, not seconds.
//! A slow first call is the model waking, not a hang.

mod http;
mod model;
mod rpc;
mod tools;

use std::io::{BufRead, Write};

fn main() {
    let config = match model::Config::from_env() {
        Ok(config) => config,
        Err(e) => {
            // A bad SILICA_LLM_URL cannot be reported through the handshake,
            // and a server that answers every call with the same error is
            // worse than one that refuses to start.
            eprintln!("silica-llm-mcp: SILICA_LLM_URL: {e}");
            std::process::exit(2);
        }
    };
    // The handshake has no room for this, and a wrong endpoint or model name
    // is the first thing to go wrong, so say both before answering anything.
    eprintln!(
        "silica-llm-mcp {} -> {} (model: {}, timeout {} ms, digest cap {} KiB)",
        env!("CARGO_PKG_VERSION"),
        config.url.origin(),
        config.model,
        config.timeout.as_millis(),
        config.max_bytes / 1024,
    );
    let backend = model::Backend::new(config);
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(e) => {
                eprintln!("silica-llm-mcp: stdin: {e}");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let Some(reply) = rpc::handle_line(&backend, &line) else {
            continue;
        };
        if writeln!(stdout, "{reply}").is_err() || stdout.flush().is_err() {
            eprintln!("silica-llm-mcp: stdout closed");
            break;
        }
    }
}
