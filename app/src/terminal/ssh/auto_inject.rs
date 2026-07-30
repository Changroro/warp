use std::time::Duration;

use instant::Instant;
use uuid::Uuid;

use crate::terminal::model::block::BlockId;
use crate::terminal::ssh::util::{
    SshLoginPromptKind, parse_interactive_ssh_command, split_ssh_destination,
};

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
mod tests {
    use super::*;
    use crate::terminal::ssh::util::{SshHostProfile, parse_ssh_login_prompt};

    fn state(command: &str) -> SshAutoInjectState {
        SshAutoInjectState::new(Uuid::new_v4(), command.to_string(), BlockId::new())
            .expect("ssh command target should parse")
    }

    fn decide(state: &SshAutoInjectState, output_tail: &str) -> SshAutoInjectDecision {
        let prompt = parse_ssh_login_prompt(output_tail).expect("output tail should be a prompt");
        state.evaluate_login_prompt(&prompt)
    }

    fn injects(state: &SshAutoInjectState, output_tail: &str) -> bool {
        decide(state, output_tail) == SshAutoInjectDecision::Inject
    }

    #[test]
    fn matches_command_handles_command_wrapper_and_double_dash() {
        let state = state("command ssh -p 5022 -- root@118.38.20.101");

        assert!(state.matches_command("ssh -p 5022 root@118.38.20.101"));
    }

    #[test]
    fn matches_command_accepts_the_rendered_profile_command() {
        let mut profile =
            SshHostProfile::new("Production".to_string(), "db.example.com".to_string());
        profile.user = Some("deploy".to_string());
        profile.port = 5022;
        let rendered = profile.to_ssh_command();
        let state = state(&rendered);

        assert!(state.matches_command(&rendered));
        assert!(state.matches_command("ssh -p 5022 deploy@db.example.com"));
        assert!(!state.matches_command("env ssh -p 5022 deploy@db.example.com"));
        assert!(!state.matches_command("ssh -p 5022 other@db.example.com"));
    }

    #[test]
    fn matches_command_rejects_different_ssh_target() {
        let state = state("command ssh -p 5022 -- root@118.38.20.101");

        assert!(!state.matches_command("ssh -p 5022 root@118.38.20.102"));
        assert!(!state.matches_command("ssh -p 22 root@118.38.20.101"));
    }

    #[test]
    fn matches_command_compares_login_user() {
        let state = state("command ssh -l root -- example.test");

        assert!(state.matches_command("ssh -l root example.test"));
        assert!(state.matches_command("ssh root@example.test"));
        assert!(!state.matches_command("ssh -l guest example.test"));
        assert!(!state.matches_command("ssh guest@example.test"));
        assert!(!state.matches_command("ssh example.test"));
    }

    #[test]
    fn matches_command_rejects_commands_without_a_target() {
        let state = state("command ssh -- example.test");

        assert!(!state.matches_command("gcloud compute ssh --zone us-west1-a"));
        assert!(!state.matches_command("ls -la"));
    }

    #[test]
    fn new_rejects_commands_without_a_resolvable_target() {
        assert!(
            SshAutoInjectState::new(Uuid::new_v4(), "ls -la".to_string(), BlockId::new()).is_none()
        );
        assert!(
            SshAutoInjectState::new(
                Uuid::new_v4(),
                "gcloud compute ssh --zone us-west1-a".to_string(),
                BlockId::new(),
            )
            .is_none()
        );
    }

    #[test]
    fn evaluate_login_prompt_accepts_the_armed_destination() {
        let state = state("command ssh -p 5022 -- root@example.test");

        assert!(injects(
            &state,
            "Warning: banner\r\nroot@example.test's password: "
        ));
        assert!(injects(&state, "root@EXAMPLE.TEST's password:"));
        assert!(injects(
            &state,
            "(root@example.test) root@example.test's password: "
        ));
    }

    #[test]
    fn evaluate_login_prompt_checks_pam_prompt_principals() {
        let state = state("command ssh -p 5022 -- root@example.test");

        assert!(injects(&state, "(root@example.test) Password: "));
        assert_eq!(
            decide(&state, "(root@other.test) Password: "),
            SshAutoInjectDecision::Skip(SshAutoInjectSkipReason::TargetMismatch)
        );
        assert_eq!(
            decide(&state, "(guest@example.test) Password:"),
            SshAutoInjectDecision::Skip(SshAutoInjectSkipReason::TargetMismatch)
        );
    }

    #[test]
    fn evaluate_login_prompt_skips_prompts_without_a_host() {
        let state = state("command ssh -p 5022 -- root@example.test");

        for output_tail in ["Password:", "Password: ", "(root) Password: "] {
            assert_eq!(
                decide(&state, output_tail),
                SshAutoInjectDecision::Skip(SshAutoInjectSkipReason::UnverifiableHost)
            );
        }
    }

    #[test]
    fn evaluate_login_prompt_skips_key_passphrase_prompts() {
        let state = state("command ssh -p 5022 -i /home/user/.ssh/id_ed25519 -- root@example.test");

        assert_eq!(
            decide(
                &state,
                "Enter passphrase for key '/home/user/.ssh/id_ed25519': "
            ),
            SshAutoInjectDecision::Skip(SshAutoInjectSkipReason::KeyPassphrasePrompt)
        );
    }

    #[test]
    fn evaluate_login_prompt_rejects_other_hosts_and_users() {
        let state = state("command ssh -p 5022 -- root@example.test");

        for output_tail in [
            "admin@other.test's password: ",
            "guest@example.test's password: ",
            "example.test's password: ",
        ] {
            assert_eq!(
                decide(&state, output_tail),
                SshAutoInjectDecision::Skip(SshAutoInjectSkipReason::TargetMismatch)
            );
        }
    }

    #[test]
    fn evaluate_login_prompt_accepts_any_user_when_the_target_has_none() {
        let state = state("command ssh -- example.test");

        assert!(injects(&state, "bch@example.test's password: "));
        assert_eq!(
            decide(&state, "bch@other.test's password: "),
            SshAutoInjectDecision::Skip(SshAutoInjectSkipReason::TargetMismatch)
        );
    }

    #[test]
    fn password_has_control_chars_detects_injectable_bytes() {
        assert!(!password_has_control_chars("plain-value"));
        assert!(!password_has_control_chars("päßwört"));
        assert!(password_has_control_chars("value\n"));
        assert!(password_has_control_chars("value\r\nrm -rf /"));
        assert!(password_has_control_chars("value\0"));
        assert!(password_has_control_chars("value\ttab"));
    }

    #[test]
    fn state_tracks_a_single_target_block() {
        let block_id = BlockId::new();
        let mut state = state("command ssh -- example.test");

        assert!(!state.has_target_block());

        state.activate(block_id.clone());

        assert_eq!(state.target_block_id(), Some(&block_id));
        assert!(state.has_target_block());
    }
}
