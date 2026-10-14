//! Deciding why an SSH connection ended and whether to reconnect on its own.

use std::time::Duration;

pub const AUTO_RECONNECT_DELAY: Duration = Duration::from_secs(5);
pub const MAX_AUTO_RECONNECT_ATTEMPTS: u32 = 5;
/// A connection that stayed up this long starts its retry count over when it drops.
pub const STABLE_CONNECTION: Duration = Duration::from_secs(30);

/// ssh messages that mean the network or server dropped the connection, as opposed to the user
/// logging out or authentication failing.
const NETWORK_FAILURES: [&str; 11] = [
    "broken pipe",
    "connection reset",
    "connection timed out",
    "operation timed out",
    "closed by remote host",
    "connection refused",
    "no route to host",
    "network is unreachable",
    "could not resolve hostname",
    "client_loop: send disconnect",
    "timeout, server",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisconnectReason {
    /// The ssh output line that explains the disconnect.
    pub message: String,
    pub is_network_failure: bool,
}

/// Finds the last line of ssh output that explains why the connection ended.
pub fn disconnect_reason(output: &str) -> Option<DisconnectReason> {
    output.lines().rev().find_map(|line| {
        let line = line.trim();
        let lower = line.to_lowercase();
        if NETWORK_FAILURES
            .iter()
            .any(|pattern| lower.contains(pattern))
        {
            Some(DisconnectReason {
                message: line.to_string(),
                is_network_failure: true,
            })
        } else if lower.contains("permission denied") || lower.starts_with("connection to ") {
            Some(DisconnectReason {
                message: line.to_string(),
                is_network_failure: false,
            })
        } else {
            None
        }
    })
}

#[cfg(test)]
#[path = "reconnect_tests.rs"]
mod tests;
