use uuid::Uuid;
use warpui::ViewContext;
use warpui::r#async::Timer;
use warpui_extras::secure_storage::AppContextExt as _;
use zeroize::Zeroizing;

use super::TerminalView;
use crate::ssh_manager::auto_inject::{
    SSH_AUTO_INJECT_POLL_INTERVAL, SSH_PASSWORD_PROMPT_SCAN_CHARS,
    SSH_PROFILE_WARPIFY_DISABLED_COMMAND_TTL, SshAutoInjectDecision, SshAutoInjectState,
    normalize_ssh_command, password_has_control_chars,
};
use crate::ssh_manager::login_prompt::parse_ssh_login_prompt;
use crate::ssh_manager::profile::ssh_profile_password_key;
use crate::terminal::model::block::BlockId;
use crate::terminal::model::escape_sequences;
use crate::terminal::ssh::util::{SshLoginState, check_ssh_login_state};

/// Per-terminal state for a connection started from a saved SSH profile.
#[derive(Default)]
pub(super) struct SshProfileSessionState {
    auto_inject: Option<SshAutoInjectState>,
    warpification_enabled_at_arm: bool,
    warpify_disabled_command: Option<String>,
}

impl SshProfileSessionState {
    pub(super) fn clear_auto_inject(&mut self) {
        self.auto_inject = None;
    }
}

fn last_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars().rev().take(max_chars).collect::<Vec<_>>();
    chars.reverse();
    chars.into_iter().collect()
}

impl TerminalView {
    /// Returns whether `command` belongs to an SSH profile connection that has Warpification
    /// disabled, and if so suppresses the remote shell bootstrap until the command finishes.
    pub(super) fn suppress_ssh_profile_bootstrap_if_warpify_disabled(
        &mut self,
        command: &str,
        warpify_command: &str,
    ) -> bool {
        let suppress = !self.ssh_profile.warpification_enabled_at_arm
            && (self.ssh_profile_warpify_disabled_command_matches(command)
                || self.ssh_profile_warpify_disabled_command_matches(warpify_command)
                || self.ssh_profile.auto_inject.as_ref().is_some_and(|state| {
                    state.matches_command(command) || state.matches_command(warpify_command)
                }));
        if suppress {
            self.model
                .lock()
                .ignore_bootstrapping_messages_until_command_finished();
            log::info!(
                "Suppressing SSH profile shell bootstrap because SSH Warpification is disabled"
            );
        }
        suppress
    }

    pub fn arm_ssh_profile_auto_inject(
        &mut self,
        profile_id: Uuid,
        command: String,
        ssh_warpification_enabled: bool,
        enable_password_auto_inject: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.ssh_profile.auto_inject = if enable_password_auto_inject {
            let armed_block_id = self.model.lock().block_list().active_block_id().clone();
            let state = SshAutoInjectState::new(profile_id, command.clone(), armed_block_id);
            if state.is_none() {
                log::warn!(
                    "Not arming SSH profile password auto-inject: no SSH target could be parsed from the profile command for profile_id={profile_id}"
                );
            }
            state
        } else {
            None
        };
        self.ssh_profile.warpification_enabled_at_arm = ssh_warpification_enabled;
        self.ssh_profile.warpify_disabled_command = (!ssh_warpification_enabled).then_some(command);
        log::info!(
            "Armed SSH profile connection: profile_id={profile_id}, ssh_warpification_enabled={ssh_warpification_enabled}, enable_password_auto_inject={enable_password_auto_inject}"
        );
        if !ssh_warpification_enabled {
            let command_to_clear = self.ssh_profile.warpify_disabled_command.clone();
            ctx.spawn(
                async {
                    Timer::after(SSH_PROFILE_WARPIFY_DISABLED_COMMAND_TTL).await;
                },
                move |terminal_view, _, _| {
                    if terminal_view.ssh_profile.warpify_disabled_command == command_to_clear {
                        terminal_view.ssh_profile.warpify_disabled_command = None;
                        terminal_view.ssh_profile.warpification_enabled_at_arm = false;
                    }
                },
            );
        }
        if self.ssh_profile.auto_inject.is_some() {
            self.schedule_ssh_profile_auto_inject_poll(ctx);
        }
    }

    fn ssh_profile_warpify_disabled_command_matches(&self, command: &str) -> bool {
        self.ssh_profile
            .warpify_disabled_command
            .as_deref()
            .is_some_and(|profile_command| {
                normalize_ssh_command(profile_command) == normalize_ssh_command(command)
            })
    }

    pub(super) fn maybe_activate_ssh_profile_auto_inject(
        &mut self,
        command: &str,
        block_id: BlockId,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(mut state) = self.ssh_profile.auto_inject.take() else {
            return;
        };
        if state.is_expired() {
            self.disarm_ssh_profile_auto_inject();
            return;
        }
        if state.has_target_block() {
            self.ssh_profile.auto_inject = Some(state);
            return;
        }
        if !state.matches_command(command) {
            if state.armed_block_id() == &block_id {
                self.ssh_profile.auto_inject = Some(state);
            } else {
                log::info!(
                    "Disarming SSH profile password auto-inject: another command started for profile_id={}",
                    state.profile_id()
                );
                self.disarm_ssh_profile_auto_inject();
            }
            return;
        }

        state.activate(block_id);
        self.ssh_profile.auto_inject = Some(state);
        self.schedule_ssh_profile_auto_inject_poll(ctx);
    }

    fn disarm_ssh_profile_auto_inject(&mut self) {
        self.ssh_profile = SshProfileSessionState::default();
    }

    fn schedule_ssh_profile_auto_inject_poll(&mut self, ctx: &mut ViewContext<Self>) {
        if self.ssh_profile.auto_inject.is_none() {
            return;
        }
        ctx.spawn(
            async {
                Timer::after(SSH_AUTO_INJECT_POLL_INTERVAL).await;
            },
            |terminal_view, _, ctx| {
                if !terminal_view.poll_ssh_profile_auto_inject(ctx) {
                    terminal_view.schedule_ssh_profile_auto_inject_poll(ctx);
                }
            },
        );
    }

    fn poll_ssh_profile_auto_inject(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let Some(mut state) = self.ssh_profile.auto_inject.take() else {
            return true;
        };

        if state.is_expired() {
            self.disarm_ssh_profile_auto_inject();
            return true;
        }

        let profile_id = state.profile_id();
        let (active_block_id, active_command, output_tail) = {
            let model = self.model.lock();
            let active_block = model.block_list().active_block();
            (
                active_block.id().clone(),
                active_block.command_to_string(),
                last_chars(
                    &active_block.output_to_string(),
                    SSH_PASSWORD_PROMPT_SCAN_CHARS,
                ),
            )
        };

        let target_block_id = if let Some(target_block_id) = state.target_block_id().cloned() {
            target_block_id
        } else if state.matches_command(&active_command) {
            state.activate(active_block_id.clone());
            active_block_id.clone()
        } else if state.armed_block_id() == &active_block_id {
            self.ssh_profile.auto_inject = Some(state);
            return false;
        } else {
            log::info!(
                "Disarming SSH profile password auto-inject: another command started for profile_id={profile_id}"
            );
            self.disarm_ssh_profile_auto_inject();
            return true;
        };

        if active_block_id != target_block_id {
            self.disarm_ssh_profile_auto_inject();
            return true;
        }

        if matches!(
            check_ssh_login_state(&output_tail),
            SshLoginState::LastLogin | SshLoginState::PromptDetected
        ) {
            log::info!(
                "Disarming SSH profile password auto-inject: the session left authentication for profile_id={profile_id}"
            );
            self.disarm_ssh_profile_auto_inject();
            return true;
        }

        let Some(prompt) = parse_ssh_login_prompt(&output_tail) else {
            self.ssh_profile.auto_inject = Some(state);
            return false;
        };

        if let SshAutoInjectDecision::Skip(reason) = state.evaluate_login_prompt(&prompt) {
            log::warn!(
                "Disarming SSH profile password auto-inject: {reason} for profile_id={profile_id}"
            );
            self.disarm_ssh_profile_auto_inject();
            return true;
        }

        let key = ssh_profile_password_key(profile_id);
        let password = match ctx.secure_storage().read_value(&key) {
            Ok(password) => Zeroizing::new(password),
            Err(err) => {
                log::warn!(
                    "Skipping SSH profile password auto-inject: failed to read the stored password for profile_id={profile_id}: {err:#}"
                );
                self.disarm_ssh_profile_auto_inject();
                return true;
            }
        };

        if password.is_empty() {
            log::warn!(
                "Skipping SSH profile password auto-inject: the stored password is empty for profile_id={profile_id}"
            );
            self.disarm_ssh_profile_auto_inject();
            return true;
        }
        if password_has_control_chars(&password) {
            log::warn!(
                "Skipping SSH profile password auto-inject: the stored password contains control characters for profile_id={profile_id}"
            );
            self.disarm_ssh_profile_auto_inject();
            return true;
        }

        let mut password_bytes = Vec::with_capacity(password.len() + 1);
        password_bytes.extend_from_slice(password.as_bytes());
        password_bytes.push(escape_sequences::C0::CR);
        drop(password);
        self.write_to_pty(password_bytes, ctx);
        true
    }
}

#[cfg(test)]
#[path = "ssh_manager_tests.rs"]
mod tests;
