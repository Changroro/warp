use serde::{Deserialize, Serialize};
use uuid::Uuid;

const SSH_COMMAND_LAUNCHER: [&str; 2] = ["command", "ssh"];

fn new_profile_id() -> Uuid {
    Uuid::new_v4()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[schemars(description = "A saved SSH jump host configuration.")]
pub struct SshJumpHost {
    #[serde(default)]
    pub source_profile_id: Option<Uuid>,
    pub host: String,
    pub user: Option<String>,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    pub identity_file: Option<String>,
}

impl settings_value::SettingsValue for SshJumpHost {}

impl SshJumpHost {
    pub fn target(&self) -> String {
        ssh_target(&self.user, &self.host, self.port, true)
    }

    fn destination(&self) -> String {
        ssh_target(&self.user, &self.host, self.port, false)
    }

    fn host_port(&self) -> String {
        host_with_port(&self.host, self.port)
    }

    fn push_proxy_command_options(&self, argv: &mut Vec<String>) {
        if let Some(identity_file) = self
            .identity_file
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            argv.push("-i".to_string());
            argv.push(escape_openssh_percent(identity_file));
        }

        if self.port != default_ssh_port() {
            argv.push("-p".to_string());
            argv.push(self.port.to_string());
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[schemars(description = "A saved SSH host profile.")]
pub struct SshHostProfile {
    #[serde(default = "new_profile_id")]
    pub id: Uuid,
    pub name: String,
    pub host: String,
    pub user: Option<String>,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    pub identity_file: Option<String>,
    #[serde(default)]
    pub jump_hosts: Vec<SshJumpHost>,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl settings_value::SettingsValue for SshHostProfile {}

impl SshHostProfile {
    pub fn new(name: String, host: String) -> Self {
        Self {
            id: new_profile_id(),
            name,
            host,
            user: None,
            port: default_ssh_port(),
            identity_file: None,
            jump_hosts: Vec::new(),
            tags: Vec::new(),
        }
    }

    pub fn target(&self) -> String {
        ssh_target(&self.user, &self.host, self.port, false)
    }

    pub fn to_ssh_command(&self) -> String {
        let mut argv = SSH_COMMAND_LAUNCHER
            .iter()
            .map(|arg| (*arg).to_string())
            .collect::<Vec<_>>();

        if let Some(identity_file) = self
            .identity_file
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            argv.push("-i".to_string());
            argv.push(identity_file.to_string());
        }

        if self.port != default_ssh_port() {
            argv.push("-p".to_string());
            argv.push(self.port.to_string());
        }

        if !self.jump_hosts.is_empty() {
            if self.jump_hosts.iter().any(|jump_host| {
                jump_host
                    .identity_file
                    .as_deref()
                    .is_some_and(|value| !value.is_empty())
            }) {
                argv.push("-o".to_string());
                argv.push(format!(
                    "ProxyCommand={}",
                    proxy_command_for_jump_hosts(
                        &self.jump_hosts,
                        &host_with_port(&self.host, self.port)
                    )
                ));
            } else {
                argv.push("-J".to_string());
                argv.push(
                    self.jump_hosts
                        .iter()
                        .map(SshJumpHost::target)
                        .collect::<Vec<_>>()
                        .join(","),
                );
            }
        }

        argv.push("--".to_string());
        argv.push(self.target());
        argv.into_iter()
            .map(|arg| shell_quote_arg(&arg))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn proxy_command_for_jump_hosts(jump_hosts: &[SshJumpHost], destination: &str) -> String {
    let mut proxy_command: Option<String> = None;
    for (index, jump_host) in jump_hosts.iter().enumerate() {
        let next_destination = jump_hosts
            .get(index + 1)
            .map(SshJumpHost::host_port)
            .unwrap_or_else(|| destination.to_string());

        let mut argv = vec!["ssh".to_string()];
        if let Some(previous_proxy_command) = proxy_command.take() {
            argv.push("-o".to_string());
            argv.push(format!(
                "ProxyCommand={}",
                escape_openssh_percent(&previous_proxy_command)
            ));
        }
        jump_host.push_proxy_command_options(&mut argv);
        argv.push("-W".to_string());
        argv.push(escape_openssh_percent(&next_destination));
        argv.push("--".to_string());
        argv.push(escape_openssh_percent(&jump_host.destination()));

        proxy_command = Some(
            argv.into_iter()
                .map(|arg| shell_quote_arg(&arg))
                .collect::<Vec<_>>()
                .join(" "),
        );
    }

    proxy_command.unwrap_or_default()
}

pub fn default_ssh_port() -> u16 {
    22
}

/// A profile id replaced by [`normalize_ssh_host_profiles`]; secrets stored under `previous_id` no
/// longer resolve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SshProfileIdReassignment {
    pub previous_id: Uuid,
    pub new_id: Uuid,
}

/// Replaces nil and duplicate profile ids with fresh ones and normalizes zero ports, returning
/// every id reassignment so callers can migrate or discard secrets keyed by the previous id.
#[must_use]
pub fn normalize_ssh_host_profiles(
    profiles: &mut [SshHostProfile],
) -> Vec<SshProfileIdReassignment> {
    let mut seen = std::collections::HashSet::new();
    let mut reassignments = Vec::new();
    for profile in profiles {
        if profile.id.is_nil() || !seen.insert(profile.id) {
            let previous_id = profile.id;
            profile.id = Uuid::new_v4();
            seen.insert(profile.id);
            reassignments.push(SshProfileIdReassignment {
                previous_id,
                new_id: profile.id,
            });
        }
        if profile.port == 0 {
            profile.port = default_ssh_port();
        }
        for jump_host in &mut profile.jump_hosts {
            if jump_host.port == 0 {
                jump_host.port = default_ssh_port();
            }
        }
    }
    reassignments
}

pub fn ssh_profile_password_key(id: Uuid) -> String {
    format!("ssh-profile-password:{id}")
}

pub fn shell_quote_arg(arg: &str) -> String {
    shell_words::quote(arg).into_owned()
}

/// Renders a `host[:port]` destination in the bracketed form OpenSSH requires for IPv6 literals.
fn host_with_port(host: &str, port: u16) -> String {
    if is_ipv6_literal(host) {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn is_ipv6_literal(host: &str) -> bool {
    host.contains(':') && !host.starts_with('[')
}

/// Escapes one level of OpenSSH `%` token expansion so the value reaches the next `ssh` literally.
fn escape_openssh_percent(value: &str) -> String {
    value.replace('%', "%%")
}

fn ssh_target(user: &Option<String>, host: &str, port: u16, include_port: bool) -> String {
    let mut target = String::new();
    if let Some(user) = user.as_deref().filter(|value| !value.is_empty()) {
        target.push_str(user);
        target.push('@');
    }
    if include_port && is_ipv6_literal(host) {
        target.push('[');
        target.push_str(host);
        target.push(']');
    } else {
        target.push_str(host);
    }
    if include_port && port != default_ssh_port() {
        target.push(':');
        target.push_str(&port.to_string());
    }
    target
}

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
