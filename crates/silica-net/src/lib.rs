//! What Silica may fetch, and how.
//!
//! One home for the network boundary: the bin's remote-image client and
//! the plugin shim's `requestUrl` share these rules, so a hole in one is
//! not a hole only the other has.

/// The largest response either caller will keep.
pub const MAX_BYTES: u64 = 32 * 1024 * 1024;

/// The largest response header block either caller will keep. A malicious
/// server can send arbitrarily large headers; cap them before they fill disk.
pub const MAX_HEADERS_BYTES: u64 = 64 * 1024;

/// How many redirects a fetch may follow. Each hop is revalidated against
/// the private-network rules before the connection is made, so a hostile
/// redirect chain cannot reach an address the original URL was refused.
pub const MAX_REDIRECTS: u32 = 5;

/// The seconds a single attempt may take.
pub const TIMEOUT_SECS: &str = "30";

/// Strip `user:pass@` from a URL's authority so credentials embedded in a
/// URL are never sent to the remote host.
fn strip_userinfo(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let rest_after_authority = &rest[authority.len()..];
    format!("{}://{}{}", scheme, host, rest_after_authority)
}

/// Ports a fetch may use. Well-known ports below 1024 (except HTTP/HTTPS)
/// are refused: they are typically router admin pages, mail servers, and
/// other local services a note should not be able to probe.
fn allowed_port(port: u16) -> bool {
    matches!(port, 80 | 443) || port >= 1024
}

pub fn allows_destination(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return false;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    // * Drop any userinfo, then any port.
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    let port = host_port
        .rsplit_once(':')
        .and_then(|(_, p)| p.parse::<u16>().ok());
    if let Some(p) = port {
        if !allowed_port(p) {
            return false;
        }
    }
    let host = if host_port.starts_with('[') {
        // * A bracketed IPv6 literal.
        host_port
            .split(']')
            .next()
            .unwrap_or(host_port)
            .trim_start_matches('[')
    } else {
        host_port.split(':').next().unwrap_or(host_port)
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || host == "localhost" || host.ends_with(".localhost") {
        return false;
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return !is_private(ip);
    }
    // ! A DNS name: resolve it and refuse if every address it could reach is
    //   private. curl resolves again at connect time, but this check catches the
    //   common rebinding case and the `--resolve` argument pins the one we
    //   validated.
    match std::net::ToSocketAddrs::to_socket_addrs(&format!("{host}:80")) {
        Ok(mut addrs) => addrs.any(|a| !is_private(a.ip())),
        Err(_) => false,
    }
}

/// The `--resolve host:port:ip` argument for curl, pinning the connection to
/// the first public address the name resolves to. Returns `None` when the
/// name cannot be resolved or every address is private, so the caller can
/// refuse the request rather than let curl resolve on its own.
pub fn resolve_target(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    let (host, port) = match host_port.rsplit_once(':') {
        Some((h, p)) if p.parse::<u16>().is_ok() => (h, p.parse::<u16>().ok()?),
        _ => (
            host_port,
            match scheme.to_ascii_lowercase().as_str() {
                "https" => 443,
                _ => 80,
            },
        ),
    };
    let ip = std::net::ToSocketAddrs::to_socket_addrs(&format!("{host}:{port}"))
        .ok()?
        .find(|a| !is_private(a.ip()))?;
    Some(format!("{host}:{port}:{}", ip.ip()))
}

/// Loopback, link-local, private, and the other ranges that mean "this
/// machine or its network" rather than "the internet".
pub fn is_private(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || v4.is_documentation()
                // * 100.64/10, carrier-grade NAT, which is a LAN in practice.
                || (v4.octets()[0] == 100 && (64..128).contains(&v4.octets()[1]))
                // * 169.254 metadata endpoints are link-local and already
                //   covered; 0/8 and 240/4 are not routable either.
                || v4.octets()[0] == 0
                || v4.octets()[0] >= 240
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                // * Unique local (fc00::/7) and link-local (fe80::/10).
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                // * An IPv4-mapped address is private if the IPv4 is.
                || v6.to_ipv4_mapped().is_some_and(|v4| is_private(v4.into()))
        }
    }
}

/// Whether an HTTP status code is a redirect that we follow and revalidate,
/// so a redirect chain cannot bounce to a private address the original URL
/// passed.
pub fn is_redirect_status(code: u16) -> bool {
    matches!(code, 301 | 302 | 303 | 307 | 308)
}

/// Extracts the `Location` header value from the headers dump curl writes
/// with `-D`.
pub fn extract_location(headers: &str) -> Option<String> {
    headers
        .lines()
        .find(|line| line.to_ascii_lowercase().starts_with("location:"))
        .and_then(|line| line.split_once(':'))
        .map(|(_, value)| value.trim().to_string())
}

/// Resolves a `Location` header value against the current URL, so relative
/// redirects such as `/images/file.png` or `//example.com/x` become absolute
/// `http`/`https` URLs before they are validated against the destination and
/// DNS rules. Returns `None` when the location cannot be resolved to an
/// absolute http/https URL.
pub fn resolve_redirect(base: &str, location: &str) -> Option<String> {
    let base_url = url::Url::parse(base).ok()?;
    let resolved = base_url.join(location).ok()?;
    let scheme = resolved.scheme();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    Some(resolved.to_string())
}

/// One response, kept whole: the status, the headers as they arrived, and
/// the body bytes (a plugin's `requestUrl` wants the text and the array
/// buffer both, so neither is derived from the other here).
#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// A fetch's inputs: the method, the URL, the headers, and the body.
#[derive(Debug, Clone, Default)]
pub struct Request<'a> {
    pub method: &'a str,
    pub url: &'a str,
    /// Header lines, `Name: value` each, already folded.
    pub headers: &'a [String],
    pub body: Option<&'a [u8]>,
    /// The seconds one attempt may take. A caller with its own deadline
    /// (a plugin's command budget) passes a smaller one.
    pub timeout_secs: u32,
}

impl<'a> Request<'a> {
    /// A GET with the shared defaults.
    pub fn get(url: &'a str) -> Self {
        Self {
            method: "GET",
            url,
            timeout_secs: TIMEOUT_SECS.parse().unwrap_or(30),
            ..Default::default()
        }
    }
}

/// Fetches `request` through the shared boundary: the destination is
/// validated, the connection is pinned to the address that was validated,
/// and every redirect hop is revalidated the same way. No cache and no
/// proxies — the plugin shim and the image client are the two callers, and
/// neither wants the other's bookkeeping.
///
/// Ponytail: one curl process per call, the same shape the image client
/// uses, so the pinning behaviour is identical in both callers. A pooled
/// async client would cut the spawn overhead but cannot pin a resolved IP
/// per hop, which is what holds the boundary together.
pub fn fetch(request: &Request<'_>) -> Result<Response, String> {
    if !allows_destination(request.url) {
        return Err(format!("the destination is not allowed: {}", request.url));
    }
    let unique = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let dir = std::env::temp_dir().join(format!("silica-net-{unique}"));
    if std::fs::create_dir_all(&dir).is_err() {
        return Err("cannot create a scratch directory".to_string());
    }
    let body_path = dir.join("body");
    let headers_path = dir.join("headers");

    let result = (|| {
        let mut current = request.url.to_string();
        let mut resolve = resolve_target(&current)
            .ok_or_else(|| format!("the destination resolves to no public address: {current}"))?;
        for _hop in 0..MAX_REDIRECTS {
            match attempt(request, &current, &resolve, &body_path, &headers_path)? {
                // * Each hop is revalidated against the destination and DNS
                //   rules before its connection is made, so a chain cannot
                //   bounce to an address the original URL was refused.
                Attempt::Redirect(location) => {
                    let next = resolve_redirect(&current, &location)
                        .ok_or_else(|| "the redirect leaves http/https".to_string())?;
                    if !allows_destination(&next) {
                        return Err(format!("the redirect is not allowed: {next}"));
                    }
                    current = next;
                    resolve = resolve_target(&current).ok_or_else(|| {
                        format!("the redirect resolves to no public address: {current}")
                    })?;
                }
                Attempt::Answered(response) => return Ok(response),
            }
        }
        Err("too many redirects".to_string())
    })();

    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// What one curl attempt answered: a redirect to follow, or the response.
enum Attempt {
    Redirect(String),
    Answered(Response),
}

/// One curl attempt: the headers dump and the body land in the caller's
/// scratch files, and the call blocks until curl exits. A transport
/// failure is a zero status with an empty body, which a caller can tell
/// from an answer because a real answer carries a status line.
fn attempt(
    request: &Request<'_>,
    url: &str,
    resolve: &str,
    body_path: &std::path::Path,
    headers_path: &std::path::Path,
) -> Result<Attempt, String> {
    let mut command = std::process::Command::new("curl");
    command
        .arg("-s")
        .arg("--no-config")
        .arg("--noproxy")
        .arg("*")
        .arg("--proto")
        .arg("=http,https")
        .arg("--proto-redir")
        .arg("=http,https")
        .arg("--resolve")
        .arg(resolve)
        .arg("--max-filesize")
        .arg(MAX_BYTES.to_string())
        .arg("--max-time")
        .arg(request.timeout_secs.to_string())
        .arg("-D")
        .arg(headers_path)
        .arg("-o")
        .arg(body_path)
        .arg("-X")
        .arg(request.method)
        .arg(strip_userinfo(url));
    for header in request.headers {
        command.arg("-H").arg(header);
    }
    if let Some(body) = request.body {
        // * A body goes in over stdin rather than a temp file: it is the
        //   caller's bytes already, and a file would only be another thing
        //   to clean up.
        use std::io::Write;
        let mut child = command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("cannot start curl: {e}"))?;
        if let Some(sink) = child.stdin.as_mut() {
            sink.write_all(body)
                .map_err(|e| format!("cannot write the request body: {e}"))?;
        }
        child.wait().map_err(|e| format!("curl failed: {e}"))?;
    } else {
        command
            .status()
            .map_err(|e| format!("cannot start curl: {e}"))?;
    }

    let headers = std::fs::read_to_string(headers_path).unwrap_or_default();
    if headers.len() as u64 > MAX_HEADERS_BYTES {
        return Err("response headers exceed the size limit".to_string());
    }
    let body = std::fs::read(body_path).unwrap_or_default();
    let code = status_code(&headers);
    if let Some(location) = code
        .filter(|c| is_redirect_status(*c))
        .and_then(|_| extract_location(&headers))
    {
        return Ok(Attempt::Redirect(location));
    }
    // * A 4xx or 5xx still carries a body and headers: it is an answer,
    //   not a transport error, and the caller asked for it.
    Ok(Attempt::Answered(Response {
        status: code.unwrap_or(0),
        headers: parse_headers(&headers),
        body,
    }))
}

/// The status code from curl's `-D` dump: the first line's second word.
fn status_code(headers: &str) -> Option<u16> {
    headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
}

/// The header dump as pairs, skipping the status line and any continuation.
fn parse_headers(headers: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for line in headers.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_string();
        let value = value.trim().to_string();
        if !name.is_empty() {
            pairs.push((name, value));
        }
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Loopback and the private ranges are refused before any request: the
    /// point of the guard is that nothing is sent at all.
    #[test]
    fn the_private_ranges_are_refused() {
        for url in [
            "http://localhost/x",
            "http://foo.localhost/x",
            "http://127.0.0.1/x",
            "http://10.0.0.5/x",
            "http://192.168.1.1/x",
            "http://172.16.0.1/x",
            "http://169.254.169.254/latest/meta-data",
            "http://100.64.0.1/x",
            "http://[::1]/x",
            "http://[fc00::1]/x",
            "http://[fe80::1]/x",
            "ftp://example.com/x",
            "file:///etc/passwd",
            "not a url",
        ] {
            assert!(
                !allows_destination(url),
                "{url} was allowed: the guard exists to stop exactly this"
            );
        }
    }

    /// A public literal is allowed without touching DNS.
    #[test]
    fn a_public_address_is_allowed() {
        assert!(allows_destination("http://93.184.216.34/x"));
        assert!(allows_destination("https://1.1.1.1/x"));
        // A port and a path are not the host.
        assert!(allows_destination("http://93.184.216.34:8080/a/b?c=d"));
    }

    /// A name that cannot resolve is refused, not allowed-by-default.
    #[test]
    fn an_unresolvable_name_is_refused() {
        assert!(!allows_destination("http://no-such-host.invalid/x"));
    }

    /// A fetch to a refused destination is an error, and curl is never
    /// started: no request exists to answer.
    #[test]
    fn a_refused_fetch_never_leaves_the_process() {
        let request = Request {
            method: "GET",
            url: "http://127.0.0.1:9/x",
            timeout_secs: 10,
            ..Default::default()
        };
        let error = fetch(&request).expect_err("loopback is refused");
        assert!(
            error.contains("not allowed"),
            "the refusal says why: {error}"
        );
    }

    /// The redirect helpers resolve relative and scheme-relative forms,
    /// and refuse a location that leaves http/https.
    #[test]
    fn redirect_locations_resolve_against_the_current_url() {
        assert_eq!(
            resolve_redirect("https://example.com/a/b", "/c.png").as_deref(),
            Some("https://example.com/c.png")
        );
        assert_eq!(
            resolve_redirect("https://example.com/a/b", "//other.example.com/c.png").as_deref(),
            Some("https://other.example.com/c.png")
        );
        assert_eq!(
            resolve_redirect("https://example.com/a/b", "c.png").as_deref(),
            Some("https://example.com/a/c.png")
        );
        assert_eq!(
            resolve_redirect("https://example.com/a/b", "ftp://example.com/c"),
            None,
            "a redirect to another scheme is refused"
        );
    }

    /// The status line and the location line are read out of curl's dump.
    #[test]
    fn the_header_dump_parses() {
        let dump = "HTTP/1.1 302 Found\r\nLocation: https://example.com/next\r\nContent-Type: text/html\r\n\r\n";
        assert_eq!(status_code(dump), Some(302));
        assert!(is_redirect_status(302));
        assert!(!is_redirect_status(200));
        assert_eq!(
            extract_location(dump).as_deref(),
            Some("https://example.com/next")
        );
        assert_eq!(
            parse_headers(dump),
            vec![
                (
                    "Location".to_string(),
                    "https://example.com/next".to_string()
                ),
                ("Content-Type".to_string(), "text/html".to_string()),
            ]
        );
    }
}
