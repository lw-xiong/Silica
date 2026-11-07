//! A minimal HTTP/1.1 client: enough to reach a model server on loopback.
//!
//! The endpoint is another process on this machine — no TLS, no redirects, no
//! proxies, no keep-alive — and the alternative is pulling an async runtime
//! and a TLS stack into a workspace that has neither. The same reasoning that
//! hand-rolls base64 in `silica-mcp` applies here: one request shape, written
//! out, beats a dependency tree.
//!
//! Nothing here may panic: both the URL and the response come from outside.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use native_tls::TlsConnector;

// Max concurrent DNS/TCP resolution workers. Each connect attempt is
// bounded by `timeout`, but we still cap the number of live worker
// threads so repeated slow resolutions cannot accumulate unboundedly.
const MAX_CONCURRENT_CONNECTIONS: usize = 64;
static ACTIVE_CONNECTIONS: AtomicUsize = AtomicUsize::new(0);

/// The pieces of a `http://` or `https://` URL.
#[derive(Debug)]
pub struct Url {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    /// The path prefix, with no trailing slash; empty for the root.
    pub prefix: String,
}

impl Url {
    /// Parse `http://host[:port][/prefix]` or `https://host[:port][/prefix]`.
    pub fn parse(url: &str) -> Result<Url, String> {
        let url = url.trim();
        // * The scheme comes off before any trailing slash does: trimming first
        // * turns a bare `http://` into `http:`, which then reads as a host.
        let (scheme, rest) = match url.split_once("://") {
            Some(("http", rest)) => ("http".to_string(), rest),
            Some(("https", rest)) => ("https".to_string(), rest),
            Some((scheme, _)) => return Err(format!("{url}: unknown scheme `{scheme}`")),
            None => ("http".to_string(), url),
        };
        let rest = rest.trim_end_matches('/');
        let (authority, prefix) = match rest.split_once('/') {
            Some((authority, prefix)) => (authority, format!("/{prefix}")),
            None => (rest, String::new()),
        };
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (
                host,
                port.parse::<u16>()
                    .map_err(|_| format!("{url}: `{port}` is not a port"))?,
            ),
            None => (authority, if scheme == "https" { 443 } else { 80 }),
        };
        if host.is_empty() {
            return Err(format!("{url}: no host"));
        }
        Ok(Url {
            scheme,
            host: host.to_string(),
            port,
            prefix: prefix.trim_end_matches('/').to_string(),
        })
    }

    /// `self.prefix` followed by `tail`, which must start with `/`.
    pub fn route(&self, tail: &str) -> String {
        format!("{}{tail}", self.prefix)
    }

    pub fn origin(&self) -> String {
        format!(
            "{}://{}:{}{}",
            self.scheme, self.host, self.port, self.prefix
        )
    }
}

/// A model answer can be arbitrarily large; this is the hard ceiling on the
/// bytes this client will hold for one response, chunked or not.
pub const MAX_RESPONSE_BYTES: usize = 16 << 20; // 16 MiB

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub body: String,
}

impl Response {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// One request, one connection. `body` is sent as JSON when present, which
/// makes this a POST; without it the request is a GET.
/// Build the HTTP/1.1 request head for one call. Extracted so tests can
/// verify header construction: `Authorization` is always terminated with
/// CRLF before `Content-Type` on authenticated POST requests, so the header
/// cannot be malformed into a single invalid line, and the bearer token is
/// actually sent instead of being redacted.
fn build_head(url: &Url, path: &str, body: Option<&str>, bearer: Option<&str>) -> String {
    let method = if body.is_some() { "POST" } else { "GET" };
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {}\r\nAccept: application/json\r\n\
         User-Agent: silica-llm-mcp/{}\r\nConnection: close\r\n",
        url.host,
        env!("CARGO_PKG_VERSION"),
    );
    if let Some(bearer) = bearer {
        head.push_str(&format!("Authorization: Bearer {bearer}\r\n"));
    }
    if let Some(body) = body {
        head.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        ));
    }
    head.push_str("\r\n");
    head
}

pub fn request(
    url: &Url,
    path: &str,
    body: Option<&str>,
    bearer: Option<&str>,
    timeout: Duration,
) -> Result<Response, String> {
    let address = format!("{}:{}", url.host, url.port);
    // ! Resolve and connect with one overall deadline: DNS resolution and the
    // ! TCP handshake run inside a thread joined with the configured timeout,
    // ! so a slow or unreachable endpoint cannot hang past it. On timeout the
    // ! worker thread is detached and exits on its own; the caller never blocks
    // ! on it. Read/write timeouts are set on the stream immediately after the
    // ! connect, so TLS negotiation and request I/O are bounded too.
    let address_for_err = address.clone();
    let host = url.host.clone();
    let port = url.port;
    // * Bound concurrent resolution workers so repeated slow resolutions
    // * cannot accumulate detached threads without limit. Each worker exits
    // * within `timeout` on its own, but the cap makes the worst case fixed.
    // * compare-and-exchange reserves a slot atomically so concurrent requests
    // * cannot all observe a count below the limit and increment past it.
    let mut n = ACTIVE_CONNECTIONS.load(Ordering::Acquire);
    loop {
        if n >= MAX_CONCURRENT_CONNECTIONS {
            return Err(format!(
                "{address}: too many concurrent connections ({MAX_CONCURRENT_CONNECTIONS})"
            ));
        }
        match ACTIVE_CONNECTIONS.compare_exchange_weak(
            n,
            n + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => break,
            Err(m) => n = m,
        }
    }
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = (|| {
            let addr = (host.as_str(), port)
                .to_socket_addrs()
                .map_err(|e| format!("cannot resolve {address_for_err}: {e}"))?
                .next()
                .ok_or_else(|| format!("no address for {address_for_err}"))?;
            TcpStream::connect_timeout(&addr, timeout).map_err(|e| {
                format!(
                    "cannot reach {address_for_err}: {e}. Is the model server running? From another machine, SILICA_LLM_URL should be that machines address (http://127.0.0.1:11434/v1 on this one)."
                )
            })
        })();
        let _ = tx.send(result);
        ACTIVE_CONNECTIONS.fetch_sub(1, Ordering::Release);
    });
    let mut stream = match rx.recv_timeout(timeout) {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => return Err(format!("{address}: {e}")),
        Err(_) => {
            return Err(format!(
                "{address}: connection timed out during DNS resolution or TCP handshake"
            ))
        }
    };
    stream
        .set_read_timeout(Some(timeout))
        .and_then(|()| stream.set_write_timeout(Some(timeout)))
        .map_err(|e| format!("{address}: cannot set a timeout: {e}"))?;

    let head = build_head(url, path, body, bearer);

    if url.scheme == "https" {
        return https_request(stream, url, &head, body, &address);
    }

    stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(body.unwrap_or_default().as_bytes()))
        .and_then(|()| stream.flush())
        .map_err(|e| format!("{address}: writing the request failed: {e}"))?;
    read_response(&mut stream, &address)
}

/// Send the request over a TLS connection: `native-tls` uses the operating
/// system's native trust store, so a remote model server is reached over an
/// encrypted, authenticated connection rather than plaintext HTTP.
fn https_request(
    stream: TcpStream,
    url: &Url,
    head: &str,
    body: Option<&str>,
    address: &str,
) -> Result<Response, String> {
    let connector = TlsConnector::new()
        .map_err(|e| format!("{address}: could not build a TLS connector: {e}"))?;
    let mut tls = connector
        .connect(url.host.as_str(), stream)
        .map_err(|e| format!("{address}: TLS handshake failed: {e}"))?;
    tls.write_all(head.as_bytes())
        .and_then(|()| tls.write_all(body.unwrap_or_default().as_bytes()))
        .and_then(|()| tls.flush())
        .map_err(|e| format!("{address}: writing the TLS request failed: {e}"))?;
    read_response(&mut tls, address)
}

/// Read until the message is complete rather than until EOF: a server that
/// ignores `Connection: close` would otherwise hold the socket open until the
/// read timeout, turning every call into a stall.
fn read_response<S: Read>(stream: &mut S, address: &str) -> Result<Response, String> {
    let mut raw: Vec<u8> = Vec::with_capacity(8 << 10);
    let mut chunk = [0u8; 8 << 10];
    let mut head_end = None;
    let mut status = 0u16;
    let mut length: Option<usize> = None;
    let mut chunked = false;
    loop {
        if head_end.is_none() {
            if let Some(at) = find(&raw, b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&raw[..at]).to_string();
                let mut lines = head.split("\r\n");
                let first = lines.next().unwrap_or_default();
                status = first
                    .split_whitespace()
                    .nth(1)
                    .and_then(|code| code.parse().ok())
                    .ok_or_else(|| format!("{address}: no status in {first:?}"))?;
                for line in lines {
                    let Some((name, value)) = line.split_once(':') else {
                        continue;
                    };
                    let value = value.trim();
                    match name.trim().to_ascii_lowercase().as_str() {
                        "content-length" => {
                            let len = value.parse().ok();
                            if let Some(len) = len.filter(|n| *n > MAX_RESPONSE_BYTES) {
                                return Err(format!(
                                    "{address}: Content-Length {len} exceeds the {MAX_RESPONSE_BYTES} byte response cap"
                                ));
                            }
                            length = len;
                        }
                        "transfer-encoding" => {
                            chunked |= value.to_ascii_lowercase().contains("chunked")
                        }
                        _ => {}
                    }
                }
                head_end = Some(at + 4);
            }
        }
        if let Some(start) = head_end {
            let body = &raw[start..];
            let done = if chunked {
                find(body, b"\r\n0\r\n").is_some() || body.starts_with(b"0\r\n")
            } else if let Some(length) = length {
                body.len() >= length
            } else {
                false
            };
            if done {
                return Ok(Response {
                    status,
                    body: decode(body, chunked, length),
                });
            }
        }
        let read = stream
            .read(&mut chunk)
            .map_err(|e| format!("{address}: reading the response failed: {e}"))?;
        if read == 0 {
            let start = head_end
                .ok_or_else(|| format!("{address}: closed before sending a complete header"))?;
            // * The server closed the socket. A body that promised more than
            // * arrived — by Content-Length, or by a chunked stream with no
            // * final chunk — is a truncated answer, and handing it back
            // * looks exactly like a whole one. A reply with no declared
            // * length has nothing to be short of and reads whole.
            let have = raw.len() - start;
            if let Some(length) = length.filter(|l| have < *l) {
                return Err(format!(
                    "{address}: the server closed after {have} of {length} bytes"
                ));
            }
            if chunked {
                return Err(format!(
                    "{address}: the server closed before the final chunk"
                ));
            }
            return Ok(Response {
                status,
                body: decode(&raw[start..], chunked, length),
            });
        }
        raw.extend_from_slice(&chunk[..read]);
        if raw.len() > MAX_RESPONSE_BYTES {
            return Err(format!(
                "{address}: response exceeds the {MAX_RESPONSE_BYTES} byte cap"
            ));
        }
    }
}

fn decode(body: &[u8], chunked: bool, length: Option<usize>) -> String {
    if !chunked {
        let end = length.unwrap_or(body.len()).min(body.len());
        return String::from_utf8_lossy(&body[..end]).into_owned();
    }
    let mut out: Vec<u8> = Vec::with_capacity(body.len());
    let mut rest = body;
    while let Some(at) = find(rest, b"\r\n") {
        // * A chunk header may carry extensions after a `;`.
        let header = String::from_utf8_lossy(&rest[..at]);
        let size = header.split(';').next().unwrap_or("").trim();
        let Ok(size) = usize::from_str_radix(size, 16) else {
            break;
        };
        if size == 0 {
            break;
        }
        let from = at + 2;
        // * `size` is the server's word, so `from + size` can wrap: a header of
        // * `ffffffffffffffff` would otherwise land `end` below `from` and panic
        // * on the slice. A header that overflows is not a size at all and the
        // * loop stops; one that merely runs past the body is clamped, so a
        // * response cut off mid-chunk still yields what arrived.
        let Some(end) = from.checked_add(size) else {
            break;
        };
        let end = end.min(rest.len());
        out.extend_from_slice(&rest[from..end]);
        if end + 2 > rest.len() {
            break;
        }
        rest = &rest[end + 2..];
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_url_forms() {
        let u = Url::parse("http://127.0.0.1:11434/v1").unwrap();
        assert_eq!(
            (u.host.as_str(), u.port, u.prefix.as_str()),
            ("127.0.0.1", 11434, "/v1")
        );
        assert_eq!(u.route("/chat/completions"), "/v1/chat/completions");

        let u = Url::parse("http://localhost:8080/").unwrap();
        assert_eq!((u.port, u.prefix.as_str()), (8080, ""));
        assert_eq!(u.route("/models"), "/models");

        // * No scheme and no port: the defaults.
        let u = Url::parse("example.test").unwrap();
        assert_eq!((u.host.as_str(), u.port), ("example.test", 80));
    }

    #[test]
    fn accepts_https_urls() {
        let u = Url::parse("https://api.example.test/v1").unwrap();
        assert_eq!(u.scheme.as_str(), "https");
        assert_eq!((u.host.as_str(), u.port), ("api.example.test", 443));
    }

    #[test]
    fn refuses_what_it_cannot_do() {
        assert!(Url::parse("ftp://x")
            .unwrap_err()
            .contains("unknown scheme"));
        assert!(Url::parse("http://host:not-a-port")
            .unwrap_err()
            .contains("not a port"));
        assert!(Url::parse("http://").unwrap_err().contains("no host"));
    }

    /// A `Content-Length: 0` is a whole answer, not a body still to come:
    /// the reply is complete the moment the header ends, and the body is
    /// empty. A first line with no status code is an error naming it,
    /// because a response without one is not a message to parse.
    #[test]
    fn a_zero_length_body_and_a_response_with_no_status_are_both_settled() {
        // * Empty answer, complete at once.
        let wire = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".to_vec();
        let got = read_response(&mut std::io::Cursor::new(wire), "test").unwrap();
        assert_eq!(got.status, 200);
        assert_eq!(got.body, "");

        // * The same with a body that follows anyway: the length rules.
        let wire = b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\nnothing".to_vec();
        let got = read_response(&mut std::io::Cursor::new(wire), "test").unwrap();
        assert_eq!(got.body, "");

        // * A first line with no status code never becomes a message.
        let wire = b"HELLO THERE\r\nContent-Length: 2\r\n\r\nhi".to_vec();
        let err = read_response(&mut std::io::Cursor::new(wire), "test")
            .expect_err("a response with no status must not answer Ok");
        assert!(err.contains("no status"), "{err}");
    }

    #[test]
    fn decodes_a_plain_body() {
        assert_eq!(decode(b"hello", false, Some(5)), "hello");
        // * Trailing bytes past Content-Length are not part of the message.
        assert_eq!(decode(b"hello there", false, Some(5)), "hello");
        assert_eq!(decode(b"hello", false, None), "hello");
    }

    #[test]
    fn decodes_a_chunked_body() {
        assert_eq!(decode(b"5\r\nhello\r\n0\r\n\r\n", true, None), "hello");
        assert_eq!(
            decode(b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n", true, None),
            "hello world"
        );
        // * A chunk-extension on the header, and a truncated final chunk.
        assert_eq!(decode(b"5;x=1\r\nhello\r\n", true, None), "hello");
        assert_eq!(decode(b"ff\r\nshort", true, None), "short");
    }

    #[test]
    fn authenticated_post_head_ends_authorization_with_crlf() {
        let url = Url::parse("http://127.0.0.1:11434/v1").unwrap();
        let head = build_head(&url, "/chat/completions", Some("{}"), Some("secret-token"));
        assert!(
            head.contains("Authorization: Bearer secret-token\r\nContent-Type:"),
            "head was:\n{head}"
        );
        assert!(head.contains("POST /chat/completions HTTP/1.1"));
    }

    #[test]
    fn authenticated_get_head_ends_authorization_with_crlf() {
        let url = Url::parse("http://127.0.0.1:11434/v1").unwrap();
        let head = build_head(&url, "/models", None, Some("secret-token"));
        assert!(
            head.contains("Authorization: Bearer secret-token\r\n"),
            "head was:\n{head}"
        );
        assert!(!head.contains("Content-Type"));
        assert!(head.contains("GET /models HTTP/1.1"));
    }

    #[test]
    fn connection_counter_returns_to_zero_after_failure() {
        // * A connect to an unreachable address must not leave the
        // * worker counter elevated; every worker decrements on exit even
        // * when the connection fails.
        let url = Url::parse("http://10.255.255.1:1/v1").unwrap();
        let before = ACTIVE_CONNECTIONS.load(Ordering::Acquire);
        let _ = request(&url, "/v1", None, None, Duration::from_millis(100));
        // * The worker decrements after the request returns, so the counter can
        // * still be elevated for a moment on a loaded CI machine. Poll until it
        // * settles rather than assuming a fixed sleep is long enough.
        let settled = (0..100).any(|_| {
            if ACTIVE_CONNECTIONS.load(Ordering::Acquire) == before {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
            false
        });
        assert!(
            settled,
            "the connection counter stayed at {} (expected {before})",
            ACTIVE_CONNECTIONS.load(Ordering::Acquire)
        );
    }

    /// A chunk header is the server's word, so its size can be anything a
    /// `usize` holds. `from + size` then wrapped and the slice panicked; the
    /// body must come back ignored instead of taking the process down.
    #[test]
    fn an_oversized_chunk_header_does_not_panic() {
        // usize::MAX as a chunk size: no arithmetic can satisfy it, so the
        // loop stops with nothing decoded rather than wrapping `end` below
        // `from` and panicking on the slice.
        assert_eq!(
            decode(b"ffffffffffffffff\r\nhello\r\n0\r\n\r\n", true, None),
            ""
        );
        // A size that fits but runs past the body is clamped to what arrived,
        // so a response cut off mid-chunk still yields its bytes.
        assert_eq!(decode(b"ff\r\nshort", true, None), "short");
        // And a header that is not hex stops the loop rather than guessing.
        assert_eq!(decode(b"zz\r\nhello\r\n0\r\n\r\n", true, None), "");
    }

    /// A server that dies mid-body must not hand back a truncated answer
    /// that looks complete. The header promised more bytes than arrived, so
    /// the shortfall is named; a completely bodied reply still reads whole,
    /// and so does the old shape with no declared length at all.
    #[test]
    fn a_body_the_server_did_not_finish_is_an_error_not_a_short_answer() {
        // * Content-Length promised 100 bytes; the socket closed after 20.
        let cut = b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nhalf of the body a".to_vec();
        let err = read_response(&mut std::io::Cursor::new(cut), "test")
            .expect_err("a body short of its Content-Length must not answer Ok");
        assert!(
            err.contains("closed") || err.contains("byte"),
            "the shortfall must be named: {err}"
        );

        // * The same for a chunked reply with no final chunk.
        let cut = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello".to_vec();
        let err = read_response(&mut std::io::Cursor::new(cut), "test")
            .expect_err("a chunked body with no final chunk must not answer Ok");
        assert!(
            err.contains("closed") || err.contains("chunk"),
            "the cut must be named: {err}"
        );

        // * A whole body still answers.
        let whole = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello".to_vec();
        let got = read_response(&mut std::io::Cursor::new(whole), "test").unwrap();
        assert_eq!(got.body, "hello");

        // * And so does the shape that never declares a length.
        let whole = b"HTTP/1.1 200 OK\r\n\r\nhello".to_vec();
        let got = read_response(&mut std::io::Cursor::new(whole), "test").unwrap();
        assert_eq!(got.body, "hello");
    }

    /// A header that promises a length cannot parse — a negative number, a
    /// float, a hex digit — means no length at all, so the reply is read to
    /// EOF rather than answered short; and a socket closed before the header
    /// ends is an error, never an empty answer.
    #[test]
    fn a_content_length_that_cannot_parse_leaves_the_body_read_to_eof() {
        for header in [
            "Content-Length: -5",
            "Content-Length: 1.5",
            "Content-Length: 0x10",
            "Content-Length: 99999999999999999999",
        ] {
            let wire = format!("HTTP/1.1 200 OK\r\n{header}\r\n\r\nhello").into_bytes();
            let got = read_response(&mut std::io::Cursor::new(wire), "test")
                .unwrap_or_else(|e| panic!("{header}: {e}"));
            assert_eq!(got.body, "hello", "{header} should read to EOF");
        }

        // * Closed before the header ends: an error, not an empty 200.
        let cut = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n".to_vec();
        let err = read_response(&mut std::io::Cursor::new(cut), "test")
            .expect_err("a header cut off mid-way must not answer Ok");
        assert!(err.contains("closed"), "{err}");
    }

    /// A chunk whose declared size covers the body but whose trailing CRLF is
    /// missing: the last chunk is taken, the loop stops cleanly.
    #[test]
    fn a_truncated_chunk_keeps_what_it_has() {
        assert_eq!(decode(b"5\r\nhello", true, None), "hello");
    }
}
