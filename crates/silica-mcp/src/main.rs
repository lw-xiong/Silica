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

/// The largest request line read from stdin. The control socket refuses one
/// bigger than this, and a line of stdin is the same unbounded allocation
/// before it is ever parsed, so the cap belongs here too.
const MAX_LINE_BYTES: usize = 64 * 1024;

/// One line from `reader`, or `None` at the end of the input. `Err` is a
/// line the protocol cannot carry: one past [`MAX_LINE_BYTES`], or one that
/// is not UTF-8. `read_until` consumes the whole line either way, so the
/// remainder of an over-long one is never read back as a second message.
fn next_line(reader: &mut impl BufRead) -> Option<std::io::Result<String>> {
    let mut bytes = Vec::new();
    if reader.read_until(b'\n', &mut bytes).ok()? == 0 {
        return None;
    }
    if bytes.len() > MAX_LINE_BYTES {
        return Some(Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "request line is too large",
        )));
    }
    match String::from_utf8(bytes) {
        Ok(line) => Some(Ok(line)),
        Err(_) => Some(Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "request line is not UTF-8",
        ))),
    }
}

fn main() {
    let config = app::Config::from_env();
    // * The handshake has no room for this, and a wrong binary or socket is the
    // * first thing to go wrong, so say both before answering anything.
    eprintln!(
        "silica-mcp {} on {} (app: {}, timeout {} ms)",
        env!("CARGO_PKG_VERSION"),
        config.socket.display(),
        config.bin,
        config.timeout.as_millis()
    );
    let mut app = app::App::new(config);
    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();
    let mut stdout = std::io::stdout();
    while let Some(line) = next_line(&mut stdin) {
        let line = match line {
            Ok(line) => line,
            // * A line the protocol cannot carry is logged and skipped, the
            //   way the control socket answers and stays open.
            Err(e) => {
                eprintln!("silica-mcp: stdin: {e}");
                continue;
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
    // ! Dropping the app quits the child: a client that disconnects without
    // ! calling silica_quit must not leave a window behind.
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_of(input: &str) -> Vec<std::io::Result<String>> {
        lines_of_bytes(input.as_bytes())
    }

    fn lines_of_bytes(input: &[u8]) -> Vec<std::io::Result<String>> {
        let mut reader = std::io::BufReader::new(input);
        std::iter::from_fn(|| next_line(&mut reader)).collect()
    }

    #[test]
    fn lines_come_back_in_order_with_their_newlines() {
        let got = lines_of("one\ntwo\n");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].as_ref().unwrap(), "one\n");
        assert_eq!(got[1].as_ref().unwrap(), "two\n");
    }

    #[test]
    fn the_last_line_may_arrive_without_its_newline() {
        let got = lines_of("tail");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].as_ref().unwrap(), "tail");
        assert!(
            lines_of("").is_empty(),
            "the end of the input ends the stream"
        );
    }

    /// A line past the cap is refused, and `read_until` has already consumed
    /// all of it, so the line after it is still a line of its own.
    #[test]
    fn an_over_long_line_is_refused_without_eating_the_next_one() {
        let big = "x".repeat(MAX_LINE_BYTES + 1);
        let got = lines_of(&format!("{big}\nafter\n"));
        assert_eq!(got.len(), 2);
        let err = got[0].as_ref().unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("too large"), "{err}");
        assert_eq!(got[1].as_ref().unwrap(), "after\n");
    }

    #[test]
    fn a_line_that_is_not_utf8_is_refused() {
        let got = lines_of_bytes(&[0xff, 0xfe, b'\n']);
        assert_eq!(got.len(), 1);
        let err = got[0].as_ref().unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("UTF-8"), "{err}");
    }
}
