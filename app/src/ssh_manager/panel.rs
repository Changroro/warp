use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::PoisonError;

use pathfinder_geometry::vector::vec2f;
use settings::Setting as _;
use uuid::Uuid;
use warp_core::ui::theme::color::internal_colors;
use warp_editor::editor::NavigationKey;
use warpui::elements::{
    Border, ChildAnchor, ChildView, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox,
    Container, CornerRadius, CrossAxisAlignment, Expanded, Fill as ElementFill, Flex, Hoverable,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, Padding, ParentAnchor,
    ParentElement, ParentOffsetBounds, Radius, ScrollbarWidth, Shrinkable, Stack, Text, Wrap,
};
use warpui::fonts::{Properties, Weight};
use warpui::platform::Cursor;
use warpui::text_layout::ClipConfig;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};
use warpui_extras::secure_storage::AppContextExt as _;
use zeroize::Zeroizing;

use crate::appearance::Appearance;
use crate::editor::{EditorView, Event as EditorEvent, SingleLineEditorOptions, TextOptions};
use crate::modal::{Modal, ModalEvent, ModalViewState};
use crate::settings::SshSettings;
use crate::ssh_manager::profile::{
    SshHostProfile, SshJumpHost, SshProfileIdReassignment, default_ssh_port,
    normalize_ssh_host_profiles, ssh_profile_password_key,
};
use crate::ssh_manager::{SshManagerAction, TOGGLE_PANEL_BINDING_NAME, config_file};
use crate::themes::theme::Fill as ThemeFill;
use crate::ui_components::buttons::{icon_button, icon_button_with_color};
use crate::ui_components::icons::Icon;
use crate::util::bindings::keybinding_name_to_display_string;
use crate::view_components::action_button::{ActionButton, PrimaryTheme, SecondaryTheme};
use crate::view_components::{Dropdown, DropdownItem};
use crate::workspace::WorkspaceAction;

pub(crate) const PANEL_WIDTH: f32 = 280.;
const PANEL_PADDING: f32 = 10.;
const ROW_WIDTH: f32 = PANEL_WIDTH - PANEL_PADDING * 2.;
const ROW_RADIUS: f32 = 4.;
const ROW_GAP: f32 = 4.;
const ROW_PADDING: f32 = 8.;
const ROW_SPACING: f32 = 8.;
const ROW_ICON_SIZE: f32 = 16.;
const ROW_ICON_PADDING: f32 = 8.;
const ROW_ICON_RADIUS: f32 = 24.;
const ROW_ACTION_BUTTON_ICON_SIZE: f32 = 12.;
const ROW_ACTION_BUTTON_PADDING: f32 = 2.;
const ROW_ACTION_BUTTON_GAP: f32 = 2.;
const SCROLLBAR_WIDTH: f32 = 4.;
const JUMP_HOST_DROPDOWN_WIDTH: f32 = 460.;
const PROFILE_MODAL_WIDTH: f32 = 520.;
const MODAL_MAX_HEIGHT_PERCENTAGE: f32 = 0.8;
const DELETE_MODAL_HEIGHT: f32 = 220.;
const DELETE_MODAL_BODY_HEIGHT: f32 = 150.;
const FIELD_RADIUS: f32 = 6.;
const FIELD_LABEL_SPACING: f32 = 5.;
const EDITOR_COUNT: usize = 7;
const LABEL_FORBIDDEN_CHARACTERS: [char; 6] = ['#', '*', '?', '!', '"', '\''];
const NOTICE_SPACING: f32 = 6.;
const PERMISSION_HINT: &str =
    " Check that you can read and write ~/.ssh/config and that ~/.ssh is yours.";

#[derive(Debug, Clone)]
pub enum SshProfilesPanelEvent {
    ModalToggled,
}

#[derive(Debug, Clone)]
pub enum SshProfilesPanelAction {
    Add,
    Edit(Uuid),
    Remove(Uuid),
}

#[derive(Default, Clone)]
struct ProfileRowMouseStates {
    row: MouseStateHandle,
    actions: MouseStateHandle,
    edit: MouseStateHandle,
    remove: MouseStateHandle,
}

pub struct SshProfilesPanelView {
    modal_state: ModalViewState<Modal<SshProfileModalView>>,
    delete_modal_state: ModalViewState<Modal<SshProfileDeleteModalView>>,
    row_mouse_states: RefCell<HashMap<Uuid, ProfileRowMouseStates>>,
    add_row_mouse_state: MouseStateHandle,
    close_button_mouse_state: MouseStateHandle,
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

        Self {
            modal_state: ModalViewState::new(modal),
            delete_modal_state: ModalViewState::new(delete_modal),
            row_mouse_states: Default::default(),
            add_row_mouse_state: Default::default(),
            close_button_mouse_state: Default::default(),
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
            .retain(|id, _| ids.contains(id));
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
        if let SshProfileModalMode::Edit(id) = mode {
            profile.id = id;
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
            .write_value(&key, password)
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
        self.row_mouse_states.borrow_mut().remove(&profile_id);
        ctx.notify();

        self.remove_profile_password(profile_id, ctx)
            .map_err(|err| {
                format!(
                    "\"{}\" was deleted, but its stored password is still there: {err}",
                    profile.name
                )
            })
    }

    fn render_header(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
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
                self.close_button_mouse_state.clone(),
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

        Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Expanded::new(
                        1.,
                        Text::new("SSH Profiles", appearance.ui_font_family(), 13.)
                            .with_color(theme.active_ui_text_color().into())
                            .with_style(Properties::default().weight(Weight::Medium))
                            .finish(),
                    )
                    .finish(),
                )
                .with_child(close_button)
                .finish(),
        )
        .with_padding_left(PANEL_PADDING)
        .with_padding_right(PANEL_PADDING)
        .with_padding_top(10.)
        .with_padding_bottom(6.)
        .finish()
    }

    fn render_empty_state(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        Container::new(
            Text::new("No saved profiles yet.", appearance.ui_font_family(), 12.)
                .with_color(theme.sub_text_color(theme.background()).into())
                .finish(),
        )
        .with_padding(Padding::uniform(12.))
        .finish()
    }

    fn render_add_row(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let mouse_state = self.add_row_mouse_state.clone();

        Hoverable::new(mouse_state, move |state| {
            let icon_color = theme.main_text_color(theme.background());
            let icon = Container::new(
                ConstrainedBox::new(Icon::Plus.to_warpui_icon(icon_color).finish())
                    .with_width(ROW_ICON_SIZE)
                    .with_height(ROW_ICON_SIZE)
                    .finish(),
            )
            .with_padding(Padding::uniform(ROW_ICON_PADDING))
            .with_background(internal_colors::fg_overlay_2(theme))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(ROW_ICON_RADIUS)))
            .finish();

            let row_content = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(ROW_SPACING)
                .with_child(icon)
                .with_child(
                    Text::new_inline("Add SSH profile", appearance.ui_font_family(), 12.)
                        .with_clip(ClipConfig::ellipsis())
                        .with_color(theme.active_ui_text_color().into())
                        .finish(),
                )
                .finish();

            let mut container = Container::new(row_content)
                .with_padding(Padding::uniform(ROW_PADDING))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(ROW_RADIUS)));

            if state.is_hovered() || state.is_clicked() {
                container = container.with_background(internal_colors::fg_overlay_1(theme));
            }

            ConstrainedBox::new(container.finish())
                .with_width(ROW_WIDTH)
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(SshProfilesPanelAction::Add))
        .finish()
    }

    fn render_profile_row(
        &self,
        profile: SshHostProfile,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let mouse_states = self
            .row_mouse_states
            .borrow_mut()
            .entry(profile.id)
            .or_default()
            .clone();
        let profile_id = profile.id;
        let subtitle = if profile.port == default_ssh_port() {
            profile.target()
        } else {
            format!("{}:{}", profile.target(), profile.port)
        };
        let action_buttons_mouse_over = mouse_states
            .actions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_mouse_over_element();

        let mut row = Hoverable::new(mouse_states.row.clone(), move |state| {
            let profile_icon = Container::new(
                ConstrainedBox::new(
                    Icon::Globe
                        .to_warpui_icon(theme.main_text_color(theme.background()))
                        .finish(),
                )
                .with_width(ROW_ICON_SIZE)
                .with_height(ROW_ICON_SIZE)
                .finish(),
            )
            .with_padding(Padding::uniform(ROW_ICON_PADDING))
            .with_background(internal_colors::fg_overlay_2(theme))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(ROW_ICON_RADIUS)))
            .finish();

            let mut text_column = Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_spacing(2.)
                .with_child(
                    Text::new_inline(profile.name.clone(), appearance.ui_font_family(), 12.)
                        .with_clip(ClipConfig::ellipsis())
                        .with_color(theme.active_ui_text_color().into())
                        .finish(),
                )
                .with_child(
                    Text::new_inline(subtitle.clone(), appearance.ui_font_family(), 12.)
                        .with_clip(ClipConfig::ellipsis())
                        .with_color(theme.sub_text_color(theme.background()).into())
                        .finish(),
                );

            if !profile.tags.is_empty() {
                text_column = text_column.with_child(
                    Text::new(profile.tags.join(", "), appearance.ui_font_family(), 10.)
                        .with_color(theme.disabled_ui_text_color().into())
                        .finish(),
                );
            }

            let row_content = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(ROW_SPACING)
                .with_child(profile_icon)
                .with_child(Shrinkable::new(1., text_column.finish()).finish())
                .finish();

            let background = (state.is_hovered() || state.is_clicked())
                .then(|| internal_colors::fg_overlay_1(theme));

            let mut container = Container::new(row_content)
                .with_padding(Padding::uniform(ROW_PADDING))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(ROW_RADIUS)));

            if let Some(background) = background {
                container = container.with_background(background);
            }

            let row_body = ConstrainedBox::new(container.finish())
                .with_width(ROW_WIDTH)
                .finish();
            let mut stack = Stack::new().with_child(row_body);

            if state.is_hovered() || action_buttons_mouse_over {
                let meta_color = theme.sub_text_color(theme.background());
                let edit_button = Hoverable::new(mouse_states.edit.clone(), move |button_state| {
                    let mut button = Container::new(
                        ConstrainedBox::new(Icon::DotsVertical.to_warpui_icon(meta_color).finish())
                            .with_width(ROW_ACTION_BUTTON_ICON_SIZE)
                            .with_height(ROW_ACTION_BUTTON_ICON_SIZE)
                            .finish(),
                    )
                    .with_padding(Padding::uniform(ROW_ACTION_BUTTON_PADDING))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(ROW_RADIUS)));
                    if button_state.is_hovered() {
                        button = button.with_background(internal_colors::fg_overlay_2(theme));
                    }
                    button.finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(SshProfilesPanelAction::Edit(profile_id));
                })
                .finish();

                let remove_button =
                    Hoverable::new(mouse_states.remove.clone(), move |button_state| {
                        let mut button = Container::new(
                            ConstrainedBox::new(Icon::Trash.to_warpui_icon(meta_color).finish())
                                .with_width(ROW_ACTION_BUTTON_ICON_SIZE)
                                .with_height(ROW_ACTION_BUTTON_ICON_SIZE)
                                .finish(),
                        )
                        .with_padding(Padding::uniform(ROW_ACTION_BUTTON_PADDING))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(ROW_RADIUS)));
                        if button_state.is_hovered() {
                            button = button.with_background(internal_colors::fg_overlay_3(theme));
                        }
                        button.finish()
                    })
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(SshProfilesPanelAction::Remove(profile_id));
                    })
                    .finish();

                let action_buttons = Flex::row()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(ROW_ACTION_BUTTON_GAP)
                    .with_child(edit_button)
                    .with_child(remove_button)
                    .finish();

                let belt_border_color = internal_colors::neutral_4(theme);
                let actions = Hoverable::new(mouse_states.actions.clone(), move |_| {
                    Container::new(action_buttons)
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
                        vec2f(-4., -4.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::TopRight,
                        ChildAnchor::TopRight,
                    ),
                );
            }

            stack.finish()
        });

        row = row.on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(WorkspaceAction::SshManager(SshManagerAction::Connect(
                profile_id,
            )));
        });
        row.with_defer_events_to_children().finish()
    }
}

impl Entity for SshProfilesPanelView {
    type Event = SshProfilesPanelEvent;
}

impl TypedActionView for SshProfilesPanelView {
    type Action = SshProfilesPanelAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SshProfilesPanelAction::Add => self.open_add_modal(ctx),
            SshProfilesPanelAction::Edit(id) => self.open_edit_modal(*id, ctx),
            SshProfilesPanelAction::Remove(id) => self.open_delete_modal(*id, ctx),
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

        let mut content = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(self.render_header(appearance, app));

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

        if profiles.is_empty() {
            content = content.with_child(self.render_empty_state(appearance));
        }

        let mut list = Flex::column()
            .with_spacing(ROW_GAP)
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        for profile in profiles {
            list = list.with_child(self.render_profile_row(profile, appearance));
        }
        let scrollable_list = ClippedScrollable::vertical(
            self.list_scroll_state.clone(),
            Container::new(list.finish())
                .with_padding_left(PANEL_PADDING)
                .with_padding_right(PANEL_PADDING)
                .finish(),
            ScrollbarWidth::Custom(SCROLLBAR_WIDTH),
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            ElementFill::None,
        )
        .with_overlayed_scrollbar()
        .finish();

        content = content.with_child(Shrinkable::new(1., scrollable_list).finish());
        content = content.with_child(
            Container::new(self.render_add_row(appearance))
                .with_padding_left(PANEL_PADDING)
                .with_padding_right(PANEL_PADDING)
                .with_padding_top(PANEL_PADDING)
                .with_padding_bottom(PANEL_PADDING)
                .finish(),
        );

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SshProfileModalMode {
    Add,
    Edit(Uuid),
}

#[derive(Clone)]
pub enum PasswordIntent {
    Keep,
    Set(Zeroizing<String>),
    Clear,
}

impl std::fmt::Debug for PasswordIntent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Keep => formatter.write_str("Keep"),
            Self::Set(_) => formatter.write_str("Set(<redacted>)"),
            Self::Clear => formatter.write_str("Clear"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum SshProfileModalEvent {
    Cancel,
    Submit {
        mode: SshProfileModalMode,
        profile: Box<SshHostProfile>,
        password: PasswordIntent,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SshProfileModalAction {
    Cancel,
    Save,
    TogglePasswordVisibility,
    ClearPassword,
    JumpHostDropdownPlaceholder,
    AddJumpHost(Uuid),
    RemoveJumpHost(usize),
}

pub struct SshProfileModalView {
    mode: SshProfileModalMode,
    draft_id: Uuid,
    name_editor: ViewHandle<EditorView>,
    host_editor: ViewHandle<EditorView>,
    user_editor: ViewHandle<EditorView>,
    port_editor: ViewHandle<EditorView>,
    password_editor: ViewHandle<EditorView>,
    identity_file_editor: ViewHandle<EditorView>,
    tags_editor: ViewHandle<EditorView>,
    jump_host_dropdown: ViewHandle<Dropdown<SshProfileModalAction>>,
    available_profiles: Vec<SshHostProfile>,
    selected_jump_hosts: Vec<SshJumpHost>,
    password_clear: bool,
    had_stored_password: bool,
    error: Option<String>,
    cancel_button: ViewHandle<ActionButton>,
    save_button: ViewHandle<ActionButton>,
    password_eye_mouse_state: MouseStateHandle,
    password_clear_mouse_state: MouseStateHandle,
    jump_host_chip_mouse_states: RefCell<HashMap<usize, MouseStateHandle>>,
    form_scroll_state: ClippedScrollStateHandle,
}

impl SshProfileModalView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let name_editor = Self::editor("Label", false, ctx);
        let host_editor = Self::editor("Host", false, ctx);
        let user_editor = Self::editor("Username", false, ctx);
        let port_editor = Self::editor("22", false, ctx);
        let password_editor = Self::editor("Password", true, ctx);
        let identity_file_editor = Self::editor("~/.ssh/id_ed25519", false, ctx);
        let tags_editor = Self::editor("prod, db", false, ctx);
        let cancel_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Cancel", SecondaryTheme)
                .with_width(88.)
                .on_click(|ctx| ctx.dispatch_typed_action(SshProfileModalAction::Cancel))
        });
        let save_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Save", PrimaryTheme)
                .with_width(88.)
                .on_click(|ctx| ctx.dispatch_typed_action(SshProfileModalAction::Save))
        });
        let jump_host_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(JUMP_HOST_DROPDOWN_WIDTH);
            dropdown.set_menu_width(JUMP_HOST_DROPDOWN_WIDTH, ctx);
            dropdown.set_menu_max_height(180., ctx);
            dropdown.set_top_bar_height(32., ctx);
            dropdown.set_menu_header_text_override(|_| "Add jump host".to_string());
            dropdown
        });

        for (index, editor) in [
            &name_editor,
            &host_editor,
            &user_editor,
            &port_editor,
            &password_editor,
            &identity_file_editor,
            &tags_editor,
        ]
        .into_iter()
        .enumerate()
        {
            ctx.subscribe_to_view(editor, move |me, _, event, ctx| {
                me.handle_editor_event(index, event, ctx);
            });
        }

        Self {
            mode: SshProfileModalMode::Add,
            draft_id: Uuid::new_v4(),
            name_editor,
            host_editor,
            user_editor,
            port_editor,
            password_editor,
            identity_file_editor,
            tags_editor,
            jump_host_dropdown,
            available_profiles: Vec::new(),
            selected_jump_hosts: Vec::new(),
            password_clear: false,
            had_stored_password: false,
            error: None,
            cancel_button,
            save_button,
            password_eye_mouse_state: Default::default(),
            password_clear_mouse_state: Default::default(),
            jump_host_chip_mouse_states: Default::default(),
            form_scroll_state: Default::default(),
        }
    }

    fn editor(
        placeholder: &'static str,
        is_password: bool,
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<EditorView> {
        let editor = ctx.add_view(|ctx| {
            EditorView::single_line(
                SingleLineEditorOptions {
                    text: TextOptions::ui_text(None, Appearance::as_ref(ctx)),
                    soft_wrap: true,
                    is_password,
                    ..Default::default()
                },
                ctx,
            )
        });
        editor.update(ctx, |editor, ctx| {
            editor.set_placeholder_text(placeholder, ctx);
        });
        editor
    }

    pub fn open_add(&mut self, profiles: Vec<SshHostProfile>, ctx: &mut ViewContext<Self>) {
        self.mode = SshProfileModalMode::Add;
        self.draft_id = Uuid::new_v4();
        self.available_profiles = profiles;
        self.selected_jump_hosts.clear();
        self.password_clear = false;
        self.had_stored_password = false;
        self.error = None;
        self.set_editor_text("", &self.name_editor, ctx);
        self.set_editor_text("", &self.host_editor, ctx);
        self.set_editor_text("", &self.user_editor, ctx);
        self.set_editor_text(&default_ssh_port().to_string(), &self.port_editor, ctx);
        self.reset_password_editor(ctx);
        self.set_editor_text("", &self.identity_file_editor, ctx);
        self.set_editor_text("", &self.tags_editor, ctx);
        self.jump_host_chip_mouse_states.borrow_mut().clear();
        self.sync_jump_host_dropdown(ctx);
        self.sync_save_button_state(ctx);
        ctx.notify();
    }

    pub fn open_edit(
        &mut self,
        profile: SshHostProfile,
        profiles: Vec<SshHostProfile>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.mode = SshProfileModalMode::Edit(profile.id);
        self.draft_id = profile.id;
        self.available_profiles = profiles
            .into_iter()
            .filter(|candidate| candidate.id != profile.id)
            .collect();
        self.selected_jump_hosts = profile.jump_hosts.clone();
        self.password_clear = false;
        self.error = None;
        self.set_editor_text(&profile.name, &self.name_editor, ctx);
        self.set_editor_text(&profile.host, &self.host_editor, ctx);
        self.set_editor_text(
            profile.user.as_deref().unwrap_or_default(),
            &self.user_editor,
            ctx,
        );
        self.set_editor_text(&profile.port.to_string(), &self.port_editor, ctx);
        let stored_password = match ctx
            .secure_storage()
            .read_value(&ssh_profile_password_key(profile.id))
        {
            Ok(password) => Some(Zeroizing::new(password)),
            Err(err) => {
                if !matches!(err, warpui_extras::secure_storage::Error::NotFound) {
                    log::warn!("Failed to read SSH profile password: {err:#}");
                }
                None
            }
        };
        self.had_stored_password = stored_password.is_some();
        self.reset_password_editor(ctx);
        if let Some(stored_password) = stored_password.as_ref() {
            self.set_editor_text(stored_password.as_str(), &self.password_editor, ctx);
        }
        self.set_editor_text(
            profile.identity_file.as_deref().unwrap_or_default(),
            &self.identity_file_editor,
            ctx,
        );
        self.set_editor_text(&profile.tags.join(", "), &self.tags_editor, ctx);
        self.jump_host_chip_mouse_states.borrow_mut().clear();
        self.sync_jump_host_dropdown(ctx);
        self.sync_save_button_state(ctx);
        ctx.notify();
    }

    pub fn on_open(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.name_editor);
    }

    fn reset_sensitive_state(&mut self, ctx: &mut ViewContext<Self>) {
        self.password_clear = false;
        self.had_stored_password = false;
        self.error = None;
        self.reset_password_editor(ctx);
    }

    fn reset_password_editor(&mut self, ctx: &mut ViewContext<Self>) {
        self.password_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
            editor.set_placeholder_text("Password", ctx);
            editor.set_is_password(true, ctx);
        });
    }

    fn set_error(&mut self, error: Option<String>, ctx: &mut ViewContext<Self>) {
        self.error = error;
        ctx.notify();
    }

    fn set_editor_text(
        &self,
        text: &str,
        editor: &ViewHandle<EditorView>,
        ctx: &mut ViewContext<Self>,
    ) {
        editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text_ignoring_undo(text, ctx);
        });
    }

    fn editors_in_tab_order(&self) -> [&ViewHandle<EditorView>; EDITOR_COUNT] {
        [
            &self.name_editor,
            &self.host_editor,
            &self.user_editor,
            &self.port_editor,
            &self.password_editor,
            &self.identity_file_editor,
            &self.tags_editor,
        ]
    }

    fn focus_editor(&self, index: usize, ctx: &mut ViewContext<Self>) {
        let editor = self.editors_in_tab_order()[index % EDITOR_COUNT].clone();
        ctx.focus(&editor);
    }

    fn handle_editor_event(
        &mut self,
        editor_index: usize,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EditorEvent::Escape => ctx.emit(SshProfileModalEvent::Cancel),
            EditorEvent::Enter => self.submit(ctx),
            EditorEvent::Navigate(NavigationKey::Tab) => self.focus_editor(editor_index + 1, ctx),
            EditorEvent::Navigate(NavigationKey::ShiftTab) => {
                self.focus_editor(editor_index + EDITOR_COUNT - 1, ctx)
            }
            _ => {
                self.error = None;
                self.sync_save_button_state(ctx);
                ctx.notify();
            }
        }
    }

    fn editor_text(&self, editor: &ViewHandle<EditorView>, app: &AppContext) -> String {
        editor.as_ref(app).buffer_text(app).trim().to_string()
    }

    fn parse_port(&self, app: &AppContext) -> Option<u16> {
        let text = self.editor_text(&self.port_editor, app);
        if text.is_empty() {
            return Some(default_ssh_port());
        }
        let port = text.parse::<u16>().ok()?;
        (port > 0).then_some(port)
    }

    fn label_error(&self, app: &AppContext) -> Option<String> {
        let label = self.editor_text(&self.name_editor, app);
        if label.is_empty() {
            return None;
        }
        if let Some(error) = ssh_alias_format_error(&label) {
            return Some(error.to_string());
        }
        self.available_profiles
            .iter()
            .any(|profile| profile.name == label)
            .then(|| format!("\"{label}\" is already a host in ~/.ssh/config"))
    }

    fn is_valid(&self, app: &AppContext) -> bool {
        !self.editor_text(&self.name_editor, app).is_empty()
            && self.label_error(app).is_none()
            && !self.editor_text(&self.host_editor, app).is_empty()
            && self.parse_port(app).is_some()
    }

    fn sync_save_button_state(&self, ctx: &mut ViewContext<Self>) {
        let is_valid = self.is_valid(ctx);
        self.save_button.update(ctx, |button, ctx| {
            button.set_disabled(!is_valid, ctx);
        });
    }

    fn build_profile(&self, app: &AppContext) -> Option<SshHostProfile> {
        let name = self.editor_text(&self.name_editor, app);
        let host = self.editor_text(&self.host_editor, app);
        if name.is_empty() || host.is_empty() {
            return None;
        }

        let user = self.optional_editor_text(&self.user_editor, app);
        let identity_file = self.optional_editor_text(&self.identity_file_editor, app);
        let tags = self
            .editor_text(&self.tags_editor, app)
            .split(',')
            .map(str::trim)
            .filter(|tag| !tag.is_empty())
            .map(ToOwned::to_owned)
            .collect();

        Some(SshHostProfile {
            id: self.draft_id,
            name,
            host,
            user,
            port: self.parse_port(app)?,
            identity_file,
            jump_hosts: self.selected_jump_hosts.clone(),
            tags,
        })
    }

    fn optional_editor_text(
        &self,
        editor: &ViewHandle<EditorView>,
        app: &AppContext,
    ) -> Option<String> {
        let value = self.editor_text(editor, app);
        (!value.is_empty()).then_some(value)
    }

    fn password_intent(&self, app: &AppContext) -> PasswordIntent {
        let password = self.password_editor.as_ref(app).buffer_text(app);
        if !password.is_empty() {
            PasswordIntent::Set(Zeroizing::new(password))
        } else if self.password_clear || self.had_stored_password {
            PasswordIntent::Clear
        } else {
            PasswordIntent::Keep
        }
    }

    fn submit(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.is_valid(ctx) {
            return;
        }
        let Some(profile) = self.build_profile(ctx) else {
            log::warn!("SSH profile form passed validation but could not be built");
            self.set_error(
                Some("This profile could not be built from the form.".to_string()),
                ctx,
            );
            return;
        };
        ctx.emit(SshProfileModalEvent::Submit {
            mode: self.mode.clone(),
            profile: Box::new(profile),
            password: self.password_intent(ctx),
        });
    }

    fn jump_host_from_profile(profile: &SshHostProfile) -> SshJumpHost {
        SshJumpHost {
            source_profile_id: Some(profile.id),
            host: profile.host.clone(),
            user: profile.user.clone(),
            port: profile.port,
            identity_file: profile.identity_file.clone(),
        }
    }

    fn jump_host_candidate_profiles(&self) -> Vec<&SshHostProfile> {
        self.available_profiles
            .iter()
            .filter(|profile| {
                !self
                    .selected_jump_hosts
                    .iter()
                    .any(|jump_host| jump_host_points_at(jump_host, profile))
            })
            .collect()
    }

    fn sync_jump_host_dropdown(&mut self, ctx: &mut ViewContext<Self>) {
        let mut items = vec![DropdownItem::new(
            "Add jump host",
            SshProfileModalAction::JumpHostDropdownPlaceholder,
        )];
        items.extend(
            self.jump_host_candidate_profiles()
                .into_iter()
                .map(|profile| {
                    DropdownItem::new(
                        format!(
                            "{}  {}",
                            profile.name,
                            Self::jump_host_from_profile(profile).target()
                        ),
                        SshProfileModalAction::AddJumpHost(profile.id),
                    )
                }),
        );
        let has_candidates = items.len() > 1;

        self.jump_host_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_items(items, ctx);
            dropdown
                .set_selected_by_action(SshProfileModalAction::JumpHostDropdownPlaceholder, ctx);
            if has_candidates {
                dropdown.set_enabled(ctx);
            } else {
                dropdown.set_disabled(ctx);
            }
        });
    }

    fn jump_host_display_label(&self, jump_host: &SshJumpHost) -> String {
        let target = jump_host.target();
        self.available_profiles
            .iter()
            .find(|profile| jump_host_points_at(jump_host, profile))
            .map(|profile| format!("{} - {}", profile.name, target))
            .unwrap_or(target)
    }

    fn add_jump_host(&mut self, profile_id: Uuid, ctx: &mut ViewContext<Self>) {
        let Some(profile) = self
            .available_profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .cloned()
        else {
            log::warn!("Cannot add jump host {profile_id}: it is not an available profile");
            return;
        };
        let jump_host = Self::jump_host_from_profile(&profile);
        let already_selected = self
            .selected_jump_hosts
            .iter()
            .any(|existing| jump_host_points_at(existing, &profile));
        if !already_selected {
            self.selected_jump_hosts.push(jump_host);
            self.sync_jump_host_dropdown(ctx);
            ctx.notify();
        }
    }

    fn remove_jump_host(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if index < self.selected_jump_hosts.len() {
            self.selected_jump_hosts.remove(index);
            self.jump_host_chip_mouse_states.borrow_mut().clear();
            self.sync_jump_host_dropdown(ctx);
            ctx.notify();
        }
    }

    fn render_label(text: &str, appearance: &Appearance) -> Box<dyn Element> {
        Text::new(text.to_string(), appearance.ui_font_family(), 11.)
            .with_color(appearance.theme().active_ui_text_color().into())
            .finish()
    }

    fn render_error_text(text: String, appearance: &Appearance) -> Box<dyn Element> {
        Text::new(text, appearance.ui_font_family(), 11.)
            .with_color(appearance.theme().ui_error_color())
            .finish()
    }

    fn render_text_input(
        &self,
        editor: &ViewHandle<EditorView>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .text_input(editor.clone())
            .with_style(UiComponentStyles {
                padding: Some(Coords {
                    top: 7.,
                    bottom: 7.,
                    left: 9.,
                    right: 9.,
                }),
                background: Some(appearance.theme().surface_1().into()),
                border_color: Some(appearance.theme().outline().into()),
                border_width: Some(1.),
                border_radius: Some(CornerRadius::with_all(Radius::Pixels(FIELD_RADIUS))),
                ..Default::default()
            })
            .build()
            .finish()
    }

    fn render_field(
        &self,
        label: &str,
        editor: &ViewHandle<EditorView>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Flex::column()
            .with_spacing(FIELD_LABEL_SPACING)
            .with_child(Self::render_label(label, appearance))
            .with_child(self.render_text_input(editor, appearance))
            .finish()
    }

    fn render_name_field(&self, app: &AppContext, appearance: &Appearance) -> Box<dyn Element> {
        let mut column = Flex::column()
            .with_spacing(FIELD_LABEL_SPACING)
            .with_child(Self::render_label("Label *", appearance))
            .with_child(self.render_text_input(&self.name_editor, appearance));
        if let Some(error) = self.label_error(app) {
            column = column.with_child(Self::render_error_text(error, appearance));
        }
        column.finish()
    }

    fn render_password_field(&self, app: &AppContext, appearance: &Appearance) -> Box<dyn Element> {
        let password_is_visible = !self.password_editor.as_ref(app).is_password();
        let eye = icon_button(
            appearance,
            Icon::Eye,
            password_is_visible,
            self.password_eye_mouse_state.clone(),
        )
        .build()
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(SshProfileModalAction::TogglePasswordVisibility)
        })
        .finish();

        let clear = icon_button(
            appearance,
            Icon::X,
            self.password_clear,
            self.password_clear_mouse_state.clone(),
        )
        .build()
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(SshProfileModalAction::ClearPassword))
        .finish();

        Flex::column()
            .with_spacing(FIELD_LABEL_SPACING)
            .with_child(Self::render_label("Password", appearance))
            .with_child(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(6.)
                    .with_child(
                        Expanded::new(
                            1.,
                            self.render_text_input(&self.password_editor, appearance),
                        )
                        .finish(),
                    )
                    .with_child(eye)
                    .with_child(clear)
                    .finish(),
            )
            .with_child(
                Text::new(
                    if (self.password_clear || self.had_stored_password)
                        && self.password_editor.as_ref(app).buffer_text(app).is_empty()
                    {
                        "Stored password will be removed"
                    } else {
                        ""
                    },
                    appearance.ui_font_family(),
                    10.,
                )
                .with_color(
                    appearance
                        .theme()
                        .sub_text_color(appearance.theme().background())
                        .into(),
                )
                .finish(),
            )
            .finish()
    }

    fn render_jump_hosts(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let mut selected = Wrap::row().with_run_spacing(6.).with_spacing(6.);
        for (index, jump_host) in self.selected_jump_hosts.iter().enumerate() {
            let mouse_state = self
                .jump_host_chip_mouse_states
                .borrow_mut()
                .entry(index)
                .or_default()
                .clone();
            let label = self.jump_host_display_label(jump_host);
            selected = selected.with_child(
                Container::new(
                    Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_spacing(4.)
                        .with_child(
                            Text::new(label, appearance.ui_font_family(), 11.)
                                .with_color(theme.active_ui_text_color().into())
                                .finish(),
                        )
                        .with_child(
                            icon_button(appearance, Icon::X, false, mouse_state)
                                .build()
                                .on_click(move |ctx, _, _| {
                                    ctx.dispatch_typed_action(
                                        SshProfileModalAction::RemoveJumpHost(index),
                                    )
                                })
                                .finish(),
                        )
                        .finish(),
                )
                .with_padding_left(8.)
                .with_padding_right(2.)
                .with_padding_top(3.)
                .with_padding_bottom(3.)
                .with_background(theme.surface_1())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(FIELD_RADIUS)))
                .finish(),
            );
        }

        let available_is_empty = self.jump_host_candidate_profiles().is_empty();
        let empty_text = if self.available_profiles.is_empty() {
            "No other profiles available"
        } else {
            "All available profiles selected"
        };

        let mut content = Flex::column()
            .with_spacing(7.)
            .with_child(Self::render_label("Jump hosts", appearance))
            .with_child(ChildView::new(&self.jump_host_dropdown).finish())
            .with_child(selected.finish());

        if available_is_empty {
            content = content.with_child(
                Text::new(empty_text, appearance.ui_font_family(), 11.)
                    .with_color(theme.disabled_ui_text_color().into())
                    .finish(),
            );
        }

        content.finish()
    }
}

impl Entity for SshProfileModalView {
    type Event = SshProfileModalEvent;
}

impl TypedActionView for SshProfileModalView {
    type Action = SshProfileModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SshProfileModalAction::Cancel => ctx.emit(SshProfileModalEvent::Cancel),
            SshProfileModalAction::Save => self.submit(ctx),
            SshProfileModalAction::TogglePasswordVisibility => {
                self.password_editor.update(ctx, |editor, ctx| {
                    editor.set_is_password(!editor.is_password(), ctx);
                });
                ctx.notify();
            }
            SshProfileModalAction::ClearPassword => {
                self.password_clear = !self.password_clear;
                if self.password_clear {
                    let password_editor = self.password_editor.clone();
                    self.set_editor_text("", &password_editor, ctx);
                }
                ctx.notify();
            }
            SshProfileModalAction::JumpHostDropdownPlaceholder => {}
            SshProfileModalAction::AddJumpHost(profile_id) => self.add_jump_host(*profile_id, ctx),
            SshProfileModalAction::RemoveJumpHost(index) => self.remove_jump_host(*index, ctx),
        }
    }
}

impl View for SshProfileModalView {
    fn ui_name() -> &'static str {
        "SshProfileModalView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let form = Flex::column()
            .with_spacing(12.)
            .with_child(self.render_name_field(app, appearance))
            .with_child(self.render_field("Host *", &self.host_editor, appearance))
            .with_child(
                Flex::row()
                    .with_spacing(10.)
                    .with_child(
                        Expanded::new(
                            1.,
                            self.render_field("Username", &self.user_editor, appearance),
                        )
                        .finish(),
                    )
                    .with_child(
                        ConstrainedBox::new(self.render_field(
                            "Port",
                            &self.port_editor,
                            appearance,
                        ))
                        .with_width(96.)
                        .finish(),
                    )
                    .finish(),
            )
            .with_child(
                Text::new("Authentication", appearance.ui_font_family(), 11.)
                    .with_color(theme.sub_text_color(theme.background()).into())
                    .with_style(Properties::default().weight(Weight::Medium))
                    .finish(),
            )
            .with_child(self.render_password_field(app, appearance))
            .with_child(self.render_field("Identity file", &self.identity_file_editor, appearance))
            .with_child(
                Text::new("Advanced", appearance.ui_font_family(), 11.)
                    .with_color(theme.sub_text_color(theme.background()).into())
                    .with_style(Properties::default().weight(Weight::Medium))
                    .finish(),
            )
            .with_child(self.render_jump_hosts(appearance))
            .with_child(self.render_field("Tags", &self.tags_editor, appearance));

        let buttons = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(8.)
                .with_child(ChildView::new(&self.cancel_button).finish())
                .with_child(ChildView::new(&self.save_button).finish())
                .finish(),
        )
        .with_padding_top(18.)
        .finish();

        let scrollable_form = ClippedScrollable::vertical(
            self.form_scroll_state.clone(),
            form.finish(),
            ScrollbarWidth::Custom(SCROLLBAR_WIDTH),
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            ElementFill::None,
        )
        .with_overlayed_scrollbar()
        .finish();

        let mut content = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(Shrinkable::new(1., scrollable_form).finish());
        if let Some(error) = self.error.as_ref() {
            content = content.with_child(
                Container::new(Self::render_error_text(error.clone(), appearance))
                    .with_padding_top(FIELD_LABEL_SPACING)
                    .finish(),
            );
        }
        content = content.with_child(buttons);

        Container::new(content.finish())
            .with_background(theme.surface_2())
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

fn jump_host_points_at(jump_host: &SshJumpHost, profile: &SshHostProfile) -> bool {
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

fn ssh_alias_format_error(label: &str) -> Option<&'static str> {
    if label.chars().any(char::is_whitespace) {
        return Some("Label cannot contain spaces or line breaks");
    }
    if label.chars().any(char::is_control) {
        return Some("Label cannot contain control characters");
    }
    if label
        .chars()
        .any(|character| LABEL_FORBIDDEN_CHARACTERS.contains(&character))
    {
        return Some("Label cannot contain # * ? ! \" or '");
    }
    None
}

pub enum SshProfileDeleteModalEvent {
    Confirm(Uuid),
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SshProfileDeleteModalAction {
    Confirm,
    Cancel,
}

pub struct SshProfileDeleteModalView {
    profile: Option<(Uuid, String)>,
    error: Option<String>,
    cancel_button_mouse_state: MouseStateHandle,
    confirm_button_mouse_state: MouseStateHandle,
}

impl SshProfileDeleteModalView {
    pub fn new() -> Self {
        Self {
            profile: None,
            error: None,
            cancel_button_mouse_state: Default::default(),
            confirm_button_mouse_state: Default::default(),
        }
    }

    fn open(&mut self, profile_id: Uuid, profile_name: String, ctx: &mut ViewContext<Self>) {
        self.profile = Some((profile_id, profile_name));
        self.error = None;
        ctx.notify();
    }

    fn reset(&mut self, ctx: &mut ViewContext<Self>) {
        self.profile = None;
        self.error = None;
        ctx.notify();
    }

    fn set_error(&mut self, error: Option<String>, ctx: &mut ViewContext<Self>) {
        self.error = error;
        ctx.notify();
    }
}

impl Entity for SshProfileDeleteModalView {
    type Event = SshProfileDeleteModalEvent;
}

impl TypedActionView for SshProfileDeleteModalView {
    type Action = SshProfileDeleteModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SshProfileDeleteModalAction::Cancel => ctx.emit(SshProfileDeleteModalEvent::Cancel),
            SshProfileDeleteModalAction::Confirm => {
                let Some((profile_id, _)) = self.profile.as_ref() else {
                    log::warn!("SSH profile delete confirmed with no profile selected");
                    return;
                };
                ctx.emit(SshProfileDeleteModalEvent::Confirm(*profile_id));
            }
        }
    }
}

impl View for SshProfileDeleteModalView {
    fn ui_name() -> &'static str {
        "SshProfileDeleteModalView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let name = self
            .profile
            .as_ref()
            .map(|(_, name)| name.as_str())
            .unwrap_or_default();

        let mut content = Flex::column()
            .with_spacing(12.)
            .with_child(
                Text::new(
                    format!(
                        "\"{name}\" will be removed from ~/.ssh/config and its stored password will be deleted. This cannot be undone."
                    ),
                    appearance.ui_font_family(),
                    12.,
                )
                .with_color(theme.sub_text_color(theme.surface_2()).into())
                .finish(),
            );

        if let Some(error) = self.error.as_ref() {
            content = content.with_child(SshProfileModalView::render_error_text(
                error.clone(),
                appearance,
            ));
        }

        let button_style = UiComponentStyles {
            font_size: Some(12.),
            padding: Some(Coords::uniform(8.).left(12.).right(12.)),
            ..Default::default()
        };
        content = content.with_child(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(8.)
                .with_child(
                    appearance
                        .ui_builder()
                        .button(
                            ButtonVariant::Secondary,
                            self.cancel_button_mouse_state.clone(),
                        )
                        .with_text_label("Cancel".to_string())
                        .with_style(button_style)
                        .build()
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(SshProfileDeleteModalAction::Cancel);
                        })
                        .finish(),
                )
                .with_child(
                    appearance
                        .ui_builder()
                        .button(
                            ButtonVariant::Error,
                            self.confirm_button_mouse_state.clone(),
                        )
                        .with_text_label("Delete".to_string())
                        .with_style(button_style)
                        .build()
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(SshProfileDeleteModalAction::Confirm);
                        })
                        .finish(),
                )
                .finish(),
        );

        Container::new(content.finish())
            .with_background(theme.surface_2())
            .finish()
    }
}
