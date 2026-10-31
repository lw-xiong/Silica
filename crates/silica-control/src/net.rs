//! The socket the protocol runs over: a Unix domain socket on every
//! platform.
//!
//! Windows has had `AF_UNIX` since Windows 10 1803, but the standard library
//! only surfaces it under `std::os::unix`, so there the two types come from
//! `uds_windows`, the same API over WinSock. What is the same everywhere: the
//! socket is a path, `bind` refuses one that already exists (a stale file has
//! to be unlinked first), `connect` to a file nobody listens on is refused,
//! and `sun_path` holds at most 108 bytes, so the path has to be short. What
//! is not: Windows has no mode bits to restrict the socket with, so there its
//! folder's ACL is the whole protection — the user's own temp directory, in
//! `silica-mcp`'s case.

#[cfg(unix)]
pub use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(windows)]
pub use uds_windows::{UnixListener, UnixStream};
