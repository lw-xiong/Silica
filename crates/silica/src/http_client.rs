use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use futures::future::BoxFuture;
use futures::FutureExt;
use http_client::{
    http::{HeaderValue, Response, StatusCode},
    AsyncBody, HttpClient, Url,
};

/// The largest remote image Silica will keep. A note's picture is a picture;
/// anything past this is not one, and the disk is not the place to find out.
pub const MAX_BYTES: u64 = 32 * 1024 * 1024;

/// How many redirects a fetch may follow. Each hop is revalidated against the
/// private-network rules before the connection is made, so a hostile redirect
/// chain cannot reach an address the original URL was refused.
pub const MAX_REDIRECTS: u32 = 5;

/// The largest remote-image cache Silica will keep on disk. A note's picture
/// is a picture; the cache is a cache, not an archive, and it is not the
/// place to let a hostile note fill the disk. Oldest entries are evicted
/// first.
pub const MAX_CACHE_BYTES: u64 = 100 * 1024 * 1024;

/// The total size of every file in `dir`, in bytes.
/// A string no two in-flight downloads will pick at the same time.
///
/// The process id keeps separate runs apart; the counter keeps threads within
/// one run apart, and the clock keeps a counter reset (a fork, a wrapped
/// sequence) from repeating a name a previous attempt still holds.
fn unique_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{}-{n}-{nanos}", std::process::id())
}

fn dir_size(dir: &Path) -> std::io::Result<u64> {
    let mut total = 0u64;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_file() {
            total += meta.len();
        }
    }
    Ok(total)
}

/// Evicts the oldest files in `dir` until its total size is at or below
/// `limit`, so the cache cannot grow without bound. Best effort: a file that
/// disappears between listing and removal is simply skipped.
fn evict_to_limit(dir: &Path, limit: u64) {
    let mut files: Vec<(std::path::PathBuf, std::time::SystemTime)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    if let Ok(mtime) = meta.modified() {
                        files.push((path, mtime));
                    }
                }
            }
        }
    }
    files.sort_by_key(|(_, mtime)| *mtime);
    let mut total = dir_size(dir).unwrap_or(0);
    for (path, _) in files {
        if total <= limit {
            break;
        }
        // * The accounting follows the removal: a file that could not be
        // * deleted is still on disk, so subtracting its size would make the
        // * cache look smaller than it is and the next sweep would stop early.
        let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

/// Removes temp files left behind by an interrupted download, so a crashed
/// run cannot leave the cache full of partial files.
fn sweep_temp_files(dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            // * Only our own prefix, exactly. Matching a bare `.tmp` suffix or
            // * a `.silica-tmp` substring would delete a user file that merely
            // * looks like a download — the cache dir holds images, not only
            // * ours.
            if name.starts_with(".silica-tmp-") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

pub struct SilicaHttpClient {
    cache_dir: PathBuf,
}

/// Whether `url` may be fetched at all.
///
/// A note can name any image it likes, so the request goes where the note
/// says. That makes the note able to probe whatever the app can reach, which
/// on a laptop includes the router's admin page and anything on `localhost`.
/// A literal private or loopback host is refused here, before any request.
///
/// A hostname that resolves to a private address is refused here, before any
/// request: the name is resolved and every candidate address is checked, so
/// DNS rebinding cannot reach the local network through a look-alike name.
pub fn allows_destination(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return false;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    // * Drop any userinfo, then any port.
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = if host.starts_with('[') {
        // * A bracketed IPv6 literal.
        host.split(']')
            .next()
            .unwrap_or(host)
            .trim_start_matches('[')
    } else {
        host.split(':').next().unwrap_or(host)
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
    //   common rebinding case and the `--resolve` argument below pins the one we
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

/// What a response's status line tells the redirect loop to do next, given how
/// many redirects have already been followed.
enum Next {
    /// A redirect whose Location is safe to validate and follow.
    Follow(String),
    /// Still redirecting with the budget spent: refused, never treated as the
    /// final answer.
    TooManyRedirects,
    /// Not a redirect (or one with nowhere to go): the body stands.
    Finish,
}

/// Decide the next move from the raw header block curl wrote to disk.
fn next_hop(headers: &str, redirects: u32) -> Next {
    let code = headers
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse::<u16>().ok())
        .unwrap_or(0);
    let Some(location) = extract_location(headers).filter(|_| is_redirect_status(code)) else {
        // * Not a redirect, or one we cannot follow: the body stands.
        return Next::Finish;
    };
    if redirects >= MAX_REDIRECTS {
        return Next::TooManyRedirects;
    }
    Next::Follow(location)
}

/// Loopback, link-local, private, and the other ranges that mean "this
/// machine or its network" rather than "the internet".
fn is_private(ip: std::net::IpAddr) -> bool {
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
fn is_redirect_status(code: u16) -> bool {
    matches!(code, 301 | 302 | 303 | 307 | 308)
}

/// Extracts the `Location` header value from the headers dump curl writes
/// with `-D`.
fn extract_location(headers: &str) -> Option<String> {
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
fn resolve_redirect(base: &str, location: &str) -> Option<String> {
    let base_url = Url::parse(base).ok()?;
    let resolved = base_url.join(location).ok()?;
    let scheme = resolved.scheme();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    Some(resolved.to_string())
}

impl SilicaHttpClient {
    pub fn new() -> Self {
        let cache_dir = if let Some(cache) = std::env::var_os("XDG_CACHE_HOME") {
            PathBuf::from(cache).join("silica").join("remote_images")
        } else if let Some(home) = std::env::var_os("HOME") {
            PathBuf::from(home)
                .join(".cache")
                .join("silica")
                .join("remote_images")
        } else {
            std::env::temp_dir().join("silica").join("remote_images")
        };
        let _ = std::fs::create_dir_all(&cache_dir);
        // * A crashed run can leave partial downloads behind; sweep them before
        //   the cache is used, and keep the cache within its size budget.
        sweep_temp_files(&cache_dir);
        evict_to_limit(&cache_dir, MAX_CACHE_BYTES);
        Self { cache_dir }
    }

    pub fn cache_path(&self, url: &str) -> PathBuf {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        url.hash(&mut hasher);
        let hash = hasher.finish();
        let ext = Path::new(url.split('?').next().unwrap_or(url))
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("img");
        self.cache_dir.join(format!("{:016x}.{}", hash, ext))
    }
}

impl Default for SilicaHttpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpClient for SilicaHttpClient {
    fn user_agent(&self) -> Option<&HeaderValue> {
        None
    }

    fn proxy(&self) -> Option<&Url> {
        None
    }

    fn send(
        &self,
        req: http_client::http::Request<AsyncBody>,
    ) -> BoxFuture<'static, Result<Response<AsyncBody>>> {
        let uri = req.uri().to_string();
        let method = req.method().clone();
        if !allows_destination(&uri) {
            return async move {
                Err(anyhow!(
                    "{uri} is not an http(s) URL, or it points into a private network, so it is not fetched"
                ))
            }
            .boxed();
        }
        // ! Pin the connection to the public address the name resolves to, so
        //   DNS rebinding cannot send the request somewhere else at connect
        //   time. A name that cannot be resolved, or that resolves only to a
        //   private address, is refused.
        let Some(resolve_arg) = resolve_target(&uri) else {
            return async move {
                Err(anyhow!(
                    "{uri} could not be resolved to a public address, so it is not fetched"
                ))
            }
            .boxed();
        };
        let cache_path = self.cache_path(&uri);

        async move {
            if method == http_client::http::Method::GET
                && cache_path.is_file() {
                    if let Ok(data) = std::fs::read(&cache_path) {
                        if !data.is_empty() {
                            let resp = Response::builder()
                                .status(StatusCode::OK)
                                .body(AsyncBody::from_bytes(data.into()))?;
                            return Ok(resp);
                        }
                    }
                }

            let (tx, rx) = futures::channel::oneshot::channel();
            let uri_clone = uri.clone();
            let cache_path_clone = cache_path.clone();
            let resolve_clone = resolve_arg.clone();

            std::thread::spawn(move || {
                // * Ponytail: one curl process per cache miss costs ~10 ms of
                //   spawn overhead and no connection reuse. A pooled async
                //   client (reqwest+rustls) would cut that, but it cannot pin
                //   the resolved IP per hop the way `--resolve` does here, so
                //   the SSRF boundary — validated redirects onto pinned
                //   addresses — would have to be rebuilt around it first.
                //   Upgrade path: swap this loop for a pooled client once a
                //   per-hop DNS resolver exists; the validation helpers stay.
                //
                // * Follow redirects manually, validating every hop, so a
                //   redirect chain cannot bounce to a private address that the
                //   original URL passed.
                let mut current_uri = uri_clone;
                let mut current_resolve = resolve_clone;
                let mut redirects = 0u32;

                let result = 'redirects: {
                    loop {
                        // * Unique per attempt, not derived from the cache
                        // * name: two downloads of the same URL — or two
                        // * windows showing the same note — would otherwise
                        // * share one `.tmp` and truncate each other's bytes.
                        // * The prefix is what `sweep_temp_files` looks for.
                        let unique = unique_suffix();
                        let temp_path = cache_path_clone
                            .with_file_name(format!(".silica-tmp-{unique}.part"));
                        let headers_path = cache_path_clone
                            .with_file_name(format!(".silica-tmp-{unique}.headers"));

                        let status = std::process::Command::new("curl")
                            .arg("-s")
                            .arg("-f")
                            .arg("--no-config")
                            .arg("--noproxy")
                            .arg("*")
                            // * No -L: we follow redirects ourselves so each hop
                            //   can be validated against the private-network
                            //   rules before the connection is made.
                            .arg("--proto").arg("=http,https")
                            .arg("--proto-redir").arg("=http,https")
                            .arg("--resolve").arg(&current_resolve)
                            .arg("--max-filesize").arg(MAX_BYTES.to_string())
                            .arg("--max-time").arg("30")
                            .arg("-D").arg(&headers_path)
                            .arg("-o").arg(&temp_path)
                            .arg(&current_uri)
                            .status();

                        let cleanup = || {
                            let _ = std::fs::remove_file(&temp_path);
                            let _ = std::fs::remove_file(&headers_path);
                        };

                        match status {
                            Ok(s) if s.success() => {
                                let headers = std::fs::read_to_string(&headers_path)
                                    .unwrap_or_default();

                                match next_hop(&headers, redirects) {
                                    Next::Follow(location) => {
                                        // Resolve relative redirects (a path, or
                                        // a protocol-relative URL) against the
                                        // current URL before validating, so
                                        // `/images/file.png` becomes an absolute
                                        // http/https URL instead of being
                                        // rejected outright.
                                        let Some(resolved) =
                                            resolve_redirect(&current_uri, &location)
                                        else {
                                            cleanup();
                                            break 'redirects Err(anyhow!(
                                                "redirect to {location} could not be resolved to an absolute URL",
                                            ));
                                        };
                                        if !allows_destination(&resolved) {
                                            cleanup();
                                            break 'redirects Err(anyhow!(
                                                "redirect to {} is not allowed",
                                                resolved
                                            ));
                                        }
                                        let Some(resolve) = resolve_target(&resolved) else {
                                            cleanup();
                                            break 'redirects Err(anyhow!(
                                                "redirect to {} could not be resolved to a public address",
                                                resolved
                                            ));
                                        };
                                        current_uri = resolved;
                                        current_resolve = resolve;
                                        redirects += 1;
                                        cleanup();
                                        continue;
                                    }
                                    // * The budget is spent and the server is
                                    //   still redirecting: refusing is the
                                    //   only honest answer. Treating it as the
                                    //   final response would cache a redirect
                                    //   page's body as the picture and hand it
                                    //   back with a 200.
                                    Next::TooManyRedirects => {
                                        cleanup();
                                        break 'redirects Err(anyhow!(
                                            "too many redirects fetching {current_uri}"
                                        ));
                                    }
                                    Next::Finish => {}
                                }

                                // * Final response (not a redirect, or too many
                                //   redirects): keep the body, discard headers.
                                let _ = std::fs::remove_file(&headers_path);

                                match std::fs::metadata(&temp_path) {
                                    Ok(meta) if meta.len() > MAX_BYTES => {
                                        let _ = std::fs::remove_file(&temp_path);
                                        break 'redirects Err(anyhow!(
                                            "{} is larger than the {} byte limit for a remote image",
                                            current_uri,
                                            MAX_BYTES
                                        ));
                                    }
                                    _ => {
                                        let _ = std::fs::rename(&temp_path, &cache_path_clone);
                                        // * Keep the cache within its size budget:
                                        //   evict the oldest entries if this write put
                                        //   it over the limit.
                                        if let Some(dir) = cache_path_clone.parent() {
                                            evict_to_limit(dir, MAX_CACHE_BYTES);
                                        }
                                        match std::fs::read(&cache_path_clone) {
                                            Ok(data) => break 'redirects Ok(data),
                                            Err(e) => {
                                                break 'redirects Err(anyhow!(e));
                                            }
                                        }
                                    }
                                }
                            }
                            Ok(s) => {
                                cleanup();
                                break 'redirects Err(anyhow!("curl failed with status: {s}"));
                            }
                            Err(e) => {
                                cleanup();
                                break 'redirects Err(anyhow!("failed to spawn curl: {e}"));
                            }
                        }
                    }
                };

                let _ = tx.send(result);
            });

            let data = rx.await.map_err(|_| anyhow!("fetch cancelled"))??;
            let resp = Response::builder()
                .status(StatusCode::OK)
                .body(AsyncBody::from_bytes(data.into()))?;
            Ok(resp)
        }
        .boxed()
    }
}

#[cfg(test)]
mod destination_tests {
    use super::*;

    #[test]
    fn a_public_image_url_is_allowed() {
        for url in [
            "https://example.com/cat.png",
            "http://example.com/a/b.jpg",
            "https://user:pass@example.com/x.png",
            "https://example.com:8443/x.png",
            "https://93.184.216.34/x.png",
        ] {
            assert!(allows_destination(url), "{url} was refused");
        }
    }

    #[test]
    fn loopback_and_private_networks_are_refused() {
        for url in [
            "http://localhost/x.png",
            "http://LOCALHOST/x.png",
            "http://localhost:8080/x.png",
            "http://app.localhost/x.png",
            "http://127.0.0.1/x.png",
            "http://127.1.2.3/x.png",
            "http://[::1]/x.png",
            "http://10.0.0.5/x.png",
            "http://192.168.1.1/x.png",
            "http://172.16.0.1/x.png",
            "http://169.254.169.254/latest/meta-data/",
            "http://100.64.0.1/x.png",
            "http://0.0.0.0/x.png",
            "http://[fd00::1]/x.png",
            "http://[fe80::1]/x.png",
        ] {
            assert!(!allows_destination(url), "{url} was allowed");
        }
    }

    #[test]
    fn only_http_and_https_are_fetched() {
        for url in [
            "file:///etc/passwd",
            "ftp://example.com/x",
            "data:text/html,x",
        ] {
            assert!(!allows_destination(url), "{url} was allowed");
        }
        assert!(!allows_destination("example.com/x.png"), "no scheme");
    }
}

#[cfg(test)]
mod next_hop_tests {
    use super::*;

    #[test]
    fn a_redirect_with_budget_left_is_followed() {
        let headers = "HTTP/1.1 302 Found\r\nLocation: https://example.com/next.png\r\n";
        assert!(matches!(
            next_hop(headers, MAX_REDIRECTS - 1),
            Next::Follow(_)
        ));
        assert!(matches!(next_hop(headers, 0), Next::Follow(_)));
    }

    #[test]
    fn a_redirect_with_the_budget_spent_is_refused_not_finished() {
        // ! Before this guard, the sixth redirect fell through to the
        //     "final response" branch: its body was renamed into the image
        //     cache and handed back as the picture with a 200.
        let headers = "HTTP/1.1 302 Found\r\nLocation: https://example.com/loop.png\r\n";
        assert!(matches!(
            next_hop(headers, MAX_REDIRECTS),
            Next::TooManyRedirects
        ));
        assert!(matches!(
            next_hop(headers, MAX_REDIRECTS + 7),
            Next::TooManyRedirects
        ));
    }

    #[test]
    fn a_non_redirect_is_finished() {
        assert!(matches!(
            next_hop("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\n", 0),
            Next::Finish
        ));
        // * A redirect with no Location has nowhere to go: its body stands,
        //   exactly as before.
        assert!(matches!(
            next_hop("HTTP/1.1 302 Found\r\nContent-Length: 0\r\n", 0),
            Next::Finish
        ));
        // * An unreadable status line is not evidence of a redirect.
        assert!(matches!(next_hop("", MAX_REDIRECTS), Next::Finish));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_path_deterministic_and_preserves_ext() {
        let client = SilicaHttpClient::new();
        let p1 = client.cache_path("https://example.com/images/cat.png");
        let p2 = client.cache_path("https://example.com/images/cat.png");
        assert_eq!(p1, p2);
        assert_eq!(p1.extension().and_then(|s| s.to_str()), Some("png"));

        let p3 = client.cache_path("https://example.com/photo.jpeg?size=large");
        assert_eq!(p3.extension().and_then(|s| s.to_str()), Some("jpeg"));
    }

    #[test]
    fn disk_cache_hit_returns_cached_bytes() {
        let client = SilicaHttpClient::new();
        let url = "https://example.com/dummy_cached_test_image.png";
        let cache_path = client.cache_path(url);
        let test_bytes = b"fake-png-content";
        std::fs::write(&cache_path, test_bytes).unwrap();

        let req = http_client::http::Request::builder()
            .uri(url)
            .body(AsyncBody::empty())
            .unwrap();
        let future = client.send(req);
        let resp = futures::executor::block_on(future).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let _ = std::fs::remove_file(cache_path);
    }

    #[test]
    fn a_public_name_resolves_to_a_public_address() {
        assert!(allows_destination("http://example.com/x.png"));
        assert!(allows_destination("https://example.com/x.png"));
        let resolve = resolve_target("http://example.com/x.png").unwrap();
        assert!(resolve.starts_with("example.com:80:"));
        // ! The resolved address must not be private or loopback.
        let ip = resolve
            .split_once(':')
            .and_then(|(_, rest)| rest.split_once(':').map(|(_, ip)| ip))
            .unwrap();
        let ip: std::net::IpAddr = ip.parse().unwrap();
        assert!(!is_private(ip));
    }

    #[test]
    fn an_unresolvable_name_is_refused() {
        assert!(!allows_destination(
            "http://this-name-does-not-exist-silica-test.invalid/x.png"
        ));
        assert!(
            resolve_target("http://this-name-does-not-exist-silica-test.invalid/x.png").is_none()
        );
    }

    #[test]
    fn resolve_target_pins_https_to_443() {
        let resolve = resolve_target("https://example.com/x.png").unwrap();
        assert!(resolve.starts_with("example.com:443:"));
    }

    #[test]
    fn dir_size_sums_only_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.bin"), b"hello").unwrap();
        std::fs::write(dir.path().join("b.bin"), b"world!").unwrap();
        std::fs::write(dir.path().join("c.bin"), b"!!").unwrap();
        assert_eq!(dir_size(dir.path()).unwrap(), 13);
    }

    #[test]
    fn sweep_temp_files_removes_partial_downloads() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("real.png"), b"png").unwrap();
        // * A user's own `.tmp` and a name that merely contains the old
        // * substring must both survive; only our prefix is swept.
        std::fs::write(dir.path().join("partial.png.tmp"), b"part").unwrap();
        std::fs::write(dir.path().join("image.silica-tmp"), b"part2").unwrap();
        std::fs::write(dir.path().join(".silica-tmp-123-0-0.part"), b"partial").unwrap();
        sweep_temp_files(dir.path());
        assert!(dir.path().join("real.png").exists());
        assert!(
            dir.path().join("partial.png.tmp").exists(),
            "a user file ending in .tmp must not be swept"
        );
        assert!(
            dir.path().join("image.silica-tmp").exists(),
            "a name merely containing the substring must not be swept"
        );
        assert!(!dir.path().join(".silica-tmp-123-0-0.part").exists());
    }

    #[test]
    fn evict_to_limit_removes_files_when_over_limit() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["a.bin", "b.bin", "c.bin"] {
            std::fs::write(dir.path().join(name), b"0123456789").unwrap();
        }
        assert_eq!(dir_size(dir.path()).unwrap(), 30);
        evict_to_limit(dir.path(), 15);
        assert!(
            dir_size(dir.path()).unwrap() <= 15,
            "the cache was not evicted"
        );
        // * At least one file had to go; at least one had to stay.
        let remaining: Vec<_> = ["a.bin", "b.bin", "c.bin"]
            .iter()
            .filter(|n| dir.path().join(n).exists())
            .collect();
        assert!(!remaining.is_empty(), "everything was evicted");
        assert!(remaining.len() < 3, "nothing was evicted");
    }

    #[test]
    fn evict_to_limit_keeps_everything_under_limit() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.bin"), b"hi").unwrap();
        std::fs::write(dir.path().join("b.bin"), b"ho").unwrap();
        evict_to_limit(dir.path(), 100);
        assert!(dir.path().join("a.bin").exists());
        assert!(dir.path().join("b.bin").exists());
    }

    #[test]
    fn concurrent_writes_stay_under_limit_and_leave_no_temps() {
        // ! Simulates the cache under load: many threads write files and
        //     evict concurrently, the way parallel image downloads would.
        //     The cache must never exceed its budget, and no temp files may
        //     survive the sweep.
        let dir = tempfile::tempdir().unwrap();
        let limit = 10_000;
        let threads = 8;
        let files_per_thread = 20;

        let path = dir.path().to_path_buf();
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let p = path.clone();
                std::thread::spawn(move || {
                    for i in 0..files_per_thread {
                        let name = format!("t{t}-f{i}.bin");
                        let bytes = vec![0u8; 500 + (i * 37) % 300];
                        std::fs::write(p.join(&name), &bytes).unwrap();
                        evict_to_limit(&p, limit);
                        sweep_temp_files(&p);
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        // * The cache must be at or under the budget.
        let size = dir_size(&path).expect("the cache dir is readable");
        assert!(
            size <= limit,
            "cache grew past its budget: {size} > {limit}"
        );

        // * No temp files may remain.
        for entry in std::fs::read_dir(&path).unwrap().flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            assert!(
                !name.starts_with(".silica-tmp-"),
                "a temp file survived the sweep: {name}"
            );
        }
    }

    #[test]
    fn redirect_status_codes_are_recognised() {
        for code in [301, 302, 303, 307, 308] {
            assert!(is_redirect_status(code), "{code} should be a redirect");
        }
        for code in [200, 404, 500, 201, 204, 304] {
            assert!(!is_redirect_status(code), "{code} should not be a redirect");
        }
    }

    #[test]
    fn location_header_is_extracted_case_insensitively() {
        let headers = "HTTP/1.1 302 Found\r\nLocation: https://example.com/elsewhere\r\nContent-Length: 0\r\n";
        assert_eq!(
            extract_location(headers),
            Some("https://example.com/elsewhere".to_string())
        );
        // * Case-insensitive header name.
        let headers_lower = "HTTP/1.1 301 Moved\r\nlocation: http://example.com/x\r\n";
        assert_eq!(
            extract_location(headers_lower),
            Some("http://example.com/x".to_string())
        );
        // * No Location header.
        let no_location = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n";
        assert_eq!(extract_location(no_location), None);
    }

    #[test]
    fn a_relative_redirect_is_resolved_against_the_current_url() {
        // * A path-relative Location resolves against the current URL's origin.
        assert_eq!(
            resolve_redirect("https://example.com/a.png", "/images/file.png"),
            Some("https://example.com/images/file.png".to_string())
        );
        // * A protocol-relative Location inherits the current scheme.
        assert_eq!(
            resolve_redirect("https://example.com/a.png", "//example.com/x.png"),
            Some("https://example.com/x.png".to_string())
        );
        // * An absolute Location passes through unchanged.
        assert_eq!(
            resolve_redirect("https://example.com/a.png", "https://other.com/b.png"),
            Some("https://other.com/b.png".to_string())
        );
        // * A non-http scheme is refused, not redirected to.
        assert_eq!(
            resolve_redirect("https://example.com/a.png", "javascript:alert(1)"),
            None
        );
        assert_eq!(
            resolve_redirect("https://example.com/a.png", "data:text/html,<x>"),
            None
        );
        // * A location that cannot be resolved against the base is refused.
        assert_eq!(resolve_redirect("not-a-url", "/x.png"), None);
    }

    #[test]
    fn a_redirect_to_a_private_address_would_be_refused() {
        // ! The redirect-following loop calls allows_destination on every
        //     Location header. A redirect to a private address must be caught
        //     here, before curl ever connects to it.
        for url in [
            "http://127.0.0.1/secret",
            "http://10.0.0.5/admin",
            "http://192.168.1.1/router",
            "http://169.254.169.254/metadata",
            "http://localhost/loopback",
        ] {
            assert!(
                !allows_destination(url),
                "a redirect to {url} was not refused"
            );
            assert!(
                resolve_target(url).is_none(),
                "a redirect to {url} was not refused by resolve_target"
            );
        }
    }

    #[test]
    fn a_redirect_to_a_public_address_would_be_allowed() {
        let url = "https://example.com/redirected-image.png";
        assert!(allows_destination(url));
        assert!(resolve_target(url).is_some());
    }
}
