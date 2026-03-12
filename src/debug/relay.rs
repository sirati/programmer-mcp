//! Debug-specific relay: wraps the shared RelayChannel for child process stdio.

use tokio::process::{ChildStdin, ChildStdout};

pub use crate::relay::build_jsonrpc_request;

pub type RelayChannel = crate::relay::RelayChannel<ChildStdin, ChildStdout>;
