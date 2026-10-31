//! The blocking end of the socket: write a line, read a line.
//!
//! Blocking is deliberate. The callers are an MCP server handling one
//! JSON-RPC request at a time and integration tests that want the window to
//! have settled before the next assertion, so a request that has not been
//! answered is exactly the thing they should be waiting on.

use crate::net::UnixStream;
use crate::{Request, Response};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct Client {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    next_id: u64,
    /// Set once a reply failed to arrive, or arrived for another request.
    broken: bool,
}

impl Client {
    /// Connect, retrying until `timeout` elapses (the app may still be starting).
    ///
    /// A launcher spawns the app and connects immediately, so the common case
    /// is a handful of refused connections while GPUI opens its window. The
    /// last failure is what the caller gets, because "connection refused" is
    /// more useful than "timed out".
    pub fn connect(path: impl AsRef<Path>, timeout: Duration) -> std::io::Result<Client> {
        let path = path.as_ref();
        let deadline = Instant::now() + timeout;
        loop {
            match UnixStream::connect(path) {
                Ok(stream) => {
                    let reader = BufReader::new(stream.try_clone()?);
                    return Ok(Client {
                        stream,
                        reader,
                        next_id: 1,
                        broken: false,
                    });
                }
                Err(e) => {
                    if Instant::now() >= deadline {
                        return Err(e);
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }

    /// Send one request, block for its response. `Err` is a protocol or transport error string.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.set_timeout(None)?;
        self.exchange(method, params)
    }

    /// `call` with a read timeout for this one request.
    pub fn call_timeout(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        self.set_timeout(Some(timeout))?;
        let result = self.exchange(method, params);
        // Leave the socket blocking again, so a later `call` is not silently
        // bounded by whatever this one asked for.
        let _ = self.set_timeout(None);
        result
    }

    /// Whether this connection is still in step with the app.
    ///
    /// A call that timed out left its reply in flight: the next read would
    /// pick up the previous answer, and every call after it would stay one
    /// reply behind. Only a fresh connection recovers, so a long-lived caller
    /// should drop a broken client and `connect` again.
    pub fn is_broken(&self) -> bool {
        self.broken
    }

    fn set_timeout(&self, timeout: Option<Duration>) -> Result<(), String> {
        // The handle that is read from. On Unix a clone shares the socket
        // and its options; on Windows a duplicated handle keeps a receive
        // timeout of its own, and one set on the writer never fires.
        self.reader
            .get_ref()
            .set_read_timeout(timeout)
            .map_err(|e| format!("control socket: cannot set read timeout: {e}"))
    }

    fn exchange(&mut self, method: &str, params: Value) -> Result<Value, String> {
        if self.broken {
            return Err(format!(
                "control socket: out of step after an earlier failure; reconnect ({method})"
            ));
        }
        let id = self.next_id;
        self.next_id += 1;
        let request = Request {
            id,
            method: method.to_string(),
            params,
        };
        let mut line = serde_json::to_string(&request)
            .map_err(|e| format!("control socket: cannot encode {method}: {e}"))?;
        line.push('\n');
        if let Err(e) = self
            .stream
            .write_all(line.as_bytes())
            .and_then(|()| self.stream.flush())
        {
            // A half-written request is a line the app will never complete.
            self.broken = true;
            return Err(format!("control socket: writing {method}: {e}"));
        }

        let mut reply = String::new();
        match self.reader.read_line(&mut reply) {
            Ok(0) => {
                self.broken = true;
                return Err(format!("control socket: closed while waiting for {method}"));
            }
            Ok(_) => {}
            Err(e) => {
                // Usually the read timeout: the request stands, and its reply
                // will land on top of whatever is asked next.
                self.broken = true;
                return Err(format!("control socket: reading {method}: {e}"));
            }
        }
        let response: Response = match serde_json::from_str(reply.trim_end()) {
            Ok(response) => response,
            Err(e) => {
                // The line was consumed, so the stream is still aligned, but
                // an app talking gibberish is not one to keep questioning.
                self.broken = true;
                return Err(format!("control socket: bad response to {method}: {e}"));
            }
        };
        if response.id != id {
            // The stream is now one reply out of step; say so rather than
            // handing back an answer to someone else's question.
            self.broken = true;
            return Err(format!(
                "control socket: response id {} does not match request {id} ({method})",
                response.id
            ));
        }
        if let Some(error) = response.error {
            return Err(error);
        }
        Ok(response.result.unwrap_or(Value::Null))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::UnixListener;
    use serde_json::json;
    use std::io::BufWriter;
    use std::path::PathBuf;

    /// A socket path of our own. `tempfile` is not a dependency here, and a
    /// socket has to live somewhere short anyway: the sun_path limit is 108
    /// bytes.
    fn socket_path(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("silica-control-{}-{name}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    /// Answer requests the way the app does, for one connection, until the
    /// client hangs up.
    fn serve(listener: UnixListener) {
        let (stream, _) = listener.accept().expect("a connection");
        let reader = BufReader::new(stream.try_clone().expect("clone"));
        let mut writer = BufWriter::new(stream);
        // A client that hung up ends the stream; on Windows it can also
        // reset it, which is the same thing to a server.
        for line in reader.lines().map_while(Result::ok) {
            let request: Request = serde_json::from_str(&line).expect("parses");
            let response = match request.method.as_str() {
                "ping" => Response::ok(request.id, json!({"ok": true, "protocol": 1})),
                "echo" => Response::ok(request.id, request.params.clone()),
                "boom" => Response::err(request.id, "no vault open"),
                // A stuck handler, to be hit with a read timeout.
                "hang" => {
                    std::thread::sleep(Duration::from_millis(500));
                    Response::ok(request.id, Value::Null)
                }
                "liar" => Response::ok(request.id + 100, Value::Null),
                other => Response::err(request.id, format!("unknown method: {other}")),
            };
            // A client that gave up on a slow reply is already gone; that is
            // its business, not an error here.
            if writer.write_all(response.line().as_bytes()).is_err() || writer.flush().is_err() {
                break;
            }
        }
    }

    #[test]
    fn a_call_round_trips_over_a_socket() {
        let path = socket_path("round-trip");
        let listener = UnixListener::bind(&path).expect("binds");
        let server = std::thread::spawn(move || serve(listener));

        let mut client = Client::connect(&path, Duration::from_secs(2)).expect("connects");
        assert_eq!(
            client.call("ping", json!({})),
            Ok(json!({"ok": true, "protocol": 1}))
        );
        // Params survive the trip, and ids advance.
        assert_eq!(
            client.call("echo", json!({"rows": 5, "text": true})),
            Ok(json!({"rows": 5, "text": true}))
        );
        // An error response is an Err with the app's own message.
        assert_eq!(
            client.call("boom", Value::Null),
            Err("no vault open".into())
        );
        assert_eq!(
            client.call("frobnicate", json!({})),
            Err("unknown method: frobnicate".into())
        );
        // A mismatched id is an error, not a panic.
        let e = client.call("liar", json!({})).expect_err("mismatch");
        assert!(e.contains("does not match"), "{e}");

        drop(client);
        server.join().expect("the server thread");
        std::fs::remove_file(&path).expect("removes the socket");
    }

    #[test]
    fn connect_waits_for_the_socket_to_appear() {
        let path = socket_path("late-bind");
        let late = path.clone();
        let server = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(120));
            let listener = UnixListener::bind(&late).expect("binds");
            serve(listener);
        });

        let mut client = Client::connect(&path, Duration::from_secs(3)).expect("connects");
        let reply = client.call("ping", json!({})).expect("a reply");
        assert_eq!(reply["ok"], json!(true));
        drop(client);
        server.join().expect("the server thread");
        std::fs::remove_file(&path).expect("removes the socket");
    }

    #[test]
    fn connect_gives_up_when_nothing_ever_binds() {
        let path = socket_path("nobody-home");
        let e = Client::connect(&path, Duration::from_millis(50)).expect_err("no socket");
        // A missing socket file is not found on Unix; WinSock calls it refused.
        assert!(
            matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ),
            "{e:?}"
        );
    }

    #[test]
    fn a_read_timeout_is_an_error_string() {
        let path = socket_path("timeout");
        let listener = UnixListener::bind(&path).expect("binds");
        let server = std::thread::spawn(move || serve(listener));

        let mut client = Client::connect(&path, Duration::from_secs(2)).expect("connects");
        let e = client
            .call_timeout("hang", json!({}), Duration::from_millis(30))
            .expect_err("times out");
        assert!(e.contains("reading hang"), "{e}");

        drop(client);
        server.join().expect("the server thread");
        std::fs::remove_file(&path).expect("removes the socket");
    }

    #[test]
    fn a_timed_out_client_refuses_to_be_reused() {
        let path = socket_path("desync");
        let listener = UnixListener::bind(&path).expect("binds");
        let server = std::thread::spawn(move || serve(listener));

        let mut client = Client::connect(&path, Duration::from_secs(2)).expect("connects");
        assert!(!client.is_broken());
        client
            .call_timeout("hang", json!({}), Duration::from_millis(30))
            .expect_err("times out");
        assert!(client.is_broken(), "a timed-out reply is still in flight");

        // Without this the slow reply to `hang` would be read as the answer to
        // `ping`, and every call after it would stay one reply behind.
        let e = client.call("ping", json!({})).expect_err("refuses");
        assert!(e.contains("out of step"), "{e}");
        assert!(e.contains("ping"), "{e}");

        drop(client);
        server.join().expect("the server thread");
        std::fs::remove_file(&path).expect("removes the socket");
    }

    #[test]
    fn an_app_level_error_leaves_the_client_usable() {
        let path = socket_path("healthy-error");
        let listener = UnixListener::bind(&path).expect("binds");
        let server = std::thread::spawn(move || serve(listener));

        let mut client = Client::connect(&path, Duration::from_secs(2)).expect("connects");
        assert_eq!(
            client.call("boom", Value::Null),
            Err("no vault open".into())
        );
        // An app that answered "no vault open" is in step and healthy.
        assert!(!client.is_broken());
        assert_eq!(
            client.call("ping", json!({})).expect("a reply")["ok"],
            json!(true)
        );

        drop(client);
        server.join().expect("the server thread");
        std::fs::remove_file(&path).expect("removes the socket");
    }
}
