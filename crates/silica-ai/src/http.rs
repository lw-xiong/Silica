//! A minimal blocking HTTP/1.1 client: enough to reach a model server on
//! this machine. One request, one connection, `Connection: close` — no
//! keep-alive, no proxies, no redirect following (a chat endpoint does not
//! redirect). Mirrors `silica-llm-mcp/src/http.rs`, trimmed to what the AI
//! pane needs. Nothing here may panic: the URL and the response both come
//! from outside.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Header bytes are capped so a broken server cannot make the read loop
/// grow without bound before the status line is even complete.
const MAX_HEADER_BYTES: usize = 64 * 1024;

/// The pieces of an `http://` or `https://` URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    /// The path prefix, without a trailing slash; empty for the root.
    pub prefix: String,
}

impl Url {
    pub fn parse(url: &str) -> Result<Url, String> {
        let url = url.trim();
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
            Some((host, port)) if !host.contains(']') => (
                host,
                port.parse::<u16>()
                    .map_err(|_| format!("{url}: `{port}` is not a port"))?,
            ),
            _ => (
                authority.trim_matches(['[', ']']),
                if scheme == "https" { 443 } else { 80 },
            ),
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
}

pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

trait Stream: Read + Write {}
impl Stream for TcpStream {}
impl<T: Read + Write> Stream for native_tls::TlsStream<T> {}

/// One request. `body` is sent as JSON, which makes this a POST; without it
/// the request is a GET. DNS + connect + all I/O are bounded by `timeout`:
/// the connect runs in a worker thread joined with that deadline, and the
/// stream gets read/write timeouts too, so a hung server cannot block the
/// caller past it (the detached worker exits on its own).
pub fn request(
    url: &Url,
    path: &str,
    body: Option<&str>,
    bearer: Option<&str>,
    timeout: Duration,
    max_bytes: usize,
) -> Result<Response, String> {
    let address = format!("{}:{}", url.host, url.port);
    let (tx, rx) = mpsc::channel();
    let worker = Worker {
        scheme: url.scheme.clone(),
        host: url.host.clone(),
        port: url.port,
        path: path.to_string(),
        body: body.map(str::to_string),
        bearer: bearer.map(str::to_string),
        timeout,
        max_bytes,
    };
    thread::spawn(move || {
        let _ = tx.send(worker.run());
    });
    rx.recv_timeout(timeout)
        .unwrap_or_else(|_| Err(format!("{address}: timed out after {}s", timeout.as_secs())))
}

struct Worker {
    scheme: String,
    host: String,
    port: u16,
    path: String,
    body: Option<String>,
    bearer: Option<String>,
    timeout: Duration,
    max_bytes: usize,
}

impl Worker {
    fn run(self) -> Result<Response, String> {
        let tcp = self.connect()?;
        let mut stream: Box<dyn Stream> = if self.scheme == "https" {
            let connector = native_tls::TlsConnector::new().map_err(|e| e.to_string())?;
            Box::new(
                connector
                    .connect(&self.host, tcp)
                    .map_err(|e| format!("TLS to {}: {e}", self.host))?,
            )
        } else {
            Box::new(tcp)
        };
        let head = build_head(&self.host, &self.path, self.body.as_deref(), self.bearer.as_deref());
        stream
            .write_all(head.as_bytes())
            .and_then(|_| stream.flush())
            .map_err(|e| format!("write: {e}"))?;
        read_response(stream.as_mut(), self.max_bytes)
    }

    fn connect(&self) -> Result<TcpStream, String> {
        let address = format!("{}:{}", self.host, self.port);
        let addrs: Vec<_> = address
            .to_socket_addrs()
            .map_err(|e| format!("{address}: {e}"))?
            .collect();
        let mut last = String::new();
        for addr in addrs {
            match TcpStream::connect_timeout(&addr, self.timeout) {
                Ok(stream) => {
                    stream.set_read_timeout(Some(self.timeout)).ok();
                    stream.set_write_timeout(Some(self.timeout)).ok();
                    return Ok(stream);
                }
                Err(e) => last = format!("{addr}: {e}"),
            }
        }
        Err(if last.is_empty() {
            format!("{address}: no address resolved")
        } else {
            last
        })
    }
}

/// The request head. The bearer token is a header value on its own line, so
/// it cannot be malformed into the status line.
fn build_head(host: &str, path: &str, body: Option<&str>, bearer: Option<&str>) -> String {
    let method = if body.is_some() { "POST" } else { "GET" };
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nAccept: application/json\r\n\
         Connection: close\r\n"
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
    if let Some(body) = body {
        head.push_str(body);
    }
    head
}

fn read_response(stream: &mut dyn Stream, max_bytes: usize) -> Result<Response, String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        if let Some(pos) = find(&buf, b"\r\n\r\n") {
            break pos;
        }
        if buf.len() > MAX_HEADER_BYTES {
            return Err("response headers over 64 KiB".to_string());
        }
        let n = stream.read(&mut chunk).map_err(|e| format!("read: {e}"))?;
        if n == 0 {
            return Err("connection closed before the headers ended".to_string());
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| "no status line in the response".to_string())?;
    let mut content_length = None;
    let mut chunked = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => content_length = value.trim().parse::<usize>().ok(),
            "transfer-encoding" => chunked = value.to_ascii_lowercase().contains("chunked"),
            _ => {}
        }
    }
    // * `Connection: close` was sent, so the body runs to EOF; read it all
    //   (capped), then apply Content-Length or decode chunked.
    let mut body = buf[head_end + 4..].to_vec();
    loop {
        if body.len() > max_bytes {
            return Err(format!("response body over {max_bytes} bytes"));
        }
        let n = stream.read(&mut chunk).map_err(|e| format!("read: {e}"))?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    if chunked {
        body = decode_chunked(&body)?;
    } else if let Some(len) = content_length {
        body.truncate(len.min(body.len()));
    }
    Ok(Response { status, body })
}

fn decode_chunked(mut data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let line_end = find(data, b"\r\n").ok_or("chunked: no size line")?;
        let size_text = std::str::from_utf8(&data[..line_end]).map_err(|_| "chunked: bad size")?;
        let size = usize::from_str_radix(
            size_text.trim().split(';').next().unwrap_or("").trim(),
            16,
        )
        .map_err(|_| format!("chunked: bad size `{size_text}`"))?;
        data = &data[line_end + 2..];
        if size == 0 {
            break;
        }
        if data.len() < size + 2 {
            return Err("chunked: truncated chunk".to_string());
        }
        out.extend_from_slice(&data[..size]);
        data = &data[size + 2..];
    }
    Ok(out)
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
        let url = Url::parse("http://127.0.0.1:8080/v1").expect("parses");
        assert_eq!(url.scheme, "http");
        assert_eq!(url.host, "127.0.0.1");
        assert_eq!(url.port, 8080);
        assert_eq!(url.prefix, "/v1");
        assert_eq!(url.route("/chat/completions"), "/v1/chat/completions");
        assert_eq!(Url::parse("https://example.com").expect("https").port, 443);
        assert!(Url::parse("ftp://x").is_err());
        assert!(Url::parse("http://").is_err());
    }

    #[test]
    fn decodes_plain_and_chunked_bodies() {
        assert_eq!(decode_chunked(b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n").unwrap(), b"Wikipedia");
        assert!(decode_chunked(b"zz\r\n").is_err());
    }
}
