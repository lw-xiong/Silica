use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use futures::future::BoxFuture;
use futures::FutureExt;
use http_client::{
    http::{HeaderValue, Response, StatusCode},
    AsyncBody, HttpClient, Url,
};

pub struct SilicaHttpClient {
    cache_dir: PathBuf,
}

impl SilicaHttpClient {
    pub fn new() -> Self {
        let cache_dir = if let Some(cache) = std::env::var_os("XDG_CACHE_HOME") {
            PathBuf::from(cache).join("silica").join("remote_images")
        } else if let Some(home) = std::env::var_os("HOME") {
            PathBuf::from(home).join(".cache").join("silica").join("remote_images")
        } else {
            std::env::temp_dir().join("silica").join("remote_images")
        };
        let _ = std::fs::create_dir_all(&cache_dir);
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
        let cache_path = self.cache_path(&uri);

        async move {
            if method == http_client::http::Method::GET {
                if cache_path.is_file() {
                    if let Ok(data) = std::fs::read(&cache_path) {
                        if !data.is_empty() {
                            let resp = Response::builder()
                                .status(StatusCode::OK)
                                .body(AsyncBody::from_bytes(data.into()))?;
                            return Ok(resp);
                        }
                    }
                }
            }

            let (tx, rx) = futures::channel::oneshot::channel();
            let uri_clone = uri.clone();
            let cache_path_clone = cache_path.clone();

            std::thread::spawn(move || {
                let temp_path = cache_path_clone.with_extension("tmp");
                let status = std::process::Command::new("curl")
                    .arg("-s")
                    .arg("-f")
                    .arg("-L")
                    .arg("--max-time")
                    .arg("30")
                    .arg(&uri_clone)
                    .arg("-o")
                    .arg(&temp_path)
                    .status();

                let res = match status {
                    Ok(s) if s.success() => {
                        let _ = std::fs::rename(&temp_path, &cache_path_clone);
                        std::fs::read(&cache_path_clone).map_err(|e| anyhow!(e))
                    }
                    Ok(s) => {
                        let _ = std::fs::remove_file(&temp_path);
                        Err(anyhow!("curl failed with status: {}", s))
                    }
                    Err(e) => {
                        let _ = std::fs::remove_file(&temp_path);
                        Err(anyhow!("failed to spawn curl: {}", e))
                    }
                };
                let _ = tx.send(res);
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
        let url = "https://test.local/dummy_cached_test_image.png";
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
}
