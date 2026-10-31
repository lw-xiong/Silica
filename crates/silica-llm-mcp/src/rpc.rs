//! JSON-RPC 2.0 over the stdio transport: one JSON object per line.
//!
//! The same framing as `silica-mcp`, and for the same reasons: MCP separates a
//! failed *request* from a failed *call*. An unknown method or malformed
//! params is a JSON-RPC error, because the client got the protocol wrong; but
//! a tool that ran and failed is a successful response carrying `isError`
//! content, because the model is meant to read the failure and try something
//! else. Nothing in this module may panic: the input comes from another
//! process.

use crate::model::Backend;
use crate::tools;
use serde_json::{json, Value};

const VERSION: &str = "2.0";

/// The MCP revisions this server speaks. The first is what it answers with
/// when the client asks for one it does not recognize.
const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;

/// A protocol-level failure: the request itself was wrong.
#[derive(Debug)]
pub struct Fault {
    pub code: i64,
    pub message: String,
}

impl Fault {
    pub fn new(code: i64, message: impl std::fmt::Display) -> Fault {
        Fault {
            code,
            message: message.to_string(),
        }
    }
}

/// One incoming message. `id` is absent for a notification, which must never
/// be answered, not even with an error.
pub struct Incoming {
    pub id: Option<Value>,
    pub method: String,
    pub params: Value,
}

/// Handle one line of stdin. `None` means nothing may be written back.
pub fn handle_line(backend: &Backend, line: &str) -> Option<String> {
    let value: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(e) => return Some(error(Value::Null, PARSE_ERROR, e).to_string()),
    };
    match incoming(&value) {
        Ok(msg) => dispatch(backend, &msg).map(|reply| reply.to_string()),
        Err(reply) => Some(reply.to_string()),
    }
}

/// Pick the three fields apart, treating a null `id` as absent the way the
/// specification does.
fn incoming(value: &Value) -> Result<Incoming, Value> {
    let object = value.as_object().ok_or_else(|| {
        error(
            Value::Null,
            INVALID_REQUEST,
            "expected one JSON-RPC object per line",
        )
    })?;
    let id = object.get("id").filter(|id| !id.is_null()).cloned();
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return Err(error(
            id.unwrap_or(Value::Null),
            INVALID_REQUEST,
            "request has no `method`",
        ));
    };
    Ok(Incoming {
        id,
        method: method.to_string(),
        params: object.get("params").cloned().unwrap_or_else(|| json!({})),
    })
}

/// Run one message. `None` for a notification: every notification this server
/// understands is a no-op, and an unknown one is ignored, as required.
pub fn dispatch(backend: &Backend, msg: &Incoming) -> Option<Value> {
    let result = match msg.method.as_str() {
        "initialize" => Ok(initialize(&msg.params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(tools::list()),
        "tools/call" => tools::call(backend, &msg.params),
        other => Err(Fault::new(
            METHOD_NOT_FOUND,
            format!("unknown method: {other}"),
        )),
    };
    let id = msg.id.clone()?;
    Some(match result {
        Ok(result) => json!({"jsonrpc": VERSION, "id": id, "result": result}),
        Err(fault) => error(id, fault.code, fault.message),
    })
}

/// Echo the client's `protocolVersion` when it is one we speak, so a client
/// pinned to an older revision is not forced to downgrade us.
fn initialize(params: &Value) -> Value {
    let version = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .filter(|asked| PROTOCOL_VERSIONS.contains(asked))
        .unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "silica-llm", "version": env!("CARGO_PKG_VERSION")},
    })
}

fn error(id: Value, code: i64, message: impl std::fmt::Display) -> Value {
    json!({
        "jsonrpc": VERSION,
        "id": id,
        "error": {"code": code, "message": message.to_string()},
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Backend, Config};

    /// A backend pointed at a port nothing listens on: enough to answer every
    /// request that does not reach the model.
    fn offline() -> Backend {
        let mut config = Config::from_env().expect("the default config parses");
        config.url = crate::http::Url::parse("http://127.0.0.1:1").unwrap();
        config.timeout = std::time::Duration::from_millis(500);
        Backend::new(config)
    }

    fn reply(line: &str) -> Value {
        let text = handle_line(&offline(), line).expect("a reply");
        serde_json::from_str(&text).expect("a JSON reply")
    }

    #[test]
    fn initialize_answers_the_handshake() {
        let r = reply(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
        );
        assert_eq!(r["id"], json!(1));
        assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(r["result"]["serverInfo"]["name"], "silica-llm");
        assert_eq!(r["result"]["capabilities"]["tools"], json!({}));
    }

    #[test]
    fn initialize_falls_back_to_our_own_revision() {
        let r =
            reply(r#"{"id":1,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#);
        assert_eq!(r["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
        let r = reply(r#"{"id":1,"method":"initialize"}"#);
        assert_eq!(r["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
    }

    #[test]
    fn notifications_are_never_answered() {
        let backend = offline();
        assert!(handle_line(&backend, r#"{"method":"notifications/initialized"}"#).is_none());
        assert!(handle_line(&backend, r#"{"id":null,"method":"frobnicate"}"#).is_none());
    }

    #[test]
    fn ping_is_an_empty_result() {
        assert_eq!(reply(r#"{"id":7,"method":"ping"}"#)["result"], json!({}));
    }

    #[test]
    fn tools_list_names_all_three() {
        let r = reply(r#"{"id":1,"method":"tools/list"}"#);
        let names: Vec<&str> = r["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["llm_models", "llm_generate", "llm_digest"]);
    }

    #[test]
    fn unknown_method_is_a_jsonrpc_error() {
        let r = reply(r#"{"id":2,"method":"frobnicate"}"#);
        assert_eq!(r["error"]["code"], json!(METHOD_NOT_FOUND));
        assert!(r["result"].is_null());
    }

    #[test]
    fn broken_input_is_an_error_not_a_panic() {
        assert_eq!(reply("{not json")["error"]["code"], json!(PARSE_ERROR));
        assert_eq!(reply("[1,2,3]")["error"]["code"], json!(INVALID_REQUEST));
        let r = reply(r#"{"id":3}"#);
        assert_eq!(r["error"]["code"], json!(INVALID_REQUEST));
        assert_eq!(r["id"], json!(3));
    }

    #[test]
    fn a_server_that_is_not_listening_is_not_a_jsonrpc_error() {
        let r = reply(r#"{"id":4,"method":"tools/call","params":{"name":"llm_models"}}"#);
        assert!(r["error"].is_null(), "{r}");
        assert_eq!(r["result"]["isError"], json!(true));
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("cannot reach"), "{text}");
    }
}
