//! JSON-RPC 2.0 over the stdio transport: one JSON object per line.
//!
//! MCP separates a failed *request* from a failed *call*. An unknown method or
//! malformed params is a JSON-RPC error, because the client got the protocol
//! wrong; but a tool that ran and failed is a successful response carrying
//! `isError` content, because the model is meant to read the failure and try
//! something else. Everything here keeps those two apart, and nothing in this
//! module may panic: the input comes from another process.

use crate::app::App;
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

/// A protocol-level failure: the request itself was wrong. A tool that merely
/// returned an error is not one of these.
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
pub fn handle_line(app: &mut App, line: &str) -> Option<String> {
    let value: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(e) => return Some(error(Value::Null, PARSE_ERROR, e).to_string()),
    };
    match incoming(&value) {
        Ok(msg) => dispatch(app, &msg).map(|reply| reply.to_string()),
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
pub fn dispatch(app: &mut App, msg: &Incoming) -> Option<Value> {
    let result = match msg.method.as_str() {
        "initialize" => Ok(initialize(&msg.params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(tools::list()),
        "tools/call" => tools::call(app, &msg.params),
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
        "serverInfo": {"name": "silica", "version": env!("CARGO_PKG_VERSION")},
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
    use crate::app::{App, Config};

    /// An `App` with a socket path nothing listens on: enough to answer every
    /// request that does not reach the app.
    fn offline() -> App {
        App::new(Config::for_test(
            std::env::temp_dir().join("silica-mcp-rpc-none.sock"),
        ))
    }

    fn reply(line: &str) -> Value {
        let text = handle_line(&mut offline(), line).expect("a reply");
        serde_json::from_str(&text).expect("a JSON reply")
    }

    #[test]
    fn initialize_answers_the_handshake() {
        let r = reply(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
        );
        assert_eq!(r["id"], json!(1));
        assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(r["result"]["serverInfo"]["name"], "silica");
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
        let mut app = offline();
        assert!(handle_line(
            &mut app,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
        )
        .is_none());
        assert!(handle_line(
            &mut app,
            r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}"#
        )
        .is_none());
        // A null id is an absent id, so even an unknown notification is silent.
        assert!(handle_line(&mut app, r#"{"id":null,"method":"frobnicate"}"#).is_none());
    }

    #[test]
    fn ping_is_an_empty_result() {
        assert_eq!(reply(r#"{"id":7,"method":"ping"}"#)["result"], json!({}));
    }

    #[test]
    fn unknown_method_is_a_jsonrpc_error() {
        let r = reply(r#"{"id":2,"method":"frobnicate"}"#);
        assert_eq!(r["error"]["code"], json!(METHOD_NOT_FOUND));
        assert!(r["error"]["message"]
            .as_str()
            .unwrap()
            .contains("frobnicate"));
        assert!(r["result"].is_null());
    }

    #[test]
    fn broken_input_is_an_error_not_a_panic() {
        let r = reply("{not json");
        assert_eq!(r["error"]["code"], json!(PARSE_ERROR));
        assert_eq!(r["id"], Value::Null);
        let r = reply("[1,2,3]");
        assert_eq!(r["error"]["code"], json!(INVALID_REQUEST));
        let r = reply(r#"{"id":3}"#);
        assert_eq!(r["error"]["code"], json!(INVALID_REQUEST));
        assert_eq!(r["id"], json!(3));
    }

    #[test]
    fn a_tool_that_cannot_reach_the_app_is_not_a_jsonrpc_error() {
        let r = reply(r#"{"id":4,"method":"tools/call","params":{"name":"silica_state"}}"#);
        assert!(r["error"].is_null(), "{r}");
        assert_eq!(r["result"]["isError"], json!(true));
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("silica_launch"), "{text}");
    }

    #[test]
    fn an_unknown_tool_is_invalid_params() {
        let r = reply(r#"{"id":5,"method":"tools/call","params":{"name":"silica_dance"}}"#);
        assert_eq!(r["error"]["code"], json!(INVALID_PARAMS));
    }
}
