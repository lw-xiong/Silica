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
use std::net::TcpStream;
use std::time::Duration;

/// The pieces of an `http://host:port/prefix` URL.
#[derive(Debug)]
pub struct Url {
    pub host: String,
    pub port: u16,
    /// The path prefix, with no trailing slash; empty for the root.
    pub prefix: String,
}

impl Url {
    /// Parse `http://host[:port][/prefix]`. `https` is refused rather than
    /// silently downgraded: this client has no TLS, and a local model server
    /// does not need it.
    pub fn parse(url: &str) -> Result<Url, String> {
        let url = url.trim();
        // The scheme comes off before any trailing slash does: trimming first
        // turns a bare `http://` into `http:`, which then reads as a host.
        let rest = match url.split_once("://") {
            Some(("http", rest)) => rest,
            Some(("https", _)) => {
                return Err(format!(
                    "{url}: this client speaks plain HTTP only, so point it at \
                     a model server on this machine"
                ))
            }
            Some((scheme, _)) => return Err(format!("{url}: unknown scheme `{scheme}`")),
            None => url,
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
            None => (authority, 80),
        };
        if host.is_empty() {
            return Err(format!("{url}: no host"));
        }
        Ok(Url {
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
        format!("http://{}:{}{}", self.host, self.port, self.prefix)
    }
}

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
pub fn request(
    url: &Url,
    path: &str,
    body: Option<&str>,
    bearer: Option<&str>,
    timeout: Duration,
) -> Result<Response, String> {
    let address = format!("{}:{}", url.host, url.port);
    // Resolve and connect with the same budget as the read: a wrong port on
    // loopback refuses at once, but a wrong host must not hang forever.
    let mut stream = TcpStream::connect(&address).map_err(|e| {
        format!(
            "cannot reach {address}: {e}. Is the model server running? \
             On misco: `systemctl status llama-cpp`. From another machine, \
             SILICA_LLM_URL should be the tailnet name \
             (http://misco.tail.ragtux.com:8080/v1)."
        )
    })?;
    stream
        .set_read_timeout(Some(timeout))
        .and_then(|()| stream.set_write_timeout(Some(timeout)))
        .map_err(|e| format!("{address}: cannot set a timeout: {e}"))?;

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
    stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(body.unwrap_or_default().as_bytes()))
        .and_then(|()| stream.flush())
        .map_err(|e| format!("{address}: writing the request failed: {e}"))?;

    read_response(&mut stream, &address, timeout)
}

/// Read until the message is complete rather than until EOF: a server that
/// ignores `Connection: close` would otherwise hold the socket open until the
/// read timeout, turning every call into a stall.
fn read_response(
    stream: &mut TcpStream,
    address: &str,
    timeout: Duration,
) -> Result<Response, String> {
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
                        "content-length" => length = value.parse().ok(),
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
                // The terminal zero-length chunk; good enough without parsing
                // every chunk header, since the de-chunker below does that.
                find(body, b"\r\n0\r\n").is_some() || body.starts_with(b"0\r\n")
            } else if let Some(length) = length {
                body.len() >= length
            } else {
                false // No framing: only EOF ends it.
            };
            if done {
                return Ok(Response {
                    status,
                    body: decode(body, chunked, length),
                });
            }
        }
        let read = stream.read(&mut chunk).map_err(|e| {
            format!("{address}: reading the response failed: {e} (after {timeout:?})")
        })?;
        if read == 0 {
            // EOF. Everything that arrived is the message.
            let start = head_end
                .ok_or_else(|| format!("{address}: closed before sending a complete header"))?;
            return Ok(Response {
                status,
                body: decode(&raw[start..], chunked, length),
            });
        }
        raw.extend_from_slice(&chunk[..read]);
    }
}

/// The body as text: de-chunked when chunked, truncated to `Content-Length`
/// when one was given, and lossy because an error page may be any encoding.
fn decode(body: &[u8], chunked: bool, length: Option<usize>) -> String {
    if !chunked {
        let end = length.unwrap_or(body.len()).min(body.len());
        return String::from_utf8_lossy(&body[..end]).into_owned();
    }
    let mut out: Vec<u8> = Vec::with_capacity(body.len());
    let mut rest = body;
    while let Some(at) = find(rest, b"\r\n") {
        // A chunk header may carry extensions after a `;`.
        let header = String::from_utf8_lossy(&rest[..at]);
        let size = header.split(';').next().unwrap_or("").trim();
        let Ok(size) = usize::from_str_radix(size, 16) else {
            break;
        };
        if size == 0 {
            break;
        }
        let from = at + 2;
        let to = (from + size).min(rest.len());
        out.extend_from_slice(&rest[from..to]);
        if to + 2 > rest.len() {
            break;
        }
        rest = &rest[to + 2..];
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

        // No scheme and no port: the defaults.
        let u = Url::parse("example.test").unwrap();
        assert_eq!((u.host.as_str(), u.port), ("example.test", 80));
    }

    #[test]
    fn refuses_what_it_cannot_do() {
        assert!(Url::parse("https://api.example.test")
            .unwrap_err()
            .contains("plain HTTP"));
        assert!(Url::parse("ftp://x")
            .unwrap_err()
            .contains("unknown scheme"));
        assert!(Url::parse("http://host:not-a-port")
            .unwrap_err()
            .contains("not a port"));
        assert!(Url::parse("http://").unwrap_err().contains("no host"));
    }

    #[test]
    fn decodes_a_plain_body() {
        assert_eq!(decode(b"hello", false, Some(5)), "hello");
        // Trailing bytes past Content-Length are not part of the message.
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
        // A chunk-extension on the header, and a truncated final chunk.
        assert_eq!(decode(b"5;x=1\r\nhello\r\n", true, None), "hello");
        assert_eq!(decode(b"ff\r\nshort", true, None), "short");
    }
}
