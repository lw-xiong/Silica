//! `silica-mcp`: a Model Context Protocol server that launches Silica and
//! drives it through the control socket.
//!
//! This is the binary a user registers in `.mcp.json`:
//!
//! ```json
//! {"mcpServers": {"silica": {"command": "target/release/silica-mcp"}}}
//! ```
//!
//! The transport is stdio, so **stdout carries the protocol and nothing else**
//! — one JSON object per line. Every log line, and everything the app itself
//! prints, goes to stderr. The tools are in [`tools`], the framing in [`rpc`],
//! and the child process in [`app`]; the protocol the socket speaks is
//! `silica-control`, which this server shares with the app so a tool's
//! parameters cannot drift from the method behind it.
//!
//! Configuration, all environment variables: `SILICA_MCP_SOCKET`,
//! `SILICA_MCP_BIN`, `SILICA_MCP_REPO`, `SILICA_MCP_SCREENSHOT`,
//! `SILICA_MCP_TIMEOUT_MS`.

mod app;
mod base64;
mod rpc;
mod tools;

use std::io::{BufRead, Write};

fn main() {
    let config = app::Config::from_env();
    // The handshake has no room for this, and a wrong binary or socket is the
    // first thing to go wrong, so say both before answering anything.
    eprintln!(
        "silica-mcp {} on {} (app: {}, timeout {} ms)",
        env!("CARGO_PKG_VERSION"),
        config.socket.display(),
        config.bin,
        config.timeout.as_millis()
    );
    let mut app = app::App::new(config);
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(e) => {
                eprintln!("silica-mcp: stdin: {e}");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let Some(reply) = rpc::handle_line(&mut app, &line) else {
            continue;
        };
        if writeln!(stdout, "{reply}").is_err() || stdout.flush().is_err() {
            eprintln!("silica-mcp: stdout closed");
            break;
        }
    }
    // Dropping the app quits the child: a client that disconnects without
    // calling silica_quit must not leave a window behind.
}
