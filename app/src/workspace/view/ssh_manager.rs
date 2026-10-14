use settings::Setting as _;
use uuid::Uuid;
use warpui::elements::{Align, ChildView, Container, MouseStateHandle, SavePosition};
use warpui::{AppContext, Element, SingletonEntity as _, ViewContext, ViewHandle};

use super::{DefaultSessionModeBehavior, NewSessionSource, Workspace};
use crate::appearance::Appearance;
use crate::settings::SshSettings;
use crate::ssh_manager::panel::{SshProfilesPanelEvent, SshProfilesPanelView};
use crate::ssh_manager::{SshManagerAction, TOGGLE_PANEL_BINDING_NAME};
use crate::terminal::warpify::settings::WarpifySettings;
use crate::ui_components::icons;
use crate::util::bindings::keybinding_name_to_display_string;
use crate::workspace::WorkspaceAction;

pub(super) struct SshManagerWorkspaceState {
    panel_open: bool,
    panel_view: ViewHandle<SshProfilesPanelView>,
    toolbar_icon_mouse_state: MouseStateHandle,
}

impl SshManagerWorkspaceState {
    pub(super) fn new(ctx: &mut ViewContext<Workspace>) -> Self {
        let panel_view = ctx.add_typed_action_view(SshProfilesPanelView::new);
        ctx.subscribe_to_view(&panel_view, |_, _, event, ctx| {
            if matches!(event, SshProfilesPanelEvent::ModalToggled) {
                ctx.notify();
            }
        });
        Self {
            panel_open: false,
            panel_view,
            toolbar_icon_mouse_state: Default::default(),
        }
    }
}

impl Workspace {
    pub(super) fn handle_ssh_manager_action(
        &mut self,
        action: &SshManagerAction,
        ctx: &mut ViewContext<Self>,
    ) {
        match action {
            SshManagerAction::TogglePanel => self.toggle_ssh_profiles_panel(ctx),
            SshManagerAction::Connect(profile_id) => self.connect_ssh_profile(*profile_id, ctx),
        }
    }

    pub(super) fn render_ssh_profiles_panel(&self) -> Option<Box<dyn Element>> {
        if !self.ssh_manager.panel_open || warpui::platform::is_mobile_device() {
            return None;
        }
        Some(ChildView::new(&self.ssh_manager.panel_view).finish())
    }

    pub(super) fn render_ssh_profiles_modal(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        if !self.ssh_manager.panel_open {
            return None;
        }
        self.ssh_manager
            .panel_view
            .read(app, |panel, _| panel.render_modal())
    }

    pub(super) fn render_ssh_profiles_button(
        &self,
        appearance: &Appearance,
        ctx: &AppContext,
    ) -> Box<dyn Element> {
        SavePosition::new(
            Container::new(
                Align::new(
                    self.render_tab_bar_icon_button(
                        appearance,
                        icons::Icon::Globe,
                        &self.ssh_manager.toolbar_icon_mouse_state,
                        WorkspaceAction::SshManager(SshManagerAction::TogglePanel),
                        "SSH profiles".to_string(),
                        keybinding_name_to_display_string(TOGGLE_PANEL_BINDING_NAME, ctx),
                        self.ssh_manager.panel_open,
                        false,
                    )
                    .finish(),
                )
                .finish(),
            )
            .finish(),
            TOGGLE_PANEL_BINDING_NAME,
        )
        .finish()
    }

    fn toggle_ssh_profiles_panel(&mut self, ctx: &mut ViewContext<Self>) {
        self.ssh_manager.panel_open = !self.ssh_manager.panel_open;
        if !self.ssh_manager.panel_open {
            self.ssh_manager
                .panel_view
                .update(ctx, |panel, ctx| panel.on_panel_hidden(ctx));
        }
        ctx.notify();
    }

    fn connect_ssh_profile(&mut self, profile_id: Uuid, ctx: &mut ViewContext<Self>) {
        let Some(profile) = SshSettings::as_ref(ctx)
            .saved_ssh_host_profiles
            .value()
            .iter()
            .find(|profile| profile.id == profile_id)
            .cloned()
        else {
            log::warn!(
                "Failed to start SSH profile session: no saved profile for profile_id={profile_id}"
            );
            return;
        };

        let enable_ssh_warpification = *WarpifySettings::as_ref(ctx)
            .enable_ssh_warpification
            .value();
        let command = profile.to_ssh_command();
        // A jump host's prompt can arrive first, so auto-inject would send the final password to it.
        let enable_password_auto_inject = profile.jump_hosts.is_empty();
        log::info!(
            "Connecting SSH profile: profile_id={profile_id}, enable_ssh_warpification={enable_ssh_warpification}"
        );
        let window_id = ctx.window_id();
        self.add_new_session_tab_internal_with_default_session_mode_behavior(
            NewSessionSource::Tab,
            Some(window_id),
            None,
            None,
            false,
            DefaultSessionModeBehavior::Ignore,
            ctx,
        );

        self.active_tab_pane_group()
            .update(ctx, |active_group, ctx| {
                if let Some(terminal_view) = active_group.active_session_view(ctx) {
                    terminal_view.update(ctx, |terminal_view, ctx| {
                        terminal_view.arm_ssh_profile_auto_inject(
                            profile_id,
                            command.clone(),
                            enable_ssh_warpification,
                            enable_password_auto_inject,
                            ctx,
                        );
                        terminal_view.execute_command_or_set_pending(&command, ctx);
                    });
                } else {
                    log::warn!(
                        "Failed to start SSH profile session: no active session view for profile_id={profile_id}"
                    );
                }
            });
        ctx.notify();
    }
}
