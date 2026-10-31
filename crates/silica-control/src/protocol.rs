//! The two wire types and typed access to a request's params.
//!
//! Every accessor returns `Result` or `Option` and none of them panics:
//! params arrive from a socket, so a wrong type is an answerable error and
//! not a bug in the caller. JSON `null` counts as absent, because the state
//! dump writes `null` for every absent value and a client that echoes one
//! back should mean the same thing by it.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One request line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// One response line: exactly one of `result` and `error` is present.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Response {
    pub fn ok(id: u64, result: Value) -> Response {
        Response {
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: u64, message: impl std::fmt::Display) -> Response {
        Response {
            id,
            result: None,
            error: Some(message.to_string()),
        }
    }

    /// One line of JSON, newline included.
    ///
    /// Serialization of a `Response` cannot fail — every value in it came from
    /// `serde_json` — but the framing must not be broken by a surprise, so the
    /// fallback is itself a well-formed error line for the same id.
    pub fn line(&self) -> String {
        match serde_json::to_string(self) {
            Ok(mut s) => {
                s.push('\n');
                s
            }
            Err(e) => format!(
                "{{\"id\":{},\"error\":{}}}\n",
                self.id,
                Value::String(format!("unserializable result: {e}"))
            ),
        }
    }
}

impl Request {
    /// A present, non-null param.
    fn param(&self, name: &str) -> Option<&Value> {
        match self.params.get(name) {
            None | Some(Value::Null) => None,
            Some(v) => Some(v),
        }
    }

    fn missing(&self, name: &str) -> String {
        format!("{}: missing param `{}`", self.method, name)
    }

    fn wrong(&self, name: &str, want: &str) -> String {
        format!("{}: param `{}` must be {}", self.method, name, want)
    }

    pub fn str_param(&self, name: &str) -> Result<&str, String> {
        match self.param(name) {
            None => Err(self.missing(name)),
            Some(v) => v.as_str().ok_or_else(|| self.wrong(name, "a string")),
        }
    }

    pub fn opt_str(&self, name: &str) -> Option<&str> {
        self.param(name).and_then(Value::as_str)
    }

    pub fn usize_param(&self, name: &str) -> Result<usize, String> {
        match self.param(name) {
            None => Err(self.missing(name)),
            Some(v) => as_usize(v).ok_or_else(|| self.wrong(name, "a non-negative integer")),
        }
    }

    pub fn opt_usize(&self, name: &str) -> Option<usize> {
        self.param(name).and_then(as_usize)
    }

    pub fn f32_param(&self, name: &str) -> Result<f32, String> {
        match self.param(name) {
            None => Err(self.missing(name)),
            Some(v) => v
                .as_f64()
                .map(|n| n as f32)
                .ok_or_else(|| self.wrong(name, "a number")),
        }
    }

    pub fn opt_f32(&self, name: &str) -> Option<f32> {
        self.param(name).and_then(Value::as_f64).map(|n| n as f32)
    }

    /// Flags have no error case: anything that is not `true` or `false` reads
    /// as the default, so a client may leave one out or send `null` for it.
    pub fn bool_param(&self, name: &str, default: bool) -> bool {
        self.param(name).and_then(Value::as_bool).unwrap_or(default)
    }

    pub fn str_list(&self, name: &str) -> Result<Vec<String>, String> {
        let items = match self.param(name) {
            None => return Err(self.missing(name)),
            Some(Value::Array(items)) => items,
            Some(_) => return Err(self.wrong(name, "an array of strings")),
        };
        items
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| self.wrong(name, "an array of strings"))
            })
            .collect()
    }
}

/// A count, however the client spelled it. MCP clients routinely send an
/// integer as `2.0`, so an integral float counts.
fn as_usize(v: &Value) -> Option<usize> {
    if let Some(n) = v.as_u64() {
        return usize::try_from(n).ok();
    }
    let n = v.as_f64()?;
    if n < 0.0 || n.fract() != 0.0 || n > usize::MAX as f64 {
        return None;
    }
    Some(n as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn req(params: Value) -> Request {
        Request {
            id: 7,
            method: "state".into(),
            params,
        }
    }

    #[test]
    fn a_response_line_round_trips() {
        let line = Response::ok(3, json!({"ok": true})).line();
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        let back: Response = serde_json::from_str(line.trim_end()).expect("parses");
        assert_eq!(back.id, 3);
        assert_eq!(back.result, Some(json!({"ok": true})));
        assert_eq!(back.error, None);

        let line = Response::err(4, "no vault open").line();
        let back: Response = serde_json::from_str(line.trim_end()).expect("parses");
        assert_eq!(back.error.as_deref(), Some("no vault open"));
        // Exactly one of the two keys is on the wire.
        assert!(!line.contains("result"));
    }

    #[test]
    fn a_request_without_params_reads_as_empty() {
        let r: Request = serde_json::from_str(r#"{"id":1,"method":"ping"}"#).expect("parses");
        assert_eq!(r.params, Value::Null);
        assert_eq!(r.opt_str("name"), None);
        assert_eq!(r.opt_usize("rows"), None);
        assert!(r.bool_param("text", true));
        assert!(r.str_param("name").is_err());
    }

    #[test]
    fn wrong_types_are_messages_not_panics() {
        let r = req(json!({"rows": "many", "text": "yes", "keys": ["a", 2], "x": "left"}));
        let e = r.usize_param("rows").expect_err("rejected");
        assert!(e.contains("rows") && e.contains("integer"), "{e}");
        let e = r.str_param("keys").expect_err("rejected");
        assert!(e.contains("string"), "{e}");
        let e = r.f32_param("x").expect_err("rejected");
        assert!(e.contains("number"), "{e}");
        let e = r.str_list("keys").expect_err("rejected");
        assert!(e.contains("array of strings"), "{e}");
        let e = r.str_list("rows").expect_err("rejected");
        assert!(e.contains("array of strings"), "{e}");
        // A flag never fails; a non-bool falls back to the default.
        assert!(r.bool_param("text", true));
        assert!(!r.bool_param("text", false));
        // Missing params name themselves and the method.
        let e = r.str_param("name").expect_err("rejected");
        assert!(e.contains("state") && e.contains("name"), "{e}");
    }

    #[test]
    fn null_counts_as_absent() {
        let r = req(json!({"mode": null, "rows": null}));
        assert_eq!(r.opt_str("mode"), None);
        assert!(r
            .str_param("mode")
            .expect_err("rejected")
            .contains("missing"));
        assert_eq!(r.opt_usize("rows"), None);
    }

    #[test]
    fn numbers_are_read_the_way_clients_write_them() {
        let r = req(json!({"rows": 2.0, "neg": -1, "frac": 1.5, "x": 12, "y": 4.5}));
        assert_eq!(r.usize_param("rows"), Ok(2));
        assert!(r.usize_param("neg").is_err());
        assert!(r.usize_param("frac").is_err());
        assert_eq!(r.f32_param("x"), Ok(12.0));
        assert_eq!(r.opt_f32("y"), Some(4.5));
        assert_eq!(r.opt_f32("missing"), None);
    }

    #[test]
    fn a_string_list_keeps_its_order() {
        let r = req(json!({"keys": ["ctrl-shift-f", "enter"]}));
        assert_eq!(
            r.str_list("keys"),
            Ok(vec!["ctrl-shift-f".to_string(), "enter".to_string()])
        );
        assert_eq!(
            r.str_list("empty"),
            Err("state: missing param `empty`".into())
        );
    }
}
