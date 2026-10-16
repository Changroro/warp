use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use settings::Setting as _;
use uuid::Uuid;
use warp_core::ui::theme::AnsiColorIdentifier;
use warpui::elements::{Align, ChildView, Container, MouseStateHandle, SavePosition};
use warpui::fonts::Properties;
use warpui::ui_components::components::UiComponentStyles;
use warpui::{AppContext, Element, SingletonEntity as _, ViewContext, ViewHandle};

use super::{DefaultSessionModeBehavior, NewSessionSource, Workspace};
use crate::appearance::Appearance;
use crate::menu::{MenuItem, MenuItemFields};
use crate::modal::{Modal, ModalEvent, ModalViewState};
use crate::pane_group::PanesLayout;
use crate::settings::SshSettings;
use crate::ssh_manager::launcher::{LAUNCHER_WIDTH, SshLauncherEvent, SshLauncherView};
use crate::ssh_manager::open_connections::SshOpenConnections;
use crate::ssh_manager::panel::{SshProfilesPanelEvent, SshProfilesPanelView};
use crate::ssh_manager::profile::{QuickConnectDestination, SshHostProfile, quick_connect_args};
use crate::ssh_manager::remote_sessions::{self, RemoteSessions};
use crate::ssh_manager::ui::{ansi_color, host_color};
use crate::ssh_manager::usage::{SshDestination, format_relative_time};
use crate::ssh_manager::{LAUNCHER_BINDING_NAME, SshManagerAction, TOGGLE_PANEL_BINDING_NAME};
use crate::tab::SelectedTabColor;
use crate::terminal::TerminalView;
use crate::themes::theme::Fill as ThemeFill;
use crate::ui_components::icons;
use crate::util::bindings::keybinding_name_to_display_string;
use crate::view_components::DismissibleToast;
use crate::workspace::WorkspaceAction;
use crate::workspace::toast_stack::ToastStack;

const MAX_NEW_SESSION_MENU_PINNED: usize = 3;
const MAX_NEW_SESSION_MENU_RECENT: usize = 3;

pub(super) struct SshManagerWorkspaceState {
    panel_open: bool,
    panel_view: ViewHandle<SshProfilesPanelView>,
    launcher: ModalViewState<Modal<SshLauncherView>>,
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

        let launcher_body = ctx.add_typed_action_view(SshLauncherView::new);
        ctx.subscribe_to_view(&launcher_body, |me, _, event, ctx| match event {
            SshLauncherEvent::Close => me.close_ssh_launcher(ctx),
        });
        let launcher = ctx.add_typed_action_view(|ctx| {
            Modal::new(None, launcher_body, ctx)
                .with_dismiss_on_click()
                .with_modal_style(UiComponentStyles {
                    width: Some(LAUNCHER_WIDTH),
                    ..Default::default()
                })
                .with_body_style(UiComponentStyles {
                    padding: Some(Default::default()),
                    ..Default::default()
                })
        });
        ctx.subscribe_to_view(&launcher, |me, _, event, ctx| {
            if matches!(event, ModalEvent::Close) {
                me.close_ssh_launcher(ctx);
            }
        });

        Self {
            panel_open: false,
            panel_view,
            launcher: ModalViewState::new(launcher),
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
            SshManagerAction::OpenPanel => self.open_ssh_profiles_panel(ctx),
            SshManagerAction::OpenLauncher => self.open_ssh_launcher(ctx),
            SshManagerAction::Connect(profile_id) => {
                self.connect_ssh_profile(*profile_id, ctx);
            }
            SshManagerAction::QuickConnect(input) => self.quick_connect_ssh(input, ctx),
            SshManagerAction::OpenRemoteSessions(profile_id) => {
                self.open_remote_ssh_sessions(*profile_id, ctx)
            }
            SshManagerAction::ReadRemoteSessions {
                profile_id,
                control_path,
            } => self.read_remote_ssh_sessions(*profile_id, control_path.clone(), ctx),
            SshManagerAction::SaveHost { input, connect } => {
                self.save_ssh_host(input, *connect, ctx)
            }
        }
    }

    /// The SSH section of the new-session menu: the launcher, pinned and recent hosts, and the
    /// hosts panel.
    pub(super) fn ssh_new_session_menu_items(
        &self,
        ctx: &AppContext,
    ) -> Vec<MenuItem<WorkspaceAction>> {
        let appearance = Appearance::as_ref(ctx);
        let settings = SshSettings::as_ref(ctx);
        let profiles = settings.saved_ssh_host_profiles.value();
        let usage = settings.ssh_host_usage.value();
        let now = Utc::now();
        let find = |id: Uuid| profiles.iter().find(|profile| profile.id == id);
        let host_item = |profile: &SshHostProfile, detail: String| {
            let color = host_color(profile).unwrap_or(AnsiColorIdentifier::White);
            MenuItemFields::new(profile.name.clone())
                .with_on_select_action(WorkspaceAction::SshManager(SshManagerAction::Connect(
                    profile.id,
                )))
                .with_icon(icons::Icon::CircleFilled)
                .with_icon_size_override(8.)
                .with_override_icon_color(ThemeFill::Solid(ansi_color(color, appearance)))
                .with_right_side_label(detail, Properties::default())
                .into_item()
        };
        let section = |title: &str| MenuItem::Header {
            fields: MenuItemFields::new(title),
            clickable: false,
            right_side_fields: None,
        };

        let mut items = vec![
            MenuItem::Separator,
            MenuItemFields::new("SSH hosts...")
                .with_on_select_action(WorkspaceAction::SshManager(SshManagerAction::OpenLauncher))
                .with_icon(icons::Icon::Globe)
                .with_override_icon_color(appearance.theme().accent())
                .with_key_shortcut_label(keybinding_name_to_display_string(
                    LAUNCHER_BINDING_NAME,
                    ctx,
                ))
                .into_item(),
        ];

        let pinned = usage
            .pinned
            .iter()
            .filter_map(|id| find(*id))
            .take(MAX_NEW_SESSION_MENU_PINNED)
            .collect::<Vec<_>>();
        if !pinned.is_empty() {
            items.push(section("Pinned"));
            items.extend(
                pinned
                    .iter()
                    .map(|profile| host_item(profile, profile.target())),
            );
        }

        let recent = usage
            .recent_profiles()
            .filter(|(id, _)| !usage.is_pinned(*id))
            .filter_map(|(id, at)| find(id).map(|profile| (profile, at)))
            .take(MAX_NEW_SESSION_MENU_RECENT)
            .collect::<Vec<_>>();
        if !recent.is_empty() {
            items.push(section("Recent"));
            items.extend(
                recent
                    .into_iter()
                    .map(|(profile, at)| host_item(profile, format_relative_time(at, now))),
            );
        }

        items.push(
            MenuItemFields::new("Show all hosts")
                .with_on_select_action(WorkspaceAction::SshManager(SshManagerAction::OpenPanel))
                .with_icon(icons::Icon::DotsHorizontal)
                .with_right_side_label("Open panel", Properties::default())
                .into_item(),
        );
        items
    }

    pub(super) fn render_ssh_profiles_panel(&self) -> Option<Box<dyn Element>> {
        if !self.ssh_manager.panel_open || warpui::platform::is_mobile_device() {
            return None;
        }
        Some(ChildView::new(&self.ssh_manager.panel_view).finish())
    }

    pub(super) fn render_ssh_profiles_modal(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        if self.ssh_manager.launcher.is_open() {
            return Some(self.ssh_manager.launcher.render());
        }
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

    fn open_ssh_profiles_panel(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.ssh_manager.panel_open {
            self.ssh_manager.panel_open = true;
            ctx.notify();
        }
        self.ssh_manager
            .panel_view
            .update(ctx, |panel, ctx| panel.focus_search(ctx));
    }

    fn open_ssh_launcher(&mut self, ctx: &mut ViewContext<Self>) {
        self.ssh_manager.launcher.open();
        self.ssh_manager.launcher.view.update(ctx, |modal, ctx| {
            modal
                .body()
                .update(ctx, |launcher, ctx| launcher.on_open(ctx));
        });
        ctx.notify();
    }

    fn close_ssh_launcher(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.ssh_manager.launcher.is_open() {
            return;
        }
        self.ssh_manager.launcher.close();
        self.focus_active_tab(ctx);
        ctx.notify();
    }

    fn record_ssh_connection(&mut self, destination: SshDestination, ctx: &mut ViewContext<Self>) {
        SshSettings::handle(ctx).update(ctx, |settings, ctx| {
            let mut usage = settings.ssh_host_usage.value().clone();
            usage.record_connection(destination, Utc::now());
            if let Err(err) = settings.ssh_host_usage.set_value(usage, ctx) {
                log::warn!("Failed to record SSH connection history: {err:#}");
            }
        });
    }

    fn save_ssh_host(&mut self, input: &str, connect: bool, ctx: &mut ViewContext<Self>) {
        let Some(destination) = QuickConnectDestination::parse(input) else {
            return;
        };
        let result = self
            .ssh_manager
            .panel_view
            .update(ctx, |panel, ctx| panel.save_destination(&destination, ctx));
        match result {
            Ok(profile_id) => {
                if let Some(args) = quick_connect_args(input) {
                    SshSettings::handle(ctx).update(ctx, |settings, ctx| {
                        let mut usage = settings.ssh_host_usage.value().clone();
                        usage.forget_quick_connect(&args);
                        if let Err(err) = settings.ssh_host_usage.set_value(usage, ctx) {
                            log::warn!("Failed to update SSH connection history: {err:#}");
                        }
                    });
                }
                if connect {
                    self.connect_ssh_profile(profile_id, ctx);
                }
            }
            Err(message) => self.show_ssh_toast(DismissibleToast::error(message), ctx),
        }
    }

    fn show_ssh_toast(
        &self,
        toast: DismissibleToast<WorkspaceAction>,
        ctx: &mut ViewContext<Self>,
    ) {
        let window_id = ctx.window_id();
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            toast_stack.add_ephemeral_toast(toast, window_id, ctx);
        });
    }

    fn saved_ssh_profile(&self, profile_id: Uuid, ctx: &AppContext) -> Option<SshHostProfile> {
        SshSettings::as_ref(ctx)
            .saved_ssh_host_profiles
            .value()
            .iter()
            .find(|profile| profile.id == profile_id)
            .cloned()
    }

    fn open_remote_ssh_sessions(&mut self, profile_id: Uuid, ctx: &mut ViewContext<Self>) {
        let live_control_path = SshOpenConnections::terminals(profile_id, ctx)
            .into_iter()
            .find_map(|terminal| terminal.as_ref(ctx).ssh_upload_control_path(ctx));
        if let Some(control_path) = live_control_path {
            self.read_remote_ssh_sessions(profile_id, control_path, ctx);
        } else if let Some(terminal_view) = self.connect_ssh_profile(profile_id, ctx) {
            terminal_view.update(ctx, |terminal_view, _| {
                terminal_view.read_remote_sessions_when_ready();
            });
        }
    }

    fn read_remote_ssh_sessions(
        &mut self,
        profile_id: Uuid,
        control_path: PathBuf,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(profile) = self.saved_ssh_profile(profile_id, ctx) else {
            return;
        };
        self.show_ssh_toast(
            DismissibleToast::default(format!("Reading Warp tabs on {}…", profile.name)),
            ctx,
        );
        ctx.spawn(
            remote_sessions::fetch(control_path),
            move |me, result, ctx| match result {
                Ok(sessions) if sessions.tabs.is_empty() => me.show_ssh_toast(
                    DismissibleToast::error(format!(
                        "{} has no terminal tabs open in {}",
                        profile.name, sessions.database
                    )),
                    ctx,
                ),
                Ok(sessions) => me.open_remote_ssh_tabs(&profile, sessions, ctx),
                Err(err) => me.show_ssh_toast(
                    DismissibleToast::error(format!(
                        "Couldn't read Warp tabs on {}: {err:#}",
                        profile.name
                    )),
                    ctx,
                ),
            },
        );
    }

    fn open_remote_ssh_tabs(
        &mut self,
        profile: &SshHostProfile,
        sessions: RemoteSessions,
        ctx: &mut ViewContext<Self>,
    ) {
        self.record_ssh_connection(SshDestination::Profile(profile.id), ctx);
        let tab_count = sessions.tabs.len();
        let host_tab_color = host_color(profile);
        for tab in sessions.tabs {
            self.add_tab_with_pane_layout(
                PanesLayout::Template(tab.layout),
                Arc::new(HashMap::new()),
                tab.title,
                ctx,
            );
            if let Some(color) = tab.color.or(host_tab_color) {
                self.set_tab_color(self.active_tab_index, SelectedTabColor::Color(color), ctx);
            }
            let pane_group = self.active_tab_pane_group().as_ref(ctx);
            let terminal_views = pane_group
                .visible_pane_ids()
                .into_iter()
                .filter_map(|pane_id| pane_group.terminal_view_from_pane_id(pane_id, ctx))
                .collect::<Vec<_>>();
            for (terminal_view, pane) in terminal_views.into_iter().zip(tab.panes) {
                if let Some(agent) = pane.agent {
                    crate::last_cli_agent::remember(terminal_view.id(), agent, ctx);
                }
                terminal_view.update(ctx, |terminal_view, ctx| {
                    terminal_view.connect_ssh_profile_in(profile, pane.directory, ctx);
                });
            }
        }
        self.show_ssh_toast(
            DismissibleToast::success(format!(
                "Opened {tab_count} tabs from {} ({})",
                profile.name, sessions.database
            )),
            ctx,
        );
        ctx.notify();
    }

    fn connect_ssh_profile(
        &mut self,
        profile_id: Uuid,
        ctx: &mut ViewContext<Self>,
    ) -> Option<ViewHandle<TerminalView>> {
        let Some(profile) = self.saved_ssh_profile(profile_id, ctx) else {
            log::warn!(
                "Failed to start SSH profile session: no saved profile for profile_id={profile_id}"
            );
            return None;
        };

        self.record_ssh_connection(SshDestination::Profile(profile_id), ctx);
        let color = host_color(&profile);
        let Some(terminal_view) = self.open_ssh_tab(&profile.name, color, ctx) else {
            log::warn!(
                "Failed to start SSH profile session: no active session view for profile_id={profile_id}"
            );
            return None;
        };
        terminal_view.update(ctx, |terminal_view, ctx| {
            terminal_view.connect_ssh_profile(&profile, ctx);
        });
        Some(terminal_view)
    }

    fn quick_connect_ssh(&mut self, input: &str, ctx: &mut ViewContext<Self>) {
        let Some(args) = quick_connect_args(input) else {
            return;
        };
        self.record_ssh_connection(SshDestination::QuickConnect(args.clone()), ctx);
        if let Some(terminal_view) = self.open_ssh_tab(&args, None, ctx) {
            terminal_view.update(ctx, |terminal_view, ctx| {
                terminal_view.connect_ssh_destination(args, ctx);
            });
        } else {
            log::warn!("Failed to start SSH quick connect: no active session view");
        }
    }

    fn open_ssh_tab(
        &mut self,
        title: &str,
        color: Option<AnsiColorIdentifier>,
        ctx: &mut ViewContext<Self>,
    ) -> Option<ViewHandle<TerminalView>> {
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
        let pane_group = self.active_tab_pane_group().clone();
        pane_group.update(ctx, |pane_group, ctx| pane_group.set_title(title, ctx));
        if let Some(color) = color {
            self.set_tab_color(self.active_tab_index, SelectedTabColor::Color(color), ctx);
        }
        ctx.notify();
        pane_group.as_ref(ctx).active_session_view(ctx)
    }
}
