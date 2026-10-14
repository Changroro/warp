//! The add/edit and delete dialogs for SSH host profiles.

use std::cell::RefCell;
use std::collections::HashMap;

use uuid::Uuid;
use warp_editor::editor::NavigationKey;
use warpui::elements::{
    ChildView, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container,
    CornerRadius, CrossAxisAlignment, Expanded, Fill as ElementFill, Flex, MainAxisAlignment,
    MainAxisSize, MouseStateHandle, ParentElement, Radius, ScrollbarWidth, Shrinkable, Text, Wrap,
};
use warpui::fonts::{Properties, Weight};
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};
use warpui_extras::secure_storage::AppContextExt as _;
use zeroize::Zeroizing;

use crate::appearance::Appearance;
use crate::editor::{EditorView, Event as EditorEvent, SingleLineEditorOptions, TextOptions};
use crate::ssh_manager::panel::jump_host_points_at;
use crate::ssh_manager::profile::{
    SshHostProfile, SshJumpHost, default_ssh_port, ssh_profile_password_key,
};
use crate::ui_components::buttons::icon_button;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{ActionButton, PrimaryTheme, SecondaryTheme};
use crate::view_components::{Dropdown, DropdownItem};

const SCROLLBAR_WIDTH: f32 = 4.;
const JUMP_HOST_DROPDOWN_WIDTH: f32 = 460.;
const FIELD_RADIUS: f32 = 6.;
const FIELD_LABEL_SPACING: f32 = 5.;
const EDITOR_COUNT: usize = 7;
const LABEL_FORBIDDEN_CHARACTERS: [char; 6] = ['#', '*', '?', '!', '"', '\''];

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

    pub(super) fn reset_sensitive_state(&mut self, ctx: &mut ViewContext<Self>) {
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

    pub(super) fn set_error(&mut self, error: Option<String>, ctx: &mut ViewContext<Self>) {
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

    pub(super) fn render_error_text(text: String, appearance: &Appearance) -> Box<dyn Element> {
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

    pub(super) fn open(
        &mut self,
        profile_id: Uuid,
        profile_name: String,
        ctx: &mut ViewContext<Self>,
    ) {
        self.profile = Some((profile_id, profile_name));
        self.error = None;
        ctx.notify();
    }

    pub(super) fn reset(&mut self, ctx: &mut ViewContext<Self>) {
        self.profile = None;
        self.error = None;
        ctx.notify();
    }

    pub(super) fn set_error(&mut self, error: Option<String>, ctx: &mut ViewContext<Self>) {
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
