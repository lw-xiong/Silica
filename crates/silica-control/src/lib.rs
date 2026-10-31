//! The control protocol Silica's app speaks on a Unix socket: its wire types,
//! its method table, and a blocking client for the other end.
//!
//! The app is a GPUI window, so nothing outside it can see what it is
//! showing or press a key in it. A line-delimited JSON socket is the smallest
//! thing that fixes both: one request per line, one response line per
//! request, so a test, a shell or the `silica-mcp` server can open a vault,
//! dispatch keystrokes and read the whole window back as JSON. This crate
//! holds the half of that which needs no window — which is why it compiles
//! (and tests) without gpui, and why [`METHODS`] rather than the app is the
//! description of the protocol that the MCP server builds its tool schemas
//! from.
//!
//! ```
//! # use silica_control::Request;
//! let req: Request = serde_json::from_str(r#"{"id":1,"method":"state","params":{"rows":5}}"#)
//!     .expect("a request line");
//! assert_eq!(req.opt_usize("rows"), Some(5));
//! // Bad input is an error string, never a panic: it crossed a socket.
//! assert!(req.str_param("rows").is_err());
//! ```

mod client;
mod methods;
pub mod net;
mod protocol;

/// The socket is a Unix domain socket on every platform ([`net`] says how
/// on Windows), and so is the client.
pub use client::Client;
pub use methods::{
    method, params_schema, protocol_json, MethodSpec, ParamSpec, ParamType, METHODS,
};
pub use protocol::{Request, Response};

/// Wire protocol version, reported by `ping` and `protocol`. Bump it when a
/// method's params or result change shape, not when one is added.
/// 2: `state` gained `status_bar` and `window.title`, `state.modal` gained
/// the Settings tab's rows, and `window`'s result gained `title`.
pub const PROTOCOL: u32 = 2;

/// This crate's version, which is the app's: `ping` reports it so a client can
/// tell which build it is driving.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
