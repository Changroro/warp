use std::time::Duration;

use instant::Instant;
use uuid::Uuid;

use crate::terminal::model::block::BlockId;
use crate::terminal::ssh::util::parse_interactive_ssh_command;

pub const SSH_AUTO_INJECT_POLL_INTERVAL: Duration = Duration::from_millis(250);
pub const SSH_AUTO_INJECT_TTL: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
pub struct SshAutoInjectState {
    profile_id: Uuid,
    command: String,
    target_block_id: Option<BlockId>,
    attempted: bool,
    login_completed: bool,
    expires_at: Instant,
}

impl SshAutoInjectState {
    pub fn new(profile_id: Uuid, command: String) -> Self {
        Self {
            profile_id,
            command,
            target_block_id: None,
            attempted: false,
            login_completed: false,
            expires_at: Instant::now() + SSH_AUTO_INJECT_TTL,
        }
    }

    pub fn profile_id(&self) -> Uuid {
        self.profile_id
    }

    pub fn matches_command(&self, command: &str) -> bool {
        if self.command.trim() == command.trim() {
            return true;
        }

        let expected = parse_interactive_ssh_command(&self.command);
        let actual = parse_interactive_ssh_command(command);
        matches!(
            (expected, actual),
            (Some(expected), Some(actual))
                if expected.host == actual.host && expected.port == actual.port
        )
    }

    pub fn activate(&mut self, block_id: BlockId) {
        self.target_block_id = Some(block_id);
        self.expires_at = Instant::now() + SSH_AUTO_INJECT_TTL;
    }

    pub fn target_block_id(&self) -> Option<&BlockId> {
        self.target_block_id.as_ref()
    }

    pub fn has_target_block(&self) -> bool {
        self.target_block_id.is_some()
    }

    pub fn attempted(&self) -> bool {
        self.attempted
    }

    pub fn mark_attempted(&mut self) {
        self.attempted = true;
    }

    pub fn mark_login_completed(&mut self) {
        self.login_completed = true;
    }

    pub fn login_completed(&self) -> bool {
        self.login_completed
    }

    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_command_handles_command_wrapper_and_double_dash() {
        let profile_id = Uuid::new_v4();
        let state = SshAutoInjectState::new(
            profile_id,
            "command ssh -p 5022 -- root@118.38.20.101".to_string(),
        );

        assert!(state.matches_command("ssh -p 5022 root@118.38.20.101"));
    }

    #[test]
    fn matches_command_rejects_different_ssh_target() {
        let profile_id = Uuid::new_v4();
        let state = SshAutoInjectState::new(
            profile_id,
            "command ssh -p 5022 -- root@118.38.20.101".to_string(),
        );

        assert!(!state.matches_command("ssh -p 5022 root@118.38.20.102"));
        assert!(!state.matches_command("ssh -p 22 root@118.38.20.101"));
    }

    #[test]
    fn state_tracks_single_target_block_and_attempt() {
        let profile_id = Uuid::new_v4();
        let block_id = BlockId::new();
        let mut state = SshAutoInjectState::new(profile_id, "command ssh host".to_string());

        assert!(!state.has_target_block());
        assert!(!state.attempted());
        assert!(!state.login_completed());

        state.activate(block_id.clone());
        state.mark_attempted();
        state.mark_login_completed();

        assert_eq!(state.target_block_id(), Some(&block_id));
        assert!(state.has_target_block());
        assert!(state.attempted());
        assert!(state.login_completed());
    }
}
