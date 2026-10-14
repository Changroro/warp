use std::time::Duration;

use instant::Instant;
use uuid::Uuid;

use crate::ssh_manager::login_prompt::SshLoginPromptKind;
use crate::terminal::model::block::BlockId;
use crate::terminal::ssh::util::{parse_interactive_ssh_command, split_ssh_destination};

pub const SSH_AUTO_INJECT_POLL_INTERVAL: Duration = Duration::from_millis(250);
pub const SSH_AUTO_INJECT_ARM_TTL: Duration = Duration::from_secs(30);
pub const SSH_PROFILE_WARPIFY_DISABLED_COMMAND_TTL: Duration = Duration::from_secs(30);
pub const SSH_PASSWORD_PROMPT_SCAN_CHARS: usize = 1024;

pub fn normalize_ssh_command(command: &str) -> &str {
    let command = command.trim();
    command
        .strip_prefix("command -p ")
        .or_else(|| command.strip_prefix("command "))
        .unwrap_or(command)
        .trim()
}

pub fn password_has_control_chars(password: &str) -> bool {
    password.chars().any(char::is_control)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshCommandTarget {
    pub user: Option<String>,
    pub host: String,
    pub port: Option<String>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SshAutoInjectSkipReason {
    UnverifiableHost,
    TargetMismatch,
    KeyPassphrasePrompt,
}

impl std::fmt::Display for SshAutoInjectSkipReason {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reason = match self {
            Self::UnverifiableHost => "the password prompt does not identify a host",
            Self::TargetMismatch => "the password prompt does not belong to the profile target",
            Self::KeyPassphrasePrompt => {
                "the prompt asks for a key passphrase instead of the account password"
            }
        };
        formatter.write_str(reason)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SshAutoInjectDecision {
    Inject,
    Skip(SshAutoInjectSkipReason),
}

#[derive(Clone, Debug)]
pub struct SshAutoInjectState {
    profile_id: Uuid,
    command: String,
    target: SshCommandTarget,
    armed_block_id: BlockId,
    target_block_id: Option<BlockId>,
    expires_at: Instant,
}

impl SshAutoInjectState {
    pub fn new(profile_id: Uuid, command: String, armed_block_id: BlockId) -> Option<Self> {
        let target = ssh_command_target(&command)?;
        Some(Self {
            profile_id,
            command,
            target,
            armed_block_id,
            target_block_id: None,
            expires_at: Instant::now() + SSH_AUTO_INJECT_ARM_TTL,
        })
    }

    pub fn profile_id(&self) -> Uuid {
        self.profile_id
    }

    pub fn matches_command(&self, command: &str) -> bool {
        if normalize_ssh_command(&self.command) == normalize_ssh_command(command) {
            return true;
        }
        ssh_command_target(command).is_some_and(|actual| actual == self.target)
    }

    pub fn evaluate_login_prompt(&self, prompt: &SshLoginPromptKind) -> SshAutoInjectDecision {
        let principal = match prompt {
            SshLoginPromptKind::KeyPassphrase { .. } => {
                return SshAutoInjectDecision::Skip(SshAutoInjectSkipReason::KeyPassphrasePrompt);
            }
            SshLoginPromptKind::HostPassword { principal: None } => {
                return SshAutoInjectDecision::Skip(SshAutoInjectSkipReason::UnverifiableHost);
            }
            SshLoginPromptKind::HostPassword {
                principal: Some(principal),
            } => principal,
        };

        if !principal.host.eq_ignore_ascii_case(&self.target.host) {
            return SshAutoInjectDecision::Skip(SshAutoInjectSkipReason::TargetMismatch);
        }
        match (principal.user.as_deref(), self.target.user.as_deref()) {
            (Some(prompt_user), Some(target_user)) if prompt_user != target_user => {
                SshAutoInjectDecision::Skip(SshAutoInjectSkipReason::TargetMismatch)
            }
            (None, Some(_)) => SshAutoInjectDecision::Skip(SshAutoInjectSkipReason::TargetMismatch),
            _ => SshAutoInjectDecision::Inject,
        }
    }

    pub fn activate(&mut self, block_id: BlockId) {
        self.target_block_id = Some(block_id);
        self.expires_at = Instant::now() + SSH_AUTO_INJECT_ARM_TTL;
    }

    pub fn armed_block_id(&self) -> &BlockId {
        &self.armed_block_id
    }

    pub fn target_block_id(&self) -> Option<&BlockId> {
        self.target_block_id.as_ref()
    }

    pub fn has_target_block(&self) -> bool {
        self.target_block_id.is_some()
    }

    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }
}

#[cfg(test)]
#[path = "auto_inject_tests.rs"]
mod tests;
