use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::PoisonError;

use chrono::Utc;
use pathfinder_geometry::vector::vec2f;
use settings::Setting as _;
use uuid::Uuid;
use warp_core::ui::theme::AnsiColorIdentifier;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    Border, ChildAnchor, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container,
    CornerRadius, CrossAxisAlignment, Empty, Expanded, Fill as ElementFill, Flex, Hoverable,
    MainAxisSize, MouseStateHandle, OffsetPositioning, Padding, ParentAnchor, ParentElement,
    ParentOffsetBounds, Radius, ScrollbarWidth, Shrinkable, Stack, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::platform::Cursor;
use warpui::text_layout::ClipConfig;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};
use warpui_extras::secure_storage::AppContextExt as _;

use crate::appearance::Appearance;
use crate::editor::{EditorView, Event as EditorEvent, SingleLineEditorOptions, TextOptions};
use crate::modal::{Modal, ModalEvent, ModalViewState};
use crate::settings::SshSettings;
use crate::ssh_manager::host_editor::{
    PasswordIntent, SshProfileDeleteModalEvent, SshProfileDeleteModalView, SshProfileModalEvent,
    SshProfileModalMode, SshProfileModalView,
};
use crate::ssh_manager::open_connections::SshOpenConnections;
use crate::ssh_manager::profile::{
    QuickConnectDestination, SshHostProfile, SshJumpHost, SshProfileIdReassignment,
    normalize_ssh_host_profiles, quick_connect_args, ssh_profile_password_key,
};
use crate::ssh_manager::ui::{ansi_color, host_color, host_subtitle, render_chip, render_dot};
use crate::ssh_manager::usage::{SshHostUsage, format_relative_time};
use crate::ssh_manager::{SshManagerAction, TOGGLE_PANEL_BINDING_NAME, config_file};
use crate::themes::theme::Fill as ThemeFill;
use crate::ui_components::buttons::icon_button_with_color;
use crate::ui_components::icons::Icon;
use crate::util::bindings::keybinding_name_to_display_string;
use crate::workspace::WorkspaceAction;

pub(crate) const PANEL_WIDTH: f32 = 360.;
const PANEL_PADDING: f32 = 10.;
const ROW_RADIUS: f32 = 4.;
const ROW_SPACING: f32 = 9.;
const ROW_ACTION_BUTTON_ICON_SIZE: f32 = 12.;
const ROW_ACTION_BUTTON_PADDING: f32 = 3.;
const ROW_ACTION_BUTTON_GAP: f32 = 2.;
const SCROLLBAR_WIDTH: f32 = 4.;
const PROFILE_MODAL_WIDTH: f32 = 520.;
const MODAL_MAX_HEIGHT_PERCENTAGE: f32 = 0.8;
const DELETE_MODAL_HEIGHT: f32 = 220.;
const DELETE_MODAL_BODY_HEIGHT: f32 = 150.;
const FIELD_RADIUS: f32 = 6.;
const NOTICE_SPACING: f32 = 6.;
const COLOR_DOT_SIZE: f32 = 10.;
const CHECKBOX_SIZE: f32 = 15.;
const MAX_RECENT_ROWS: usize = 5;
const PERMISSION_HINT: &str =
    " Check that you can read and write ~/.ssh/config and that ~/.ssh is yours.";

#[derive(Debug, Clone)]
pub enum SshProfilesPanelEvent {
    ModalToggled,
}

#[derive(Debug, Clone)]
pub enum SshProfilesPanelAction {
    Add,
    Connect(Uuid),
    Edit(Uuid),
    Remove(Uuid),
    TogglePinned(Uuid),
    QuickConnect,
    SetFilter(HostFilter),
    ToggleGroup(String),
    ToggleSelected(Uuid),
    ClearSelection,
    OpenSelected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostFilter {
    All,
    Pinned,
    Open,
}

#[derive(Default, Clone)]
struct ProfileRowMouseStates {
    row: MouseStateHandle,
    checkbox: MouseStateHandle,
    actions: MouseStateHandle,
    connect: MouseStateHandle,
    pin: MouseStateHandle,
    edit: MouseStateHandle,
    remove: MouseStateHandle,
}

/// The list section a row is drawn in. A host can show up in more than one section, and each copy
/// needs its own mouse state.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum RowSection {
    Search,
    Pinned,
    Recent,
    Group,
}

#[derive(Default)]
struct PanelMouseStates {
    close: MouseStateHandle,
    new_host: MouseStateHandle,
    quick_connect: MouseStateHandle,
    clear_selection: MouseStateHandle,
    open_selected: MouseStateHandle,
    filters: HashMap<HostFilter, MouseStateHandle>,
    groups: RefCell<HashMap<String, MouseStateHandle>>,
}

pub struct SshProfilesPanelView {
    modal_state: ModalViewState<Modal<SshProfileModalView>>,
    delete_modal_state: ModalViewState<Modal<SshProfileDeleteModalView>>,
    search_editor: ViewHandle<EditorView>,
    filter: HostFilter,
    collapsed_groups: HashSet<String>,
    selected: Vec<Uuid>,
    row_mouse_states: RefCell<HashMap<(RowSection, Uuid), ProfileRowMouseStates>>,
    mouse_states: PanelMouseStates,
    list_scroll_state: ClippedScrollStateHandle,
    notices: Vec<String>,
}

impl SshProfilesPanelView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let notices = sync_profiles_with_ssh_config(ctx);

        let modal_body = ctx.add_typed_action_view(SshProfileModalView::new);
        ctx.subscribe_to_view(&modal_body, |me, _, event, ctx| {
            me.handle_modal_body_event(event, ctx);
        });

        let modal = ctx.add_typed_action_view(|ctx| {
            Modal::new(Some("SSH profile".to_string()), modal_body, ctx)
                .with_dismiss_on_click()
                .with_max_height_percentage(MODAL_MAX_HEIGHT_PERCENTAGE)
                .with_modal_style(UiComponentStyles {
                    width: Some(PROFILE_MODAL_WIDTH),
                    ..Default::default()
                })
                .with_body_style(UiComponentStyles {
                    padding: Some(Coords {
                        top: 18.,
                        bottom: 18.,
                        left: 20.,
                        right: 20.,
                    }),
                    ..Default::default()
                })
        });
        ctx.subscribe_to_view(&modal, |me, _, event, ctx| {
            if matches!(event, ModalEvent::Close) {
                me.close_modal(ctx);
            }
        });

        let delete_modal_body = ctx.add_typed_action_view(|_| SshProfileDeleteModalView::new());
        ctx.subscribe_to_view(&delete_modal_body, |me, _, event, ctx| {
            me.handle_delete_modal_event(event, ctx);
        });
        let delete_modal = ctx.add_typed_action_view(|ctx| {
            Modal::new(
                Some("Delete SSH profile?".to_string()),
                delete_modal_body,
                ctx,
            )
            .with_dismiss_on_click()
            .with_modal_style(UiComponentStyles {
                height: Some(DELETE_MODAL_HEIGHT),
                ..Default::default()
            })
            .with_body_style(UiComponentStyles {
                height: Some(DELETE_MODAL_BODY_HEIGHT),
                ..Default::default()
            })
        });
        ctx.subscribe_to_view(&delete_modal, |me, _, event, ctx| {
            if matches!(event, ModalEvent::Close) {
                me.close_delete_modal(ctx);
            }
        });

        ctx.subscribe_to_model(&SshSettings::handle(ctx), |me, _, _, ctx| {
            me.reconcile_row_mouse_states(ctx);
            ctx.notify();
        });
        let open_connections = SshOpenConnections::handle_or_init(ctx);
        ctx.subscribe_to_model(&open_connections, |_, _, _, ctx| ctx.notify());

        let search_editor = ctx.add_view(|ctx| {
            EditorView::single_line(
                SingleLineEditorOptions {
                    text: TextOptions::ui_text(None, Appearance::as_ref(ctx)),
                    ..Default::default()
                },
                ctx,
            )
        });
        search_editor.update(ctx, |editor, ctx| {
            editor.set_placeholder_text("Search name, tag, or user@host", ctx);
        });
        ctx.subscribe_to_view(&search_editor, |me, _, event, ctx| {
            me.handle_search_event(event, ctx);
        });

        Self {
            modal_state: ModalViewState::new(modal),
            delete_modal_state: ModalViewState::new(delete_modal),
            search_editor,
            filter: HostFilter::All,
            collapsed_groups: HashSet::new(),
            selected: Vec::new(),
            row_mouse_states: Default::default(),
            mouse_states: PanelMouseStates {
                filters: [HostFilter::All, HostFilter::Pinned, HostFilter::Open]
                    .into_iter()
                    .map(|filter| (filter, MouseStateHandle::default()))
                    .collect(),
                ..Default::default()
            },
            list_scroll_state: Default::default(),
            notices,
        }
    }

    fn sync_profiles(&mut self, ctx: &mut ViewContext<Self>) {
        self.notices = sync_profiles_with_ssh_config(ctx);
    }

    pub fn render_modal(&self) -> Option<Box<dyn Element>> {
        if self.modal_state.is_open() {
            Some(self.modal_state.render())
        } else if self.delete_modal_state.is_open() {
            Some(self.delete_modal_state.render())
        } else {
            None
        }
    }

    pub fn on_panel_hidden(&mut self, ctx: &mut ViewContext<Self>) {
        self.close_modal(ctx);
        self.close_delete_modal(ctx);
    }

    pub fn focus_search(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.search_editor);
    }

    fn search_query(&self, app: &AppContext) -> String {
        self.search_editor
            .as_ref(app)
            .buffer_text(app)
            .trim()
            .to_string()
    }

    fn clear_search(&mut self, ctx: &mut ViewContext<Self>) {
        self.search_editor
            .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
    }

    fn handle_search_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Enter => {
                let query = self.search_query(ctx);
                if query.is_empty() {
                    return;
                }
                let first_match = Self::profiles(ctx)
                    .into_iter()
                    .find(|profile| profile.matches_query(&query));
                self.clear_search(ctx);
                let action = match first_match {
                    Some(profile) => SshManagerAction::Connect(profile.id),
                    None => SshManagerAction::QuickConnect(query),
                };
                ctx.dispatch_typed_action(&WorkspaceAction::SshManager(action));
            }
            EditorEvent::Escape => {
                if self.search_query(ctx).is_empty() {
                    ctx.dispatch_typed_action(&WorkspaceAction::SshManager(
                        SshManagerAction::TogglePanel,
                    ));
                } else {
                    self.clear_search(ctx);
                }
            }
            _ => ctx.notify(),
        }
    }

    fn quick_connect(&mut self, ctx: &mut ViewContext<Self>) {
        let query = self.search_query(ctx);
        self.clear_search(ctx);
        ctx.dispatch_typed_action(&WorkspaceAction::SshManager(
            SshManagerAction::QuickConnect(query),
        ));
    }

    fn toggle_pinned(&mut self, profile_id: Uuid, ctx: &mut ViewContext<Self>) {
        SshSettings::handle(ctx).update(ctx, |settings, ctx| {
            let mut usage = settings.ssh_host_usage.value().clone();
            usage.toggle_pinned(profile_id);
            if let Err(err) = settings.ssh_host_usage.set_value(usage, ctx) {
                log::warn!("Failed to save pinned SSH profiles: {err:#}");
            }
        });
    }

    /// Saves `destination` as a profile in `~/.ssh/config` and returns its id.
    pub fn save_destination(
        &mut self,
        destination: &QuickConnectDestination,
        ctx: &mut ViewContext<Self>,
    ) -> Result<Uuid, String> {
        let profiles = Self::profiles(ctx);
        let mut profile = destination.to_profile();
        let taken = |name: &str| profiles.iter().any(|other| other.name == name);
        let mut suffix = 2;
        while taken(&profile.name) {
            profile.name = format!("{}-{suffix}", destination.host);
            suffix += 1;
        }
        if let Err(err) = config_file::upsert_profile(&profile) {
            log::error!("Failed to save SSH profile to ~/.ssh/config: {err:#}");
            return Err(format!("Could not write ~/.ssh/config: {err:#}"));
        }
        self.sync_profiles(ctx);
        ctx.notify();
        Ok(profile.id)
    }

    fn toggle_selected(&mut self, profile_id: Uuid, ctx: &mut ViewContext<Self>) {
        if let Some(index) = self.selected.iter().position(|id| *id == profile_id) {
            self.selected.remove(index);
        } else {
            self.selected.push(profile_id);
        }
        ctx.notify();
    }

    fn open_selected(&mut self, ctx: &mut ViewContext<Self>) {
        for profile_id in std::mem::take(&mut self.selected) {
            ctx.dispatch_typed_action(&WorkspaceAction::SshManager(SshManagerAction::Connect(
                profile_id,
            )));
        }
        ctx.notify();
    }

    fn profiles(app: &AppContext) -> Vec<SshHostProfile> {
        SshSettings::as_ref(app)
            .saved_ssh_host_profiles
            .value()
            .clone()
    }

    fn reconcile_row_mouse_states(&mut self, ctx: &mut ViewContext<Self>) {
        let ids = Self::profiles(ctx)
            .into_iter()
            .map(|profile| profile.id)
            .collect::<std::collections::HashSet<_>>();
        self.row_mouse_states
            .borrow_mut()
            .retain(|(_, id), _| ids.contains(id));
    }

    fn open_add_modal(&mut self, ctx: &mut ViewContext<Self>) {
        let profiles = Self::profiles(ctx);
        self.modal_state.view.update(ctx, |modal, ctx| {
            modal.set_title(Some("New SSH profile".to_string()));
            modal.body().update(ctx, |body, ctx| {
                body.open_add(profiles, ctx);
            });
        });
        self.modal_state.open();
        self.focus_modal_body(ctx);
        ctx.emit(SshProfilesPanelEvent::ModalToggled);
        ctx.notify();
    }

    fn open_edit_modal(&mut self, profile_id: Uuid, ctx: &mut ViewContext<Self>) {
        let profiles = Self::profiles(ctx);
        let Some(profile) = profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .cloned()
        else {
            log::warn!("Cannot edit SSH profile {profile_id}: it is not in the saved profiles");
            return;
        };

        self.modal_state.view.update(ctx, |modal, ctx| {
            modal.set_title(Some("Edit SSH profile".to_string()));
            modal.body().update(ctx, |body, ctx| {
                body.open_edit(profile, profiles, ctx);
            });
        });
        self.modal_state.open();
        self.focus_modal_body(ctx);
        ctx.emit(SshProfilesPanelEvent::ModalToggled);
        ctx.notify();
    }

    fn focus_modal_body(&mut self, ctx: &mut ViewContext<Self>) {
        self.modal_state.view.update(ctx, |modal, ctx| {
            modal.body().update(ctx, |body, ctx| {
                body.on_open(ctx);
            });
        });
    }

    fn close_modal(&mut self, ctx: &mut ViewContext<Self>) {
        if self.modal_state.is_open() {
            self.modal_state.view.update(ctx, |modal, ctx| {
                modal.body().update(ctx, |body, ctx| {
                    body.reset_sensitive_state(ctx);
                });
            });
            self.modal_state.close();
            ctx.emit(SshProfilesPanelEvent::ModalToggled);
            ctx.notify();
        }
    }

    fn handle_modal_body_event(
        &mut self,
        event: &SshProfileModalEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SshProfileModalEvent::Cancel => self.close_modal(ctx),
            SshProfileModalEvent::Submit {
                mode,
                profile,
                password,
            } => match self.save_profile(
                mode.clone(),
                profile.as_ref().clone(),
                password.clone(),
                ctx,
            ) {
                Ok(()) => self.close_modal(ctx),
                Err(message) => self.modal_state.view.update(ctx, |modal, ctx| {
                    modal.body().update(ctx, |body, ctx| {
                        body.set_error(Some(message), ctx);
                    });
                }),
            },
        }
    }

    fn open_delete_modal(&mut self, profile_id: Uuid, ctx: &mut ViewContext<Self>) {
        let profiles = Self::profiles(ctx);
        let Some(profile) = profiles.iter().find(|profile| profile.id == profile_id) else {
            log::warn!("Cannot delete SSH profile {profile_id}: it is not in the saved profiles");
            return;
        };

        let name = profile.name.clone();
        self.delete_modal_state.view.update(ctx, |modal, ctx| {
            modal.body().update(ctx, |body, ctx| {
                body.open(profile_id, name, ctx);
            });
        });
        self.delete_modal_state.open();
        ctx.emit(SshProfilesPanelEvent::ModalToggled);
        ctx.notify();
    }

    fn close_delete_modal(&mut self, ctx: &mut ViewContext<Self>) {
        if self.delete_modal_state.is_open() {
            self.delete_modal_state.view.update(ctx, |modal, ctx| {
                modal.body().update(ctx, |body, ctx| {
                    body.reset(ctx);
                });
            });
            self.delete_modal_state.close();
            ctx.emit(SshProfilesPanelEvent::ModalToggled);
            ctx.notify();
        }
    }

    fn handle_delete_modal_event(
        &mut self,
        event: &SshProfileDeleteModalEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SshProfileDeleteModalEvent::Cancel => self.close_delete_modal(ctx),
            SshProfileDeleteModalEvent::Confirm(profile_id) => {
                match self.remove_profile(*profile_id, ctx) {
                    Ok(()) => self.close_delete_modal(ctx),
                    Err(message) => self.delete_modal_state.view.update(ctx, |modal, ctx| {
                        modal.body().update(ctx, |body, ctx| {
                            body.set_error(Some(message), ctx);
                        });
                    }),
                }
            }
        }
    }

    fn save_profile(
        &mut self,
        mode: SshProfileModalMode,
        profile: SshHostProfile,
        password: PasswordIntent,
        ctx: &mut ViewContext<Self>,
    ) -> Result<(), String> {
        let mut profile = profile;
        let mut password = password;
        if let SshProfileModalMode::Edit(id) = mode {
            profile.id = id;
            let destination_changed = Self::profiles(ctx).iter().any(|old| {
                old.id == id
                    && (old.host != profile.host
                        || old.user != profile.user
                        || old.port != profile.port)
            });
            // A saved password belongs to the machine it was typed for.
            if destination_changed && matches!(password, PasswordIntent::Keep) {
                password = PasswordIntent::Clear;
            }
        }

        if let Err(err) = config_file::upsert_profile(&profile) {
            log::error!("Failed to save SSH profile to ~/.ssh/config: {err:#}");
            return Err(format!("Could not write ~/.ssh/config: {err:#}"));
        }

        let profile_id = profile.id;
        self.sync_profiles(ctx);

        match password {
            PasswordIntent::Keep => Ok(()),
            PasswordIntent::Set(password) => {
                self.write_profile_password(profile_id, password.as_str(), ctx)
            }
            PasswordIntent::Clear => self
                .remove_profile_password(profile_id, ctx)
                .map_err(|err| {
                    format!("The profile was saved, but its stored password is still there: {err}")
                }),
        }
    }

    fn write_profile_password(
        &self,
        profile_id: Uuid,
        password: &str,
        ctx: &mut ViewContext<Self>,
    ) -> Result<(), String> {
        let key = ssh_profile_password_key(profile_id);
        ctx.secure_storage()
            .write_value_with_owner_only_fallback(&key, password)
            .map_err(|err| {
                log::error!("Failed to write SSH profile password: {err:#}");
                format!("The profile was saved, but its password could not be stored: {err:#}")
            })
    }

    fn remove_profile_password(
        &self,
        profile_id: Uuid,
        ctx: &mut ViewContext<Self>,
    ) -> Result<(), String> {
        match ctx
            .secure_storage()
            .remove_value(&ssh_profile_password_key(profile_id))
        {
            Ok(()) => Ok(()),
            Err(warpui_extras::secure_storage::Error::NotFound) => Ok(()),
            Err(err) => {
                log::error!("Failed to remove SSH profile password: {err:#}");
                Err(format!("{err:#}"))
            }
        }
    }

    fn remove_profile(
        &mut self,
        profile_id: Uuid,
        ctx: &mut ViewContext<Self>,
    ) -> Result<(), String> {
        let profiles = Self::profiles(ctx);
        let Some(profile) = profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .cloned()
        else {
            log::warn!("Cannot delete SSH profile {profile_id}: it is not in the saved profiles");
            return Err("This profile is no longer in the list.".to_string());
        };

        let warp_owned_ids = warp_owned_profile_ids().map_err(|err| {
            log::error!("Failed to read SSH profiles from ~/.ssh/config: {err:#}");
            format!("Could not read ~/.ssh/config: {err:#}")
        })?;
        if !warp_owned_ids.contains(&profile_id) {
            log::warn!(
                "Refusing to delete hand-written ssh config host {}",
                profile.name
            );
            return Err(format!(
                "\"{}\" is written by hand in ~/.ssh/config. Remove it by editing that file.",
                profile.name
            ));
        }

        if let Err(err) = config_file::remove_profile(profile_id) {
            log::error!("Failed to remove SSH profile from ~/.ssh/config: {err:#}");
            return Err(format!("Could not write ~/.ssh/config: {err:#}"));
        }

        for mut other in profiles.into_iter().filter(|other| {
            other.id != profile_id
                && warp_owned_ids.contains(&other.id)
                && other
                    .jump_hosts
                    .iter()
                    .any(|jump_host| jump_host_points_at(jump_host, &profile))
        }) {
            other
                .jump_hosts
                .retain(|jump_host| !jump_host_points_at(jump_host, &profile));
            if let Err(err) = config_file::upsert_profile(&other) {
                log::error!("Failed to drop jump host from ~/.ssh/config: {err:#}");
                self.sync_profiles(ctx);
                return Err(format!(
                    "\"{}\" was deleted, but \"{}\" still lists it as a jump host: {err:#}",
                    profile.name, other.name
                ));
            }
        }

        self.sync_profiles(ctx);
        self.row_mouse_states
            .borrow_mut()
            .retain(|(_, id), _| *id != profile_id);
        ctx.notify();

        self.remove_profile_password(profile_id, ctx)
            .map_err(|err| {
                format!(
                    "\"{}\" was deleted, but its stored password is still there: {err}",
                    profile.name
                )
            })
    }

    fn render_header(
        &self,
        count: usize,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let close_button = {
            let ui_builder = appearance.ui_builder().clone();
            let keybinding = keybinding_name_to_display_string(TOGGLE_PANEL_BINDING_NAME, app);
            let tooltip = if let Some(keybinding) = keybinding {
                ui_builder
                    .tool_tip_with_sublabel("Close panel".to_string(), keybinding)
                    .build()
                    .finish()
            } else {
                ui_builder
                    .tool_tip("Close panel".to_string())
                    .build()
                    .finish()
            };

            icon_button_with_color(
                appearance,
                Icon::X,
                false,
                self.mouse_states.close.clone(),
                theme.sub_text_color(theme.background()),
            )
            .with_tooltip(move || tooltip)
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(WorkspaceAction::SshManager(
                    SshManagerAction::TogglePanel,
                ))
            })
            .with_cursor(Cursor::PointingHand)
            .finish()
        };

        let new_host = Hoverable::new(self.mouse_states.new_host.clone(), move |state| {
            let background = if state.is_hovered() {
                internal_colors::accent_overlay_3(theme)
            } else {
                internal_colors::accent_overlay_2(theme)
            };
            Container::new(
                Text::new_inline("+ New host", appearance.ui_font_family(), 12.)
                    .with_color(theme.accent().into_solid())
                    .with_style(Properties::default().weight(Weight::Semibold))
                    .finish(),
            )
            .with_background(background)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(ROW_RADIUS)))
            .with_horizontal_padding(8.)
            .with_vertical_padding(3.)
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(SshProfilesPanelAction::Add))
        .finish();

        Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(8.)
                .with_child(
                    Text::new_inline("SSH hosts", appearance.ui_font_family(), 13.)
                        .with_color(theme.active_ui_text_color().into())
                        .with_style(Properties::default().weight(Weight::Semibold))
                        .finish(),
                )
                .with_child(render_chip(&count.to_string(), appearance))
                .with_child(Expanded::new(1., Empty::new().finish()).finish())
                .with_child(new_host)
                .with_child(close_button)
                .finish(),
        )
        .with_padding_left(PANEL_PADDING + 2.)
        .with_padding_right(PANEL_PADDING)
        .with_vertical_padding(8.)
        .with_border(Border::bottom(1.).with_border_fill(theme.outline()))
        .finish()
    }

    fn render_search_field(&self, appearance: &Appearance) -> Box<dyn Element> {
        Container::new(
            appearance
                .ui_builder()
                .text_input(self.search_editor.clone())
                .with_style(UiComponentStyles {
                    padding: Some(Coords {
                        top: 6.,
                        bottom: 6.,
                        left: 8.,
                        right: 8.,
                    }),
                    background: Some(appearance.theme().background().into()),
                    border_color: Some(appearance.theme().outline().into()),
                    border_width: Some(1.),
                    border_radius: Some(CornerRadius::with_all(Radius::Pixels(FIELD_RADIUS))),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .with_padding_left(PANEL_PADDING + 2.)
        .with_padding_right(PANEL_PADDING + 2.)
        .with_padding_top(10.)
        .with_padding_bottom(6.)
        .finish()
    }

    fn render_filters(
        &self,
        profiles: &[SshHostProfile],
        usage: &SshHostUsage,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let pinned = profiles.iter().filter(|p| usage.is_pinned(p.id)).count();
        let open = profiles
            .iter()
            .filter(|p| SshOpenConnections::open_count(p.id, app) > 0)
            .count();
        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.);
        for (filter, label) in [
            (HostFilter::All, format!("All {}", profiles.len())),
            (HostFilter::Pinned, format!("★ {pinned}")),
            (HostFilter::Open, format!("● Open {open}")),
        ] {
            let active = self.filter == filter;
            let mouse_state = self.mouse_states.filters[&filter].clone();
            row = row.with_child(
                Hoverable::new(mouse_state, move |state| {
                    let (border, text, background) = if active {
                        (
                            theme.accent(),
                            theme.accent().into_solid(),
                            Some(internal_colors::accent_overlay_2(theme)),
                        )
                    } else {
                        (
                            theme.outline(),
                            theme.sub_text_color(theme.background()).into_solid(),
                            state
                                .is_hovered()
                                .then(|| internal_colors::fg_overlay_1(theme)),
                        )
                    };
                    let mut chip = Container::new(
                        Text::new_inline(label.clone(), appearance.ui_font_family(), 11.5)
                            .with_color(text)
                            .finish(),
                    )
                    .with_border(Border::all(1.).with_border_fill(border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(11.)))
                    .with_horizontal_padding(9.)
                    .with_vertical_padding(2.);
                    if let Some(background) = background {
                        chip = chip.with_background(background);
                    }
                    chip.finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(SshProfilesPanelAction::SetFilter(filter))
                })
                .finish(),
            );
        }
        Container::new(row.finish())
            .with_padding_left(PANEL_PADDING + 2.)
            .with_padding_right(PANEL_PADDING + 2.)
            .with_padding_bottom(8.)
            .finish()
    }

    fn render_section_header(
        &self,
        title: &str,
        count: usize,
        group_key: Option<&str>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let collapsed = group_key.is_some_and(|key| self.collapsed_groups.contains(key));
        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.);
        if group_key.is_some() {
            let chevron = if collapsed {
                Icon::ChevronRight
            } else {
                Icon::ChevronDown
            };
            row = row.with_child(
                ConstrainedBox::new(
                    chevron
                        .to_warpui_icon(theme.disabled_ui_text_color())
                        .finish(),
                )
                .with_width(10.)
                .with_height(10.)
                .finish(),
            );
        }
        row = row
            .with_child(
                Text::new_inline(title.to_string(), appearance.ui_font_family(), 11.5)
                    .with_color(theme.sub_text_color(theme.background()).into())
                    .with_style(Properties::default().weight(Weight::Semibold))
                    .finish(),
            )
            .with_child(
                Text::new_inline(count.to_string(), appearance.ui_font_family(), 11.5)
                    .with_color(theme.disabled_ui_text_color().into())
                    .finish(),
            );
        let header = Container::new(row.finish())
            .with_padding_left(PANEL_PADDING + 2.)
            .with_padding_top(9.)
            .with_padding_bottom(5.)
            .finish();
        let Some(group_key) = group_key else {
            return header;
        };
        let group_key = group_key.to_string();
        let mouse_state = self
            .mouse_states
            .groups
            .borrow_mut()
            .entry(group_key.clone())
            .or_default()
            .clone();
        Hoverable::new(mouse_state, move |_| header)
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(SshProfilesPanelAction::ToggleGroup(group_key.clone()))
            })
            .finish()
    }

    fn render_quick_connect_row(
        &self,
        command: String,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        Hoverable::new(self.mouse_states.quick_connect.clone(), move |state| {
            let text = Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_spacing(2.)
                .with_child(
                    Text::new_inline(command.clone(), appearance.monospace_font_family(), 12.)
                        .with_clip(ClipConfig::ellipsis())
                        .with_color(theme.active_ui_text_color().into())
                        .finish(),
                )
                .with_child(
                    Text::new_inline(
                        "Quick connect · connects without saving",
                        appearance.ui_font_family(),
                        11.,
                    )
                    .with_color(theme.sub_text_color(theme.background()).into())
                    .finish(),
                )
                .finish();
            let mut container = Container::new(text)
                .with_border(Border::all(1.).with_border_fill(theme.accent()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                .with_horizontal_padding(10.)
                .with_vertical_padding(8.)
                .with_margin_left(PANEL_PADDING)
                .with_margin_right(PANEL_PADDING)
                .with_margin_top(6.);
            container = container.with_background(if state.is_hovered() {
                internal_colors::accent_overlay_2(theme)
            } else {
                internal_colors::accent_overlay_1(theme)
            });
            container.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(SshProfilesPanelAction::QuickConnect))
        .finish()
    }

    fn render_host_list(
        &self,
        profiles: Vec<SshHostProfile>,
        query: &str,
        usage: &SshHostUsage,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Vec<Box<dyn Element>> {
        let mut children = Vec::new();
        let row = |profile: SshHostProfile, section: RowSection| {
            self.render_profile_row(profile, section, usage, appearance, app)
        };

        if !query.is_empty() {
            let matches = profiles
                .into_iter()
                .filter(|profile| profile.matches_query(query))
                .collect::<Vec<_>>();
            if !matches.is_empty() {
                children.push(self.render_section_header("Hosts", matches.len(), None, appearance));
            }
            children.extend(
                matches
                    .into_iter()
                    .map(|profile| row(profile, RowSection::Search)),
            );
            if let Some(args) = quick_connect_args(query) {
                children.push(self.render_quick_connect_row(format!("ssh {args}"), appearance));
            }
            return children;
        }

        let profiles = match self.filter {
            HostFilter::All => profiles,
            HostFilter::Pinned => profiles
                .into_iter()
                .filter(|profile| usage.is_pinned(profile.id))
                .collect(),
            HostFilter::Open => profiles
                .into_iter()
                .filter(|profile| SshOpenConnections::open_count(profile.id, app) > 0)
                .collect(),
        };
        let find = |id: Uuid| profiles.iter().find(|profile| profile.id == id).cloned();

        if self.filter == HostFilter::All {
            let pinned = usage
                .pinned
                .iter()
                .filter_map(|id| find(*id))
                .collect::<Vec<_>>();
            if !pinned.is_empty() {
                children.push(self.render_section_header(
                    "★ Pinned",
                    pinned.len(),
                    None,
                    appearance,
                ));
                children.extend(
                    pinned
                        .into_iter()
                        .map(|profile| row(profile, RowSection::Pinned)),
                );
            }
            let recent = usage
                .recent_profiles()
                .filter(|(id, _)| !usage.is_pinned(*id))
                .filter_map(|(id, _)| find(id))
                .take(MAX_RECENT_ROWS)
                .collect::<Vec<_>>();
            if !recent.is_empty() {
                children.push(self.render_section_header("Recent", recent.len(), None, appearance));
                children.extend(
                    recent
                        .into_iter()
                        .map(|profile| row(profile, RowSection::Recent)),
                );
            }
        }

        let has_tags = profiles.iter().any(|profile| profile.group().is_some());
        let mut groups = BTreeMap::<(bool, String), Vec<SshHostProfile>>::new();
        for profile in profiles {
            let key = match profile.group() {
                Some(group) => (false, group.to_string()),
                None if has_tags => (true, "Ungrouped".to_string()),
                None => (true, "All hosts".to_string()),
            };
            groups.entry(key).or_default().push(profile);
        }
        for ((_, title), rows) in groups {
            children.push(self.render_section_header(&title, rows.len(), Some(&title), appearance));
            if self.collapsed_groups.contains(&title) {
                continue;
            }
            children.extend(
                rows.into_iter()
                    .map(|profile| row(profile, RowSection::Group)),
            );
        }
        children
    }

    fn render_row_action(
        icon: Icon,
        mouse_state: MouseStateHandle,
        action: SshProfilesPanelAction,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let color = theme.sub_text_color(theme.background());
        Hoverable::new(mouse_state, move |state| {
            let mut button = Container::new(
                ConstrainedBox::new(icon.to_warpui_icon(color).finish())
                    .with_width(ROW_ACTION_BUTTON_ICON_SIZE)
                    .with_height(ROW_ACTION_BUTTON_ICON_SIZE)
                    .finish(),
            )
            .with_padding(Padding::uniform(ROW_ACTION_BUTTON_PADDING))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(ROW_RADIUS)));
            if state.is_hovered() {
                button = button.with_background(internal_colors::fg_overlay_3(theme));
            }
            button.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .finish()
    }

    fn render_checkbox(
        checked: bool,
        mouse_state: MouseStateHandle,
        profile_id: Uuid,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        Hoverable::new(mouse_state, move |_| {
            let mut box_ = Container::new(if checked {
                Icon::Check.to_warpui_icon(theme.background()).finish()
            } else {
                Empty::new().finish()
            })
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.)))
            .with_padding(Padding::uniform(1.5));
            box_ = if checked {
                box_.with_background(theme.accent())
                    .with_border(Border::all(1.).with_border_fill(theme.accent()))
            } else {
                box_.with_border(Border::all(1.).with_border_fill(theme.outline()))
            };
            ConstrainedBox::new(box_.finish())
                .with_width(CHECKBOX_SIZE)
                .with_height(CHECKBOX_SIZE)
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(SshProfilesPanelAction::ToggleSelected(profile_id))
        })
        .finish()
    }

    fn render_profile_row(
        &self,
        profile: SshHostProfile,
        section: RowSection,
        usage: &SshHostUsage,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let mouse_states = self
            .row_mouse_states
            .borrow_mut()
            .entry((section, profile.id))
            .or_default()
            .clone();
        let profile_id = profile.id;
        let is_pinned = usage.is_pinned(profile_id);
        let checked = self.selected.contains(&profile_id);
        let selecting = !self.selected.is_empty();
        let open_count = SshOpenConnections::open_count(profile_id, app);
        let status = if open_count > 0 {
            Some((true, format!("Open ×{open_count}")))
        } else {
            usage
                .last_connected(profile_id)
                .map(|at| (false, format_relative_time(at, Utc::now())))
        };
        let color = host_color(&profile)
            .map(|color| ansi_color(color, appearance))
            .unwrap_or_else(|| internal_colors::fg_overlay_4(theme).into_solid());
        let action_buttons_mouse_over = mouse_states
            .actions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_mouse_over_element();

        let row = Hoverable::new(mouse_states.row.clone(), move |state| {
            let hovered = state.is_hovered() || action_buttons_mouse_over;
            let name = if is_pinned && section != RowSection::Pinned {
                format!("{} ★", profile.name)
            } else {
                profile.name.clone()
            };
            let text = Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_spacing(2.)
                .with_child(
                    Text::new_inline(name, appearance.ui_font_family(), 13.)
                        .with_clip(ClipConfig::ellipsis())
                        .with_color(theme.active_ui_text_color().into())
                        .finish(),
                )
                .with_child(
                    Text::new_inline(
                        host_subtitle(&profile),
                        appearance.monospace_font_family(),
                        11.,
                    )
                    .with_clip(ClipConfig::ellipsis())
                    .with_color(theme.sub_text_color(theme.background()).into())
                    .finish(),
                )
                .finish();

            let mut right = Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::End)
                .with_spacing(3.);
            if !profile.tags.is_empty() {
                let mut tags = Flex::row().with_spacing(4.);
                for tag in profile.tags.iter().take(2) {
                    tags = tags.with_child(render_chip(tag, appearance));
                }
                right = right.with_child(tags.finish());
            }
            if let Some((open, label)) = status.clone() {
                let mut line = Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(5.);
                if open {
                    line = line.with_child(render_dot(
                        ansi_color(AnsiColorIdentifier::Green, appearance),
                        7.,
                    ));
                }
                right = right.with_child(
                    line.with_child(
                        Text::new_inline(label, appearance.ui_font_family(), 11.)
                            .with_color(theme.sub_text_color(theme.background()).into())
                            .finish(),
                    )
                    .finish(),
                );
            }

            let mut content = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(ROW_SPACING);
            if selecting || hovered {
                content = content.with_child(Self::render_checkbox(
                    checked,
                    mouse_states.checkbox.clone(),
                    profile_id,
                    appearance,
                ));
            }
            content = content
                .with_child(render_color_square(color))
                .with_child(Expanded::new(1., text).finish())
                .with_child(right.finish());

            let mut container = Container::new(content.finish())
                .with_padding_left(PANEL_PADDING + 4.)
                .with_padding_right(PANEL_PADDING + 2.)
                .with_vertical_padding(6.);
            if checked {
                container = container.with_background(internal_colors::accent_overlay_2(theme));
            } else if hovered || state.is_clicked() {
                container = container.with_background(internal_colors::fg_overlay_1(theme));
            }
            let mut stack = Stack::new().with_child(container.finish());

            if hovered && !selecting {
                let actions = Flex::row()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(ROW_ACTION_BUTTON_GAP)
                    .with_child(Self::render_row_action(
                        Icon::Play,
                        mouse_states.connect.clone(),
                        SshProfilesPanelAction::Connect(profile_id),
                        appearance,
                    ))
                    .with_child(Self::render_row_action(
                        if is_pinned {
                            Icon::PinFilled
                        } else {
                            Icon::Pin
                        },
                        mouse_states.pin.clone(),
                        SshProfilesPanelAction::TogglePinned(profile_id),
                        appearance,
                    ))
                    .with_child(Self::render_row_action(
                        Icon::Pencil,
                        mouse_states.edit.clone(),
                        SshProfilesPanelAction::Edit(profile_id),
                        appearance,
                    ))
                    .with_child(Self::render_row_action(
                        Icon::Trash,
                        mouse_states.remove.clone(),
                        SshProfilesPanelAction::Remove(profile_id),
                        appearance,
                    ))
                    .finish();
                let belt_border_color = internal_colors::neutral_4(theme);
                let actions = Hoverable::new(mouse_states.actions.clone(), move |_| {
                    Container::new(actions)
                        .with_background(ThemeFill::Solid(internal_colors::neutral_3(theme)))
                        .with_border(
                            Border::all(1.).with_border_fill(ThemeFill::Solid(belt_border_color)),
                        )
                        .with_padding(Padding::uniform(ROW_ACTION_BUTTON_PADDING))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(ROW_RADIUS)))
                        .finish()
                })
                .with_defer_events_to_children()
                .finish();
                stack.add_positioned_overlay_child(
                    actions,
                    OffsetPositioning::offset_from_parent(
                        vec2f(-8., 0.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::MiddleRight,
                        ChildAnchor::MiddleRight,
                    ),
                );
            }
            stack.finish()
        });

        row.with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                if selecting {
                    ctx.dispatch_typed_action(SshProfilesPanelAction::ToggleSelected(profile_id));
                } else {
                    ctx.dispatch_typed_action(WorkspaceAction::SshManager(
                        SshManagerAction::Connect(profile_id),
                    ));
                }
            })
            .with_defer_events_to_children()
            .finish()
    }

    fn render_selection_bar(
        &self,
        profiles: &[SshHostProfile],
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let names = self
            .selected
            .iter()
            .filter_map(|id| profiles.iter().find(|profile| profile.id == *id))
            .map(|profile| profile.name.as_str())
            .collect::<Vec<_>>()
            .join(" · ");
        let count = self.selected.len();

        let clear = Hoverable::new(self.mouse_states.clear_selection.clone(), move |state| {
            let color = if state.is_hovered() {
                theme.active_ui_text_color()
            } else {
                theme.sub_text_color(theme.background())
            };
            Text::new_inline("Clear", appearance.ui_font_family(), 12.)
                .with_color(color.into())
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(SshProfilesPanelAction::ClearSelection))
        .finish();

        let open = Hoverable::new(self.mouse_states.open_selected.clone(), move |state| {
            let background = if state.is_hovered() {
                internal_colors::accent_overlay_4(theme)
            } else {
                theme.accent()
            };
            Container::new(
                Text::new_inline(
                    format!("Open {count} hosts in new tabs"),
                    appearance.ui_font_family(),
                    12.5,
                )
                .with_color(theme.background().into_solid())
                .with_style(Properties::default().weight(Weight::Semibold))
                .finish(),
            )
            .with_background(background)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(5.)))
            .with_vertical_padding(6.)
            .with_horizontal_padding(12.)
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(SshProfilesPanelAction::OpenSelected))
        .finish();

        Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_spacing(8.)
                .with_child(
                    Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_spacing(8.)
                        .with_child(
                            Text::new_inline(
                                format!("{count} selected"),
                                appearance.ui_font_family(),
                                12.5,
                            )
                            .with_color(theme.active_ui_text_color().into())
                            .with_style(Properties::default().weight(Weight::Semibold))
                            .finish(),
                        )
                        .with_child(
                            Shrinkable::new(
                                1.,
                                Text::new_inline(names, appearance.ui_font_family(), 11.5)
                                    .with_clip(ClipConfig::ellipsis())
                                    .with_color(theme.disabled_ui_text_color().into())
                                    .finish(),
                            )
                            .finish(),
                        )
                        .with_child(clear)
                        .finish(),
                )
                .with_child(open)
                .finish(),
        )
        .with_background(internal_colors::fg_overlay_1(theme))
        .with_border(Border::top(1.).with_border_fill(theme.outline()))
        .with_uniform_padding(PANEL_PADDING + 2.)
        .finish()
    }
}

fn render_color_square(color: pathfinder_color::ColorU) -> Box<dyn Element> {
    ConstrainedBox::new(
        Container::new(Empty::new().finish())
            .with_background(ThemeFill::Solid(color))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.)))
            .finish(),
    )
    .with_width(COLOR_DOT_SIZE)
    .with_height(COLOR_DOT_SIZE)
    .finish()
}

impl Entity for SshProfilesPanelView {
    type Event = SshProfilesPanelEvent;
}

impl TypedActionView for SshProfilesPanelView {
    type Action = SshProfilesPanelAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SshProfilesPanelAction::Add => self.open_add_modal(ctx),
            SshProfilesPanelAction::Connect(id) => ctx.dispatch_typed_action(
                &WorkspaceAction::SshManager(SshManagerAction::Connect(*id)),
            ),
            SshProfilesPanelAction::Edit(id) => self.open_edit_modal(*id, ctx),
            SshProfilesPanelAction::Remove(id) => self.open_delete_modal(*id, ctx),
            SshProfilesPanelAction::TogglePinned(id) => self.toggle_pinned(*id, ctx),
            SshProfilesPanelAction::QuickConnect => self.quick_connect(ctx),
            SshProfilesPanelAction::SetFilter(filter) => {
                self.filter = *filter;
                ctx.notify();
            }
            SshProfilesPanelAction::ToggleGroup(group) => {
                if !self.collapsed_groups.remove(group) {
                    self.collapsed_groups.insert(group.clone());
                }
                ctx.notify();
            }
            SshProfilesPanelAction::ToggleSelected(id) => self.toggle_selected(*id, ctx),
            SshProfilesPanelAction::ClearSelection => {
                self.selected.clear();
                ctx.notify();
            }
            SshProfilesPanelAction::OpenSelected => self.open_selected(ctx),
        }
    }
}

impl View for SshProfilesPanelView {
    fn ui_name() -> &'static str {
        "SshProfilesPanelView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let profiles = Self::profiles(app);
        let query = self.search_query(app);
        let usage = SshSettings::as_ref(app).ssh_host_usage.value();

        let mut content = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(self.render_header(profiles.len(), appearance, app))
            .with_child(self.render_search_field(appearance));
        if query.is_empty() {
            content = content.with_child(self.render_filters(&profiles, usage, appearance, app));
        }

        if !self.notices.is_empty() {
            let mut banner = Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_spacing(NOTICE_SPACING);
            for notice in &self.notices {
                banner = banner.with_child(SshProfileModalView::render_error_text(
                    notice.clone(),
                    appearance,
                ));
            }
            content = content.with_child(
                Container::new(banner.finish())
                    .with_padding(Padding::uniform(PANEL_PADDING))
                    .finish(),
            );
        }

        if profiles.is_empty() && query.is_empty() {
            content = content.with_child(
                Container::new(
                    Text::new(
                        "No saved hosts yet. Add one, or search with user@host to connect.",
                        appearance.ui_font_family(),
                        12.,
                    )
                    .with_color(theme.sub_text_color(theme.background()).into())
                    .finish(),
                )
                .with_padding(Padding::uniform(PANEL_PADDING + 2.))
                .finish(),
            );
        }

        let selection_bar =
            (!self.selected.is_empty()).then(|| self.render_selection_bar(&profiles, appearance));
        let list = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_children(self.render_host_list(profiles, &query, usage, appearance, app));
        let scrollable_list = ClippedScrollable::vertical(
            self.list_scroll_state.clone(),
            Container::new(list.finish())
                .with_padding_bottom(PANEL_PADDING)
                .with_border(Border::top(1.).with_border_fill(theme.outline()))
                .finish(),
            ScrollbarWidth::Custom(SCROLLBAR_WIDTH),
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            ElementFill::None,
        )
        .with_overlayed_scrollbar()
        .finish();

        content = content.with_child(Shrinkable::new(1., scrollable_list).finish());
        if let Some(selection_bar) = selection_bar {
            content = content.with_child(selection_bar);
        }

        ConstrainedBox::new(
            Container::new(content.finish())
                .with_background(theme.background())
                .with_border(Border::right(1.).with_border_fill(theme.outline()))
                .finish(),
        )
        .with_width(PANEL_WIDTH)
        .finish()
    }
}

fn sync_profiles_with_ssh_config(ctx: &mut ViewContext<SshProfilesPanelView>) -> Vec<String> {
    let mut notices = Vec::new();
    let cached = SshSettings::as_ref(ctx)
        .saved_ssh_host_profiles
        .value()
        .clone();

    let mut profiles = match config_file::load_profiles() {
        Ok(profiles) => profiles,
        Err(err) => {
            log::error!("Failed to read SSH profiles from ~/.ssh/config: {err:#}");
            return vec![failure_notice(
                "read your SSH profiles from ~/.ssh/config",
                &err,
            )];
        }
    };

    let mut written_ids = profiles
        .iter()
        .map(|profile| profile.id)
        .collect::<HashSet<_>>();
    let mut written_names = profiles
        .iter()
        .map(|profile| profile.name.clone())
        .collect::<HashSet<_>>();
    let mut migrated = false;
    for profile in &cached {
        if written_ids.contains(&profile.id) {
            continue;
        }
        if written_names.contains(&profile.name) {
            log::warn!(
                "Not migrating cached SSH profile {} to ~/.ssh/config: the host \"{}\" is already taken",
                profile.id,
                profile.name
            );
            continue;
        }
        match config_file::upsert_profile(profile) {
            Ok(()) => {
                written_ids.insert(profile.id);
                written_names.insert(profile.name.clone());
                migrated = true;
            }
            Err(err) => {
                log::error!("Failed to migrate SSH profile to ~/.ssh/config: {err:#}");
                notices.push(failure_notice(
                    &format!("write the profile \"{}\" to ~/.ssh/config", profile.name),
                    &err,
                ));
            }
        }
    }

    if migrated {
        profiles = match config_file::load_profiles() {
            Ok(profiles) => profiles,
            Err(err) => {
                log::error!("Failed to re-read SSH profiles from ~/.ssh/config: {err:#}");
                notices.push(failure_notice(
                    "re-read your SSH profiles from ~/.ssh/config",
                    &err,
                ));
                return notices;
            }
        };
    }

    let reassignments = normalize_ssh_host_profiles(&mut profiles);
    let mut ids_recorded = true;
    if !reassignments.is_empty() {
        let ids = profiles
            .iter()
            .map(|profile| profile.id)
            .collect::<Vec<_>>();
        if let Err(err) = config_file::rewrite_profile_ids(&ids) {
            log::error!("Failed to write the new SSH profile ids to ~/.ssh/config: {err:#}");
            notices.push(failure_notice(
                "record the new profile ids in ~/.ssh/config",
                &err,
            ));
            ids_recorded = false;
        }
    }
    if let Some(notice) =
        discard_secrets_of_reassigned_ids(&reassignments, &profiles, ids_recorded, ctx)
    {
        notices.push(notice);
    }

    if profiles != cached {
        let mut cache_error = None;
        SshSettings::handle(ctx).update(ctx, |settings, ctx| {
            if let Err(err) = settings.saved_ssh_host_profiles.set_value(profiles, ctx) {
                log::error!("Failed to cache SSH profiles: {err:#}");
                cache_error = Some(err);
            }
        });
        if let Some(err) = cache_error {
            notices.push(failure_notice(
                "save your SSH profile list to Warp's settings",
                &err,
            ));
        }
    }

    notices
}

fn failure_notice(action: &str, err: &anyhow::Error) -> String {
    let mut notice = format!("Warp could not {action}: {err:#}");
    if is_permission_denied(err) {
        notice.push_str(PERMISSION_HINT);
    }
    notice
}

fn is_permission_denied(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|err| err.kind() == std::io::ErrorKind::PermissionDenied)
    })
}

fn discard_secrets_of_reassigned_ids(
    reassignments: &[SshProfileIdReassignment],
    profiles: &[SshHostProfile],
    ids_recorded: bool,
    ctx: &mut ViewContext<SshProfilesPanelView>,
) -> Option<String> {
    if reassignments.is_empty() {
        return None;
    }

    let mut orphans_left_behind = false;
    for reassignment in reassignments {
        log::warn!(
            "Regenerated invalid or duplicate SSH profile id: old_id={}, new_id={}",
            reassignment.previous_id,
            reassignment.new_id
        );
        if profiles
            .iter()
            .any(|profile| profile.id == reassignment.previous_id)
        {
            continue;
        }
        match ctx
            .secure_storage()
            .remove_value(&ssh_profile_password_key(reassignment.previous_id))
        {
            Ok(()) | Err(warpui_extras::secure_storage::Error::NotFound) => {}
            Err(err) => {
                log::error!(
                    "Failed to discard the password stored under SSH profile id {}: {err:#}",
                    reassignment.previous_id
                );
                orphans_left_behind = true;
            }
        }
    }

    let mut notice = format!(
        "{} SSH profile(s) in ~/.ssh/config had a missing or duplicate id and were given a new one.",
        reassignments.len()
    );
    notice.push_str(match ids_recorded {
        true => " Open each of them and save the password again.",
        false => " Their ids will change again on every launch until ~/.ssh/config can be updated.",
    });
    if orphans_left_behind {
        notice.push_str(" A password stored under an old id could not be deleted from the keychain — see the log.");
    }
    Some(notice)
}

fn warp_owned_profile_ids() -> anyhow::Result<HashSet<Uuid>> {
    Ok(config_file::load_owned_profile_ids()?.into_iter().collect())
}

pub(super) fn jump_host_points_at(jump_host: &SshJumpHost, profile: &SshHostProfile) -> bool {
    match jump_host.source_profile_id {
        Some(source_profile_id) => source_profile_id == profile.id,
        None => {
            jump_host.host == profile.host
                && jump_host.user == profile.user
                && jump_host.port == profile.port
                && jump_host.identity_file == profile.identity_file
        }
    }
}
