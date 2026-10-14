use std::path::PathBuf;
use std::time::Duration;

use chrono::Local;
use instant::Instant;
use settings::Setting as _;
use uuid::Uuid;
use warp_core::ui::theme::AnsiColorIdentifier;
use warp_core::ui::theme::color::internal_colors;
use warpui::r#async::Timer;
use warpui::clipboard::ClipboardContent;
use warpui::elements::{
    Border, Container, CrossAxisAlignment, Expanded, Flex, MainAxisSize, ParentElement, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::text_layout::ClipConfig;
use warpui::{AppContext, Element, SingletonEntity as _, ViewContext, ViewHandle};
use warpui_extras::secure_storage::AppContextExt as _;
use zeroize::Zeroizing;

use super::{
    BRACKETED_PASTE_PREFIX, BRACKETED_PASTE_SUFFIX, RichContentInsertionPosition, TerminalView,
};
use crate::appearance::Appearance;
use crate::settings::SshSettings;
use crate::ssh_manager::auto_inject::{
    SSH_AUTO_INJECT_POLL_INTERVAL, SSH_PASSWORD_PROMPT_SCAN_CHARS,
    SSH_PROFILE_WARPIFY_DISABLED_COMMAND_TTL, SshAutoInjectDecision, SshAutoInjectState,
    normalize_ssh_command, password_has_control_chars,
};
use crate::ssh_manager::control_master::{
    MAX_UPLOAD_BYTES, new_control_path, remote_image_path, upload, with_control_master,
};
use crate::ssh_manager::login_prompt::{
    only_ssh_client_output_before_prompt, parse_ssh_login_prompt,
};
use crate::ssh_manager::open_connections::SshOpenConnections;
use crate::ssh_manager::profile::{SshHostProfile, ssh_profile_password_key};
use crate::ssh_manager::reconnect::{
    AUTO_RECONNECT_DELAY, MAX_AUTO_RECONNECT_ATTEMPTS, STABLE_CONNECTION, disconnect_reason,
};
use crate::ssh_manager::reconnect_banner::{
    AutoReconnectCountdown, SshReconnectBanner, SshReconnectBannerEvent,
};
use crate::ssh_manager::ui::{
    ansi_color, badge_label, host_color, render_badge, render_chip, render_dot,
};
use crate::terminal::model::block::{BlockId, BlockMetadata};
use crate::terminal::model::escape_sequences;
use crate::terminal::model::session::SessionId;
use crate::terminal::ssh::util::{SshLoginState, check_ssh_login_state};
use crate::terminal::warpify::settings::WarpifySettings;

/// Per-terminal state for a connection started from the SSH manager.
#[derive(Default)]
pub(super) struct SshProfileSessionState {
    auto_inject: Option<SshAutoInjectState>,
    warpification_enabled_at_arm: bool,
    warpify_disabled_command: Option<String>,
    connection: Option<SshConnectionWatch>,
    reconnect_banner: Option<ViewHandle<SshReconnectBanner>>,
}

impl SshProfileSessionState {
    pub(super) fn clear_auto_inject(&mut self) {
        self.auto_inject = None;
    }
}

/// The SSH command this terminal ran from the SSH manager, kept so it can be rerun after the
/// connection ends.
struct SshConnectionWatch {
    command: String,
    profile_id: Option<Uuid>,
    /// Shown to the user: the profile name, or the quick-connect arguments.
    target: String,
    control_path: Option<PathBuf>,
    /// The local shell's session while the command runs. A precmd from it means ssh has exited,
    /// whether or not the remote shell was Warpified in between.
    local_session_id: Option<SessionId>,
    started_at: Option<Instant>,
    disconnected: bool,
    /// Automatic reconnects since the connection was last stable.
    attempt: u32,
    /// Bumped to cancel a pending automatic reconnect.
    reconnect_generation: u64,
}

impl SshConnectionWatch {
    fn new(
        command: String,
        profile_id: Option<Uuid>,
        target: String,
        control_path: Option<PathBuf>,
    ) -> Self {
        Self {
            command,
            profile_id,
            target,
            control_path,
            local_session_id: None,
            started_at: None,
            disconnected: false,
            attempt: 0,
            reconnect_generation: 0,
        }
    }
}

/// How many characters of recent output to scan for ssh's disconnect message.
const DISCONNECT_SCAN_CHARS: usize = 600;
const SESSION_BAR_HEIGHT: f32 = 34.;

fn command_with_control_master(command: String, control_path: Option<&PathBuf>) -> String {
    match control_path {
        Some(control_path) => with_control_master(&command, control_path),
        None => command,
    }
}

fn last_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars().rev().take(max_chars).collect::<Vec<_>>();
    chars.reverse();
    chars.into_iter().collect()
}

impl TerminalView {
    /// Handles a command starting. Returns whether `command` belongs to an SSH profile connection
    /// that has Warpification disabled, and if so suppresses the remote shell bootstrap until the
    /// command finishes.
    pub(super) fn handle_ssh_manager_block_started(
        &mut self,
        command: &str,
        warpify_command: &str,
    ) -> bool {
        let local_session_id = self.active_block_session_id();
        if let Some(connection) = self.ssh_profile.connection.as_mut()
            && [command, warpify_command].into_iter().any(|command| {
                normalize_ssh_command(command) == normalize_ssh_command(&connection.command)
            })
        {
            connection.local_session_id = local_session_id;
            connection.started_at = Some(Instant::now());
            connection.disconnected = false;
            connection.reconnect_generation += 1;
            self.remove_ssh_reconnect_banner();
        } else if let Some(connection) = self.ssh_profile.connection.as_mut()
            && connection.disconnected
        {
            // The user moved on in the local shell, so don't rerun ssh under them.
            connection.reconnect_generation += 1;
        }

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

    /// Runs `profile`'s SSH command in this terminal.
    pub fn connect_ssh_profile(&mut self, profile: &SshHostProfile, ctx: &mut ViewContext<Self>) {
        let enable_ssh_warpification = *WarpifySettings::as_ref(ctx)
            .enable_ssh_warpification
            .value();
        let control_path = new_control_path();
        let command = command_with_control_master(profile.to_ssh_command(), control_path.as_ref());
        // A jump host's prompt can arrive first, so auto-inject would send the final password to it.
        let enable_password_auto_inject = profile.jump_hosts.is_empty();
        log::info!(
            "Connecting SSH profile: profile_id={}, enable_ssh_warpification={enable_ssh_warpification}",
            profile.id
        );
        self.arm_ssh_profile_auto_inject(
            profile.id,
            command.clone(),
            enable_ssh_warpification,
            enable_password_auto_inject,
            ctx,
        );
        self.ssh_profile.connection = Some(SshConnectionWatch::new(
            command.clone(),
            Some(profile.id),
            profile.name.clone(),
            control_path,
        ));
        SshOpenConnections::connected(ctx.handle(), Some(profile.id), ctx);
        self.execute_command_or_set_pending(&command, ctx);
    }

    /// Runs `ssh` with the given arguments in this terminal.
    pub fn connect_ssh_destination(&mut self, args: String, ctx: &mut ViewContext<Self>) {
        let control_path = new_control_path();
        let command =
            command_with_control_master(format!("command ssh {args}"), control_path.as_ref());
        self.ssh_profile.connection = Some(SshConnectionWatch::new(
            command.clone(),
            None,
            args,
            control_path,
        ));
        SshOpenConnections::connected(ctx.handle(), None, ctx);
        self.execute_command_or_set_pending(&command, ctx);
    }

    /// Sends a clipboard image to the remote host of a connected SSH manager session and pastes its
    /// remote path, since programs on the remote host can't read the local clipboard. Returns
    /// whether the paste was handled.
    pub(super) fn paste_image_over_ssh(
        &mut self,
        clipboard_content: &ClipboardContent,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let Some(control_path) = self
            .ssh_profile
            .connection
            .as_ref()
            .filter(|connection| connection.local_session_id.is_some())
            .and_then(|connection| connection.control_path.clone())
        else {
            return false;
        };
        // Text copied from apps like spreadsheets often carries an image rendition too.
        if !clipboard_content.plain_text.is_empty() || clipboard_content.paths.is_some() {
            return false;
        }
        let Some(image) = clipboard_content
            .images
            .as_ref()
            .and_then(|images| images.first())
        else {
            return false;
        };
        if image.data.len() > MAX_UPLOAD_BYTES {
            self.show_error_toast(
                format!(
                    "The image is too large to send over SSH (over {} MB).",
                    MAX_UPLOAD_BYTES / (1024 * 1024)
                ),
                ctx,
            );
            return true;
        }

        let remote_path = remote_image_path(&image.mime_type);
        let data = image.data.clone();
        let path_to_paste = remote_path.clone();
        let uploaded_through = control_path.clone();
        ctx.spawn(
            upload(control_path, data, remote_path),
            move |me, result, ctx| match result {
                Ok(())
                    if !me
                        .ssh_profile
                        .connection
                        .as_ref()
                        .is_some_and(|connection| {
                            connection.local_session_id.is_some()
                                && connection.control_path.as_ref() == Some(&uploaded_through)
                        }) =>
                {
                    log::info!("Not pasting the uploaded image path: the SSH connection ended");
                }
                Ok(()) => {
                    let text = if me.model.lock().needs_bracketed_paste() {
                        format!("{BRACKETED_PASTE_PREFIX}{path_to_paste}{BRACKETED_PASTE_SUFFIX}")
                    } else {
                        path_to_paste
                    };
                    me.write_user_bytes_to_pty(text.into_bytes(), ctx);
                }
                Err(err) => {
                    log::warn!("Failed to send a pasted image over SSH: {err:#}");
                    me.show_error_toast(format!("Couldn't send the image over SSH: {err:#}"), ctx);
                }
            },
        );
        true
    }

    /// Shows the reconnect banner once the local shell reports a prompt after a watched SSH
    /// command started, and schedules an automatic reconnect when the network dropped.
    pub(super) fn handle_ssh_manager_precmd(
        &mut self,
        block_metadata: &BlockMetadata,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(connection) = self.ssh_profile.connection.as_ref() else {
            return;
        };
        if connection.local_session_id.is_none()
            || connection.local_session_id != block_metadata.session_id()
        {
            return;
        }

        let recent_output = {
            let model = self.model.lock();
            let blocks = model.block_list().blocks();
            blocks
                .iter()
                .rev()
                .take(3)
                .rev()
                .map(|block| last_chars(&block.output_to_string(), DISCONNECT_SCAN_CHARS))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let reason = disconnect_reason(&recent_output);
        let is_network_failure = reason
            .as_ref()
            .is_some_and(|reason| reason.is_network_failure);

        let Some(connection) = self.ssh_profile.connection.as_mut() else {
            return;
        };
        connection.local_session_id = None;
        connection.disconnected = true;
        if connection
            .started_at
            .is_some_and(|started_at| started_at.elapsed() >= STABLE_CONNECTION)
        {
            connection.attempt = 0;
        }
        let auto_reconnect = is_network_failure && connection.attempt < MAX_AUTO_RECONNECT_ATTEMPTS;
        if auto_reconnect {
            connection.attempt += 1;
        }
        connection.reconnect_generation += 1;
        let generation = connection.reconnect_generation;
        let attempt = connection.attempt;
        let target = connection.target.clone();

        SshOpenConnections::disconnected(ctx.view_id(), ctx);
        self.show_ssh_reconnect_banner(
            target,
            reason.map(|reason| reason.message),
            is_network_failure,
            ctx,
        );
        if auto_reconnect {
            self.tick_ssh_auto_reconnect(
                generation,
                attempt,
                Instant::now() + AUTO_RECONNECT_DELAY,
                ctx,
            );
        }
        ctx.notify();
    }

    fn tick_ssh_auto_reconnect(
        &mut self,
        generation: u64,
        attempt: u32,
        deadline: Instant,
        ctx: &mut ViewContext<Self>,
    ) {
        let still_pending = self
            .ssh_profile
            .connection
            .as_ref()
            .is_some_and(|connection| connection.reconnect_generation == generation);
        let Some(banner) = self.ssh_profile.reconnect_banner.clone() else {
            return;
        };
        if !still_pending {
            banner.update(ctx, |banner, ctx| banner.set_countdown(None, ctx));
            return;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            self.reconnect_ssh(false, ctx);
            return;
        }
        banner.update(ctx, |banner, ctx| {
            banner.set_countdown(
                Some(AutoReconnectCountdown {
                    seconds_left: remaining.as_secs_f32().ceil() as u64,
                    attempt,
                }),
                ctx,
            )
        });
        let wait = remaining.min(Duration::from_secs(1));
        ctx.spawn(
            async move {
                Timer::after(wait).await;
            },
            move |me, _, ctx| me.tick_ssh_auto_reconnect(generation, attempt, deadline, ctx),
        );
    }

    fn cancel_ssh_auto_reconnect(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(connection) = self.ssh_profile.connection.as_mut() {
            connection.reconnect_generation += 1;
        }
        if let Some(banner) = self.ssh_profile.reconnect_banner.clone() {
            banner.update(ctx, |banner, ctx| banner.set_countdown(None, ctx));
        }
    }

    fn show_ssh_reconnect_banner(
        &mut self,
        target: String,
        reason: Option<String>,
        is_network_failure: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.remove_ssh_reconnect_banner();
        let disconnected_at = Local::now().format("%H:%M:%S").to_string();
        let banner = ctx.add_typed_action_view(|_| {
            SshReconnectBanner::new(target, reason, is_network_failure, disconnected_at)
        });
        ctx.subscribe_to_view(&banner, |me, _, event, ctx| match event {
            SshReconnectBannerEvent::Reconnect => me.reconnect_ssh(true, ctx),
            SshReconnectBannerEvent::StopAutoReconnect => me.cancel_ssh_auto_reconnect(ctx),
            SshReconnectBannerEvent::Dismissed => {
                me.remove_ssh_reconnect_banner();
                me.ssh_profile.connection = None;
                ctx.notify();
            }
        });
        self.insert_rich_content(
            None,
            banner.clone(),
            None,
            RichContentInsertionPosition::Append {
                insert_below_long_running_block: true,
            },
            ctx,
        );
        self.ssh_profile.reconnect_banner = Some(banner);
    }

    fn remove_ssh_reconnect_banner(&mut self) {
        let Some(banner) = self.ssh_profile.reconnect_banner.take() else {
            return;
        };
        let view_id = banner.id();
        self.model
            .lock()
            .block_list_mut()
            .remove_rich_content(view_id);
        self.rich_content_views
            .retain(|rich_content| rich_content.view_id() != view_id);
    }

    /// Reruns the watched connection. A manual reconnect starts the automatic retry count over.
    fn reconnect_ssh(&mut self, manual: bool, ctx: &mut ViewContext<Self>) {
        // The ssh command would be appended to whatever the user is typing and run with it.
        if !self.input.as_ref(ctx).buffer_text(ctx).trim().is_empty() {
            self.cancel_ssh_auto_reconnect(ctx);
            if manual {
                self.show_error_toast("Clear the command line to reconnect.".to_string(), ctx);
            }
            return;
        }
        let Some(connection) = self.ssh_profile.connection.as_ref() else {
            return;
        };
        let attempt = if manual { 0 } else { connection.attempt };
        let profile_id = connection.profile_id;
        let target = connection.target.clone();
        let profile = profile_id.and_then(|profile_id| {
            SshSettings::as_ref(ctx)
                .saved_ssh_host_profiles
                .value()
                .iter()
                .find(|profile| profile.id == profile_id)
                .cloned()
        });
        self.remove_ssh_reconnect_banner();
        match (profile, profile_id) {
            (Some(profile), _) => self.connect_ssh_profile(&profile, ctx),
            (None, Some(_)) => {
                self.show_error_toast(
                    format!("Can't reconnect: the SSH host \"{target}\" was removed."),
                    ctx,
                );
                return;
            }
            (None, None) => self.connect_ssh_destination(target, ctx),
        }
        if let Some(connection) = self.ssh_profile.connection.as_mut() {
            connection.attempt = attempt;
        }
    }

    /// Puts the session bar for an SSH manager connection above the terminal contents.
    pub(super) fn wrap_with_ssh_session_bar(
        &self,
        element: Box<dyn Element>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let Some(connection) = self.ssh_profile.connection.as_ref() else {
            return element;
        };
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let profile = connection.profile_id.and_then(|profile_id| {
            SshSettings::as_ref(app)
                .saved_ssh_host_profiles
                .value()
                .iter()
                .find(|profile| profile.id == profile_id)
                .cloned()
        });
        let color = profile.as_ref().and_then(host_color);
        let line_color = color
            .map(|color| ansi_color(color, appearance))
            .unwrap_or_else(|| theme.outline().into_solid());
        let (status_color, status) = if connection.local_session_id.is_some() {
            (AnsiColorIdentifier::Green, "Connected")
        } else if connection.disconnected {
            (AnsiColorIdentifier::Red, "Disconnected")
        } else {
            (AnsiColorIdentifier::Yellow, "Connecting…")
        };
        let sub_color = theme.sub_text_color(theme.background());

        let mut row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(10.);
        match &profile {
            Some(profile) => {
                row = row
                    .with_child(render_badge(&badge_label(profile), color, appearance))
                    .with_child(
                        Text::new_inline(profile.name.clone(), appearance.ui_font_family(), 13.)
                            .with_color(theme.active_ui_text_color().into())
                            .with_style(Properties::default().weight(Weight::Semibold))
                            .finish(),
                    )
                    .with_child(
                        Text::new_inline(
                            format!("{}:{}", profile.target(), profile.port),
                            appearance.monospace_font_family(),
                            12.,
                        )
                        .with_clip(ClipConfig::ellipsis())
                        .with_color(sub_color.into())
                        .finish(),
                    );
                if let Some(jump_host) = profile.jump_hosts.first() {
                    row =
                        row.with_child(render_chip(&format!("via {}", jump_host.host), appearance));
                }
            }
            None => {
                row = row.with_child(
                    Text::new_inline(
                        format!("ssh {}", connection.target),
                        appearance.monospace_font_family(),
                        12.,
                    )
                    .with_clip(ClipConfig::ellipsis())
                    .with_color(theme.active_ui_text_color().into())
                    .finish(),
                );
            }
        }
        row = row
            .with_child(
                Container::new(warpui::elements::Empty::new().finish())
                    .with_border(Border::left(1.).with_border_fill(theme.outline()))
                    .finish(),
            )
            .with_child(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(6.)
                    .with_child(render_dot(ansi_color(status_color, appearance), 8.))
                    .with_child(
                        Text::new_inline(status, appearance.ui_font_family(), 12.)
                            .with_color(sub_color.into())
                            .finish(),
                    )
                    .finish(),
            )
            .with_child(Expanded::new(1., warpui::elements::Empty::new().finish()).finish());

        let bar = warpui::elements::ConstrainedBox::new(
            Container::new(row.finish())
                .with_background(internal_colors::fg_overlay_1(theme))
                .with_border(
                    Border::top(2.).with_border_fill(crate::themes::theme::Fill::Solid(line_color)),
                )
                .with_horizontal_padding(14.)
                .finish(),
        )
        .with_height(SESSION_BAR_HEIGHT)
        .finish();

        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(
                Container::new(bar)
                    .with_border(Border::bottom(1.).with_border_fill(theme.outline()))
                    .finish(),
            )
            .with_child(Expanded::new(1., element).finish())
            .finish()
    }

    fn arm_ssh_profile_auto_inject(
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
        self.ssh_profile.auto_inject = None;
        self.ssh_profile.warpification_enabled_at_arm = false;
        self.ssh_profile.warpify_disabled_command = None;
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
        let (active_block_id, active_command, output) = {
            let model = self.model.lock();
            let active_block = model.block_list().active_block();
            (
                active_block.id().clone(),
                active_block.command_to_string(),
                active_block.output_to_string(),
            )
        };
        let output_tail = last_chars(&output, SSH_PASSWORD_PROMPT_SCAN_CHARS);

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
        if !only_ssh_client_output_before_prompt(&output) {
            log::warn!(
                "Disarming SSH profile password auto-inject: the password prompt follows output that did not come from ssh for profile_id={profile_id}"
            );
            self.disarm_ssh_profile_auto_inject();
            return true;
        }

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
