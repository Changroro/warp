use std::cell::RefCell;
use std::collections::HashMap;

use pathfinder_geometry::vector::vec2f;
use settings::Setting as _;
use uuid::Uuid;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    Border, ChildAnchor, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    Expanded, Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    OffsetPositioning, Padding, ParentAnchor, ParentElement, ParentOffsetBounds, Radius,
    Shrinkable, Stack, Text, Wrap,
};
use warpui::fonts::{Properties, Weight};
use warpui::platform::Cursor;
use warpui::text_layout::ClipConfig;
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
use crate::terminal::ssh::config_file;
use crate::terminal::ssh::util::{
    default_ssh_port, normalize_ssh_host_profiles, ssh_profile_password_key, SshHostProfile,
    SshJumpHost,
};
use crate::themes::theme::Fill as ThemeFill;
use crate::ui_components::buttons::icon_button;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{ActionButton, PrimaryTheme, SecondaryTheme};
use crate::view_components::{Dropdown, DropdownItem};
use crate::workspace::WorkspaceAction;

const PANEL_WIDTH: f32 = 280.;
const PANEL_PADDING: f32 = 10.;
const ROW_WIDTH: f32 = PANEL_WIDTH - PANEL_PADDING * 2.;
const ROW_RADIUS: f32 = 4.;
const ROW_ICON_SIZE: f32 = 16.;
const ROW_ICON_PADDING: f32 = 8.;
const ROW_ACTION_BUTTON_ICON_SIZE: f32 = 12.;
const ROW_ACTION_BUTTON_PADDING: f32 = 2.;
const ROW_ACTION_BUTTON_GAP: f32 = 2.;
const JUMP_HOST_DROPDOWN_WIDTH: f32 = 460.;

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
    row_mouse_states: RefCell<HashMap<Uuid, ProfileRowMouseStates>>,
    add_button_mouse_state: MouseStateHandle,
}

impl SshProfilesPanelView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        sync_profiles_with_ssh_config(ctx);

        let modal_body = ctx.add_typed_action_view(SshProfileModalView::new);
        ctx.subscribe_to_view(&modal_body, |me, _, event, ctx| {
            me.handle_modal_body_event(event, ctx);
        });

        let modal = ctx.add_typed_action_view(|ctx| {
            Modal::new(Some("SSH profile".to_string()), modal_body, ctx)
                .with_dismiss_on_click()
                .with_modal_style(UiComponentStyles {
                    width: Some(520.),
                    height: Some(760.),
                    ..Default::default()
                })
                .with_body_style(UiComponentStyles {
                    height: Some(690.),
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

        ctx.subscribe_to_model(&SshSettings::handle(ctx), |me, _, _, ctx| {
            me.reconcile_row_mouse_states(ctx);
            ctx.notify();
        });

        Self {
            modal_state: ModalViewState::new(modal),
            row_mouse_states: Default::default(),
            add_button_mouse_state: Default::default(),
        }
    }

    pub fn render_modal(&self) -> Option<Box<dyn Element>> {
        self.modal_state
            .is_open()
            .then(|| self.modal_state.render())
    }

    fn profiles(app: &AppContext) -> Vec<SshHostProfile> {
        let mut profiles = SshSettings::as_ref(app)
            .saved_ssh_host_profiles
            .value()
            .clone();
        normalize_ssh_host_profiles(&mut profiles);
        profiles
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
            return;
        };

        self.modal_state.view.update(ctx, |modal, ctx| {
            modal.set_title(Some("Edit SSH profile".to_string()));
            modal.body().update(ctx, |body, ctx| {
                body.open_edit(profile, profiles, ctx);
            });
        });
        self.modal_state.open();
        ctx.emit(SshProfilesPanelEvent::ModalToggled);
        ctx.notify();
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
            } => {
                self.save_profile(
                    mode.clone(),
                    profile.as_ref().clone(),
                    password.clone(),
                    ctx,
                );
                self.close_modal(ctx);
            }
        }
    }

    fn save_profile(
        &mut self,
        mode: SshProfileModalMode,
        profile: SshHostProfile,
        password: PasswordIntent,
        ctx: &mut ViewContext<Self>,
    ) {
        let mut profile = profile;
        if let SshProfileModalMode::Edit(id) = mode {
            profile.id = id;
        }

        // If the config can't be written we must not touch the password either, or the two would
        // disagree about which profile exists.
        if let Err(err) = config_file::upsert_profile(&profile) {
            log::error!("Failed to save SSH profile to ~/.ssh/config: {err:#}");
            return;
        }

        let profile_id = profile.id;
        sync_profiles_with_ssh_config(ctx);

        match password {
            PasswordIntent::Keep => {}
            PasswordIntent::Set(password) => {
                self.write_profile_password(profile_id, password.as_str(), ctx);
            }
            PasswordIntent::Clear => {
                if let Err(err) = ctx
                    .secure_storage()
                    .remove_value(&ssh_profile_password_key(profile_id))
                {
                    if !matches!(err, warpui_extras::secure_storage::Error::NotFound) {
                        log::warn!("Failed to remove SSH profile password: {err:#}");
                    }
                }
            }
        }
    }

    fn write_profile_password(
        &self,
        profile_id: Uuid,
        password: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        let key = ssh_profile_password_key(profile_id);
        if ctx.secure_storage().write_value(&key, password).is_ok() {
            return;
        }

        let _ = ctx.secure_storage().remove_value(&key);
        if let Err(err) = ctx.secure_storage().write_value(&key, password) {
            log::error!("Failed to write SSH profile password: {err:#}");
        }
    }

    fn remove_profile(&mut self, profile_id: Uuid, ctx: &mut ViewContext<Self>) {
        let profiles = Self::profiles(ctx);
        let removed_jump_host = profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .map(SshProfileModalView::jump_host_from_profile);

        if let Err(err) = config_file::remove_profile(profile_id) {
            log::error!("Failed to remove SSH profile from ~/.ssh/config: {err:#}");
            return;
        }

        // Any profile that jumped through the removed host would otherwise keep a dangling hop.
        if let Some(removed_jump_host) = removed_jump_host {
            let removed_target = removed_jump_host.target();
            for mut profile in profiles
                .into_iter()
                .filter(|profile| profile.id != profile_id)
            {
                if !profile
                    .jump_hosts
                    .iter()
                    .any(|jump_host| jump_host.target() == removed_target)
                {
                    continue;
                }
                profile
                    .jump_hosts
                    .retain(|jump_host| jump_host.target() != removed_target);
                if let Err(err) = config_file::upsert_profile(&profile) {
                    log::error!("Failed to drop jump host from ~/.ssh/config: {err:#}");
                }
            }
        }

        sync_profiles_with_ssh_config(ctx);

        if let Err(err) = ctx
            .secure_storage()
            .remove_value(&ssh_profile_password_key(profile_id))
        {
            if !matches!(err, warpui_extras::secure_storage::Error::NotFound) {
                log::warn!("Failed to remove SSH profile password: {err:#}");
            }
        }
        self.row_mouse_states.borrow_mut().remove(&profile_id);
        ctx.notify();
    }

    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let add_button = icon_button(
            appearance,
            Icon::Plus,
            false,
            self.add_button_mouse_state.clone(),
        )
        .with_tooltip({
            let ui_builder = appearance.ui_builder().clone();
            move || {
                ui_builder
                    .tool_tip("Add SSH profile".to_string())
                    .build()
                    .finish()
            }
        })
        .build()
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(SshProfilesPanelAction::Add))
        .finish();

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
                .with_child(add_button)
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
            Text::new(
                "No saved profiles yet. Click + to add one.",
                appearance.ui_font_family(),
                12.,
            )
            .with_color(theme.sub_text_color(theme.background()).into())
            .finish(),
        )
        .with_padding(Padding::uniform(12.))
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
        let subtitle = profile.target();
        let action_buttons_mouse_over = mouse_states
            .actions
            .lock()
            .expect("SSH profile row action buttons hover state lock poisoned")
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
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(24.)))
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
                .with_spacing(8.)
                .with_child(profile_icon)
                .with_child(Shrinkable::new(1., text_column.finish()).finish())
                .finish();

            let background = (state.is_hovered() || state.is_clicked())
                .then(|| internal_colors::fg_overlay_1(theme));

            let mut container = Container::new(row_content)
                .with_padding(Padding::uniform(8.))
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
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
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
                            ConstrainedBox::new(Icon::X.to_warpui_icon(meta_color).finish())
                                .with_width(ROW_ACTION_BUTTON_ICON_SIZE)
                                .with_height(ROW_ACTION_BUTTON_ICON_SIZE)
                                .finish(),
                        )
                        .with_padding(Padding::uniform(ROW_ACTION_BUTTON_PADDING))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
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
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
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
            ctx.dispatch_typed_action(WorkspaceAction::ConnectSshProfile(profile_id));
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
            SshProfilesPanelAction::Remove(id) => self.remove_profile(*id, ctx),
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
            .with_child(self.render_header(appearance));

        if profiles.is_empty() {
            content = content.with_child(self.render_empty_state(appearance));
        } else {
            let mut list = Flex::column()
                .with_spacing(4.)
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch);
            for profile in profiles {
                list = list.with_child(self.render_profile_row(profile, appearance));
            }
            content = content.with_child(
                Container::new(list.finish())
                    .with_padding_left(PANEL_PADDING)
                    .with_padding_right(PANEL_PADDING)
                    .with_padding_bottom(PANEL_PADDING)
                    .finish(),
            );
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SshProfileModalMode {
    Add,
    Edit(Uuid),
}

#[derive(Debug, Clone)]
pub enum PasswordIntent {
    Keep,
    Set(Zeroizing<String>),
    Clear,
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
    cancel_button: ViewHandle<ActionButton>,
    save_button: ViewHandle<ActionButton>,
    password_eye_mouse_state: MouseStateHandle,
    password_clear_mouse_state: MouseStateHandle,
    jump_host_chip_mouse_states: RefCell<HashMap<usize, MouseStateHandle>>,
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

        for editor in [
            &name_editor,
            &host_editor,
            &user_editor,
            &port_editor,
            &password_editor,
            &identity_file_editor,
            &tags_editor,
        ] {
            ctx.subscribe_to_view(editor, |me, _, event, ctx| {
                me.handle_editor_event(event, ctx);
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
            cancel_button,
            save_button,
            password_eye_mouse_state: Default::default(),
            password_clear_mouse_state: Default::default(),
            jump_host_chip_mouse_states: Default::default(),
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
        self.set_editor_text("", &self.name_editor, ctx);
        self.set_editor_text("", &self.host_editor, ctx);
        self.set_editor_text("", &self.user_editor, ctx);
        self.set_editor_text("22", &self.port_editor, ctx);
        self.set_editor_text("", &self.password_editor, ctx);
        self.password_editor.update(ctx, |editor, ctx| {
            editor.set_placeholder_text("Password", ctx);
            editor.set_is_password(true, ctx);
        });
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
        self.set_editor_text(&profile.name, &self.name_editor, ctx);
        self.set_editor_text(&profile.host, &self.host_editor, ctx);
        self.set_editor_text(
            profile.user.as_deref().unwrap_or_default(),
            &self.user_editor,
            ctx,
        );
        self.set_editor_text(&profile.port.to_string(), &self.port_editor, ctx);
        // The editor stores plain text so the user can reveal a saved password on demand. Zeroizing
        // only bounds this temporary secure-storage read; it is not an end-to-end memory guarantee.
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
        self.set_editor_text(
            stored_password
                .as_ref()
                .map(|password| password.as_str())
                .unwrap_or_default(),
            &self.password_editor,
            ctx,
        );
        self.password_editor.update(ctx, |editor, ctx| {
            editor.set_placeholder_text("Password", ctx);
            editor.set_is_password(true, ctx);
        });
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

    fn reset_sensitive_state(&mut self, ctx: &mut ViewContext<Self>) {
        self.set_editor_text("", &self.password_editor, ctx);
        self.password_clear = false;
        self.password_editor.update(ctx, |editor, ctx| {
            editor.set_placeholder_text("Password", ctx);
            editor.set_is_password(true, ctx);
        });
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

    fn handle_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Escape => ctx.emit(SshProfileModalEvent::Cancel),
            EditorEvent::Enter => self.submit(ctx),
            _ => {
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

    fn is_valid(&self, app: &AppContext) -> bool {
        !self.editor_text(&self.name_editor, app).is_empty()
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
        } else if self.password_clear {
            PasswordIntent::Clear
        } else {
            PasswordIntent::Keep
        }
    }

    fn submit(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.is_valid(ctx) {
            return;
        }
        if let Some(profile) = self.build_profile(ctx) {
            ctx.emit(SshProfileModalEvent::Submit {
                mode: self.mode.clone(),
                profile: Box::new(profile),
                password: self.password_intent(ctx),
            });
        }
    }

    fn jump_host_from_profile(profile: &SshHostProfile) -> SshJumpHost {
        SshJumpHost {
            host: profile.host.clone(),
            user: profile.user.clone(),
            port: profile.port,
            identity_file: profile.identity_file.clone(),
        }
    }

    fn profile_matches_jump_host(profile: &SshHostProfile, jump_host: &SshJumpHost) -> bool {
        Self::jump_host_from_profile(profile).target() == jump_host.target()
    }

    fn jump_host_candidate_profiles(&self) -> Vec<&SshHostProfile> {
        let selected_targets = self
            .selected_jump_hosts
            .iter()
            .map(SshJumpHost::target)
            .collect::<std::collections::HashSet<_>>();

        self.available_profiles
            .iter()
            .filter(|profile| {
                !selected_targets.contains(&Self::jump_host_from_profile(profile).target())
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
            .find(|profile| Self::profile_matches_jump_host(profile, jump_host))
            .map(|profile| format!("{} - {}", profile.name, target))
            .unwrap_or(target)
    }

    fn add_jump_host(&mut self, profile_id: Uuid, ctx: &mut ViewContext<Self>) {
        let Some(jump_host) = self
            .available_profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .map(Self::jump_host_from_profile)
        else {
            return;
        };
        let already_selected = self
            .selected_jump_hosts
            .iter()
            .any(|existing| existing.target() == jump_host.target());
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
                border_radius: Some(CornerRadius::with_all(Radius::Pixels(6.))),
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
            .with_spacing(5.)
            .with_child(Self::render_label(label, appearance))
            .with_child(self.render_text_input(editor, appearance))
            .finish()
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
            .with_spacing(5.)
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
                    if self.password_clear
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
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
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
            // The placeholder keeps the dropdown trigger selectable without mutating the form.
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
            .with_child(self.render_field("Label *", &self.name_editor, appearance))
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

        Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_child(Shrinkable::new(1., form.finish()).finish())
                .with_child(buttons)
                .finish(),
        )
        .with_background(theme.surface_2())
        .finish()
    }
}

/// `~/.ssh/config` owns the profiles; the settings entry is only the cache the UI renders from.
///
/// Profiles that predate this (they lived only in Warp's settings) are written into the config once,
/// so the user gets them in every terminal, not just Warp.
fn sync_profiles_with_ssh_config(ctx: &mut ViewContext<SshProfilesPanelView>) {
    let cached = SshSettings::as_ref(ctx)
        .saved_ssh_host_profiles
        .value()
        .clone();

    let mut profiles = match config_file::load_profiles() {
        Ok(profiles) => profiles,
        Err(err) => {
            log::error!("Failed to read SSH profiles from ~/.ssh/config: {err:#}");
            return;
        }
    };

    let migrated = cached
        .iter()
        .filter(|profile| {
            !profiles
                .iter()
                .any(|existing| existing.id == profile.id || existing.name == profile.name)
        })
        .fold(false, |migrated, profile| {
            match config_file::upsert_profile(profile) {
                Ok(()) => true,
                Err(err) => {
                    log::error!("Failed to migrate SSH profile to ~/.ssh/config: {err:#}");
                    migrated
                }
            }
        });

    if migrated {
        profiles = match config_file::load_profiles() {
            Ok(profiles) => profiles,
            Err(err) => {
                log::error!("Failed to re-read SSH profiles from ~/.ssh/config: {err:#}");
                return;
            }
        };
    }

    let before = profiles.clone();
    normalize_ssh_host_profiles(&mut profiles);
    warn_normalized_profile_id_changes(&before, &profiles);

    if profiles != cached {
        SshSettings::handle(ctx).update(ctx, |settings, ctx| {
            if let Err(err) = settings.saved_ssh_host_profiles.set_value(profiles, ctx) {
                log::error!("Failed to cache SSH profiles: {err:#}");
            }
        });
    }
}

fn warn_normalized_profile_id_changes(before: &[SshHostProfile], after: &[SshHostProfile]) {
    for (old_profile, normalized_profile) in before.iter().zip(after) {
        if old_profile.id != normalized_profile.id {
            log::warn!(
                "Regenerated invalid or duplicate SSH profile id: old_id={}, new_id={}. Any password stored under the old id must be saved again.",
                old_profile.id,
                normalized_profile.id
            );
        }
    }
}
