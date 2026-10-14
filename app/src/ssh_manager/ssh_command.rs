use std::time::Duration;

use crate::terminal::ssh::util::{parse_interactive_ssh_command, split_ssh_destination};

pub const SSH_PROFILE_WARPIFY_DISABLED_COMMAND_TTL: Duration = Duration::from_secs(30);

pub fn normalize_ssh_command(command: &str) -> &str {
    let command = command.trim();
    command
        .strip_prefix("command -p ")
        .or_else(|| command.strip_prefix("command "))
        .unwrap_or(command)
        .trim()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshCommandTarget {
    pub user: Option<String>,
    pub host: String,
    pub port: Option<String>,
}

/// Whether two commands run `ssh` to the same destination, even if written differently.
pub fn same_ssh_command(a: &str, b: &str) -> bool {
    normalize_ssh_command(a) == normalize_ssh_command(b)
        || ssh_command_target(a).is_some_and(|target| ssh_command_target(b) == Some(target))
}

pub fn ssh_command_target(command: &str) -> Option<SshCommandTarget> {
    let command = normalize_ssh_command(command);
    let parsed = parse_interactive_ssh_command(command)?;
    let destination = parsed.host?;
    let (_, host) = split_ssh_destination(&destination);
    if host.is_empty() {
        return None;
    }
    Some(SshCommandTarget {
        user: parsed.user,
        host: host.to_string(),
        port: parsed.port,
    })
}

#[cfg(test)]
#[path = "ssh_command_tests.rs"]
mod tests;
