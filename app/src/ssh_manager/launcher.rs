//! The SSH launcher palette: search saved hosts or connect to a typed destination.

use std::cell::RefCell;
use std::collections::HashMap;

use chrono::Utc;
use settings::Setting as _;
use uuid::Uuid;
use warp_core::ui::theme::color::internal_colors;
use warp_editor::editor::NavigationKey;
use warpui::elements::{
    Border, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Expanded, Fill as ElementFill, Flex, Hoverable, MainAxisSize,
    MouseStateHandle, ParentElement, Radius, ScrollbarWidth, Shrinkable, Text,
};
use warpui::platform::Cursor;
use warpui::text_layout::ClipConfig;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::appearance::Appearance;
use crate::editor::{EditorView, Event as EditorEvent, SingleLineEditorOptions, TextOptions};
use crate::settings::SshSettings;
use crate::ssh_manager::SshManagerAction;
use crate::ssh_manager::open_connections::SshOpenConnections;
use crate::ssh_manager::profile::{QuickConnectDestination, SshHostProfile, quick_connect_args};
use crate::ssh_manager::ui::{
    ansi_color, badge_label, host_color, host_subtitle, render_badge, render_chip, render_dot,
    render_kbd,
};
use crate::ssh_manager::usage::{SshHostUsage, format_relative_time};
use crate::themes::theme::Fill as ThemeFill;
use crate::ui_components::icons::Icon;
use crate::workspace::WorkspaceAction;

pub const LAUNCHER_WIDTH: f32 = 680.;
const LIST_MAX_HEIGHT: f32 = 440.;
const MAX_RECENT_HOSTS: usize = 3;
const MAX_RECENT_QUICK_CONNECTS: usize = 3;
const MAX_ALL_HOSTS: usize = 5;
const ROW_PADDING_H: f32 = 14.;
const ROW_PADDING_V: f32 = 7.;

#[derive(Debug, Clone)]
pub enum SshLauncherEvent {
    Close,
}

#[derive(Debug, Clone)]
pub enum SshLauncherAction {
    Activate(usize),
    SaveQuickConnect(String),
}

/// A row's badge, title, subtitle, and trailing elements.
type RowParts = (
    Box<dyn Element>,
    Box<dyn Element>,
    String,
    Vec<Box<dyn Element>>,
);

#[derive(Clone)]
enum Row {
    QuickConnect {
        args: String,
    },
    Host {
        profile: SshHostProfile,
        pinned: bool,
        last_connected: Option<i64>,
    },
    RecentQuickConnect {
        args: String,
        connected_at: i64,
    },
    SaveAndConnect {
        input: String,
        destination: QuickConnectDestination,
    },
    ShowAll,
}

enum Item {
    Header { title: String, detail: String },
    Row(Row),
}

pub struct SshLauncherView {
    editor: ViewHandle<EditorView>,
    selected: usize,
    row_mouse_states: RefCell<HashMap<usize, MouseStateHandle>>,
    save_mouse_states: RefCell<HashMap<usize, MouseStateHandle>>,
    scroll_state: ClippedScrollStateHandle,
}

impl SshLauncherView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let editor = ctx.add_view(|ctx| {
            EditorView::single_line(
                SingleLineEditorOptions {
                    text: TextOptions::ui_text(Some(15.), Appearance::as_ref(ctx)),
                    ..Default::default()
                },
                ctx,
            )
        });
        editor.update(ctx, |editor, ctx| {
            editor.set_placeholder_text("Search hosts or type user@host[:port]", ctx);
        });
        ctx.subscribe_to_view(&editor, |me, _, event, ctx| {
            me.handle_editor_event(event, ctx)
        });
        ctx.subscribe_to_model(&SshSettings::handle(ctx), |_, _, _, ctx| ctx.notify());
        let open_connections = SshOpenConnections::handle_or_init(ctx);
        ctx.subscribe_to_model(&open_connections, |_, _, _, ctx| ctx.notify());

        Self {
            editor,
            selected: 0,
            row_mouse_states: Default::default(),
            save_mouse_states: Default::default(),
            scroll_state: Default::default(),
        }
    }

    /// Clears the input and focuses it.
    pub fn on_open(&mut self, ctx: &mut ViewContext<Self>) {
        self.editor
            .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
        self.selected = 0;
        ctx.focus(&self.editor);
        ctx.notify();
    }

    fn query(&self, app: &AppContext) -> String {
        self.editor.as_ref(app).buffer_text(app).trim().to_string()
    }

    fn handle_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Escape => ctx.emit(SshLauncherEvent::Close),
            EditorEvent::Enter => self.activate(self.selected, ctx),
            EditorEvent::CmdEnter => {
                let items = self.items(ctx);
                if let Some(index) =
                    rows(&items).position(|row| matches!(row, Row::SaveAndConnect { .. }))
                {
                    self.activate(index, ctx);
                }
            }
            EditorEvent::Navigate(NavigationKey::Up) => self.move_selection(-1, ctx),
            EditorEvent::Navigate(NavigationKey::Down | NavigationKey::Tab) => {
                self.move_selection(1, ctx)
            }
            EditorEvent::Edited(_) => {
                self.selected = 0;
                ctx.notify();
            }
            _ => {}
        }
    }

    fn move_selection(&mut self, delta: isize, ctx: &mut ViewContext<Self>) {
        let count = rows(&self.items(ctx)).count();
        if count == 0 {
            return;
        }
        self.selected = (self.selected as isize + delta).rem_euclid(count as isize) as usize;
        ctx.notify();
    }

    fn activate(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        let items = self.items(ctx);
        let Some(row) = rows(&items).nth(index).cloned() else {
            return;
        };
        let action = match row {
            Row::QuickConnect { args } | Row::RecentQuickConnect { args, .. } => {
                SshManagerAction::QuickConnect(args)
            }
            Row::Host { profile, .. } => SshManagerAction::Connect(profile.id),
            Row::SaveAndConnect { input, .. } => SshManagerAction::SaveHost {
                input,
                connect: true,
            },
            Row::ShowAll => SshManagerAction::OpenPanel,
        };
        ctx.emit(SshLauncherEvent::Close);
        ctx.dispatch_typed_action(&WorkspaceAction::SshManager(action));
    }

    fn items(&self, app: &AppContext) -> Vec<Item> {
        let settings = SshSettings::as_ref(app);
        build_items(
            &self.query(app),
            settings.saved_ssh_host_profiles.value(),
            settings.ssh_host_usage.value(),
        )
    }

    fn render_header(title: &str, detail: &str, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let color = theme.disabled_ui_text_color();
        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_child(
                    Expanded::new(
                        1.,
                        Text::new_inline(title.to_string(), appearance.ui_font_family(), 11.)
                            .with_color(color.into())
                            .finish(),
                    )
                    .finish(),
                )
                .with_child(
                    Text::new_inline(detail.to_string(), appearance.ui_font_family(), 11.)
                        .with_color(color.into())
                        .finish(),
                )
                .finish(),
        )
        .with_padding_left(ROW_PADDING_H)
        .with_padding_right(ROW_PADDING_H)
        .with_padding_top(10.)
        .with_padding_bottom(4.)
        .finish()
    }

    fn render_row(
        &self,
        index: usize,
        row: &Row,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let selected = index == self.selected;
        let now = Utc::now();
        let main_color = theme.active_ui_text_color();
        let sub_color = theme.sub_text_color(theme.background());

        let (badge, title, subtitle, trailing): RowParts = match row {
            Row::QuickConnect { args } => (
                render_icon_badge(Icon::Terminal, appearance, true),
                Text::new_inline(
                    format!("ssh {args}"),
                    appearance.monospace_font_family(),
                    13.,
                )
                .with_clip(ClipConfig::ellipsis())
                .with_color(main_color.into())
                .finish(),
                "Quick connect · connects without saving".to_string(),
                vec![render_kbd("↵", appearance)],
            ),
            Row::Host {
                profile,
                pinned,
                last_connected,
            } => {
                let name = if *pinned {
                    format!("{} ★", profile.name)
                } else {
                    profile.name.clone()
                };
                let mut trailing = profile
                    .tags
                    .iter()
                    .take(2)
                    .map(|tag| render_chip(tag, appearance))
                    .collect::<Vec<_>>();
                let open_count = SshOpenConnections::open_count(profile.id, app);
                if open_count > 0 {
                    trailing.push(render_dot(
                        ansi_color(warp_core::ui::theme::AnsiColorIdentifier::Green, appearance),
                        8.,
                    ));
                }
                if let Some(last_connected) = last_connected {
                    trailing.push(
                        Text::new_inline(
                            format_relative_time(*last_connected, now),
                            appearance.ui_font_family(),
                            11.,
                        )
                        .with_color(sub_color.into())
                        .finish(),
                    );
                }
                (
                    render_badge(&badge_label(profile), host_color(profile), appearance),
                    Text::new_inline(name, appearance.ui_font_family(), 13.)
                        .with_clip(ClipConfig::ellipsis())
                        .with_color(main_color.into())
                        .finish(),
                    host_subtitle(profile),
                    trailing,
                )
            }
            Row::RecentQuickConnect { args, connected_at } => {
                let mut trailing = Vec::new();
                if QuickConnectDestination::parse(args).is_some() {
                    let args = args.clone();
                    let save_state = self
                        .save_mouse_states
                        .borrow_mut()
                        .entry(index)
                        .or_default()
                        .clone();
                    trailing.push(
                        Hoverable::new(save_state, move |state| {
                            let color = if state.is_hovered() {
                                theme.active_ui_text_color()
                            } else {
                                theme.sub_text_color(theme.background())
                            };
                            Text::new_inline("Save", appearance.ui_font_family(), 11.)
                                .with_color(color.into())
                                .finish()
                        })
                        .with_cursor(Cursor::PointingHand)
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(SshLauncherAction::SaveQuickConnect(
                                args.clone(),
                            ))
                        })
                        .finish(),
                    );
                }
                (
                    render_icon_badge(Icon::Clock, appearance, false),
                    Text::new_inline(args.clone(), appearance.monospace_font_family(), 12.5)
                        .with_clip(ClipConfig::ellipsis())
                        .with_color(main_color.into())
                        .finish(),
                    format!(
                        "Quick connect · {}",
                        format_relative_time(*connected_at, now)
                    ),
                    trailing,
                )
            }
            Row::SaveAndConnect { destination, .. } => (
                render_icon_badge(Icon::Plus, appearance, false),
                Text::new_inline("Save as host and connect", appearance.ui_font_family(), 13.)
                    .with_color(main_color.into())
                    .finish(),
                format!(
                    "{} → ~/.ssh/config",
                    host_subtitle(&destination.to_profile())
                ),
                vec![render_kbd(cmd_or_ctrl_enter(), appearance)],
            ),
            Row::ShowAll => (
                render_icon_badge(Icon::DotsHorizontal, appearance, false),
                Text::new_inline("Show all hosts", appearance.ui_font_family(), 13.)
                    .with_color(main_color.into())
                    .finish(),
                "Open the SSH hosts panel".to_string(),
                vec![],
            ),
        };

        let text = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(1.)
            .with_child(title)
            .with_child(
                Text::new_inline(subtitle, appearance.monospace_font_family(), 11.5)
                    .with_clip(ClipConfig::ellipsis())
                    .with_color(sub_color.into())
                    .finish(),
            )
            .finish();

        let mut content = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(10.)
            .with_child(badge)
            .with_child(Expanded::new(1., text).finish());
        for element in trailing {
            content = content.with_child(element);
        }
        let content = content.finish();

        let is_quick_connect = matches!(row, Row::QuickConnect { .. });
        let mouse_state = self
            .row_mouse_states
            .borrow_mut()
            .entry(index)
            .or_default()
            .clone();
        Hoverable::new(mouse_state, move |state| {
            let mut container = Container::new(content)
                .with_horizontal_padding(if is_quick_connect { 12. } else { ROW_PADDING_H })
                .with_vertical_padding(if is_quick_connect { 10. } else { ROW_PADDING_V });
            if is_quick_connect {
                let accent = theme.accent();
                container = container
                    .with_margin_left(8.)
                    .with_margin_right(8.)
                    .with_margin_top(6.)
                    .with_margin_bottom(6.)
                    .with_border(Border::all(1.).with_border_fill(accent))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)));
            }
            if selected {
                container = container.with_background(internal_colors::accent_overlay_2(theme));
            } else if state.is_hovered() {
                container = container.with_background(internal_colors::fg_overlay_1(theme));
            }
            container.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(SshLauncherAction::Activate(index)))
        .finish()
    }

    fn render_footer(appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let hint = |keys: &str, label: &str| -> Box<dyn Element> {
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(5.)
                .with_child(render_kbd(keys, appearance))
                .with_child(
                    Text::new_inline(label.to_string(), appearance.ui_font_family(), 11.)
                        .with_color(theme.sub_text_color(theme.background()).into())
                        .finish(),
                )
                .finish()
        };
        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(14.)
                .with_child(hint("↑↓", "Move"))
                .with_child(hint("↵", "Connect"))
                .with_child(hint(cmd_or_ctrl_enter(), "Save and connect"))
                .with_child(Expanded::new(1., warpui::elements::Empty::new().finish()).finish())
                .with_child(hint("esc", "Close"))
                .finish(),
        )
        .with_background(internal_colors::fg_overlay_1(theme))
        .with_border(Border::top(1.).with_border_fill(theme.outline()))
        .with_horizontal_padding(ROW_PADDING_H)
        .with_vertical_padding(8.)
        .finish()
    }
}

fn cmd_or_ctrl_enter() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘↵"
    } else {
        "Ctrl↵"
    }
}

fn render_icon_badge(icon: Icon, appearance: &Appearance, accent: bool) -> Box<dyn Element> {
    let theme = appearance.theme();
    let (background, icon_color) = if accent {
        (theme.accent(), theme.background())
    } else {
        (
            internal_colors::fg_overlay_3(theme),
            theme.sub_text_color(theme.background()),
        )
    };
    ConstrainedBox::new(
        Container::new(icon.to_warpui_icon(icon_color).finish())
            .with_background(background)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(5.)))
            .with_uniform_padding(4.)
            .finish(),
    )
    .with_width(crate::ssh_manager::ui::BADGE_SIZE)
    .with_height(crate::ssh_manager::ui::BADGE_SIZE)
    .finish()
}

fn rows(items: &[Item]) -> impl Iterator<Item = &Row> {
    items.iter().filter_map(|item| match item {
        Item::Row(row) => Some(row),
        Item::Header { .. } => None,
    })
}

/// Input that names a destination rather than a search term, so quick connect leads the list.
fn looks_like_destination(query: &str) -> bool {
    query.contains('@')
        || query.starts_with("ssh ")
        || query.contains(char::is_whitespace)
        || QuickConnectDestination::parse(query).is_some_and(|destination| destination.port != 22)
}

fn build_items(query: &str, profiles: &[SshHostProfile], usage: &SshHostUsage) -> Vec<Item> {
    let mut items = Vec::new();
    let host_row = |profile: &SshHostProfile| {
        Item::Row(Row::Host {
            profile: profile.clone(),
            pinned: usage.is_pinned(profile.id),
            last_connected: usage.last_connected(profile.id),
        })
    };

    if !query.is_empty() {
        let mut quick_connect =
            quick_connect_args(query).map(|args| Item::Row(Row::QuickConnect { args }));
        let lead_with_quick_connect = looks_like_destination(query);
        // A typed destination also surfaces saved hosts on the same machine.
        let search_term = QuickConnectDestination::parse(query)
            .map(|destination| destination.host)
            .unwrap_or_else(|| query.to_string());
        let matches = profiles
            .iter()
            .filter(|profile| profile.matches_query(&search_term))
            .collect::<Vec<_>>();

        if lead_with_quick_connect {
            items.extend(quick_connect.take());
        }
        if !matches.is_empty() {
            items.push(Item::Header {
                title: "Matching hosts".to_string(),
                detail: matches.len().to_string(),
            });
            items.extend(matches.into_iter().map(host_row));
        }
        if !lead_with_quick_connect {
            items.extend(quick_connect);
        }
        if let Some(destination) = QuickConnectDestination::parse(query) {
            let already_saved = profiles.iter().any(|profile| {
                profile.host == destination.host
                    && profile.user == destination.user
                    && profile.port == destination.port
            });
            if !already_saved {
                items.push(Item::Header {
                    title: "Save this destination".to_string(),
                    detail: String::new(),
                });
                items.push(Item::Row(Row::SaveAndConnect {
                    input: query.to_string(),
                    destination,
                }));
            }
        }
        return items;
    }

    let find = |id: Uuid| profiles.iter().find(|profile| profile.id == id);
    let pinned = usage
        .pinned
        .iter()
        .filter_map(|id| find(*id))
        .collect::<Vec<_>>();
    if !pinned.is_empty() {
        items.push(Item::Header {
            title: "Pinned".to_string(),
            detail: pinned.len().to_string(),
        });
        items.extend(pinned.iter().map(|profile| host_row(profile)));
    }

    let recent = usage
        .recent_profiles()
        .filter(|(id, _)| !usage.is_pinned(*id))
        .filter_map(|(id, _)| find(id))
        .take(MAX_RECENT_HOSTS)
        .collect::<Vec<_>>();
    if !recent.is_empty() {
        items.push(Item::Header {
            title: "Recent".to_string(),
            detail: String::new(),
        });
        items.extend(recent.iter().map(|profile| host_row(profile)));
    }

    let quick_connects = usage
        .recent_quick_connects()
        .take(MAX_RECENT_QUICK_CONNECTS)
        .collect::<Vec<_>>();
    if !quick_connects.is_empty() {
        items.push(Item::Header {
            title: "Recent quick connects (not saved)".to_string(),
            detail: String::new(),
        });
        items.extend(quick_connects.into_iter().map(|(args, connected_at)| {
            Item::Row(Row::RecentQuickConnect {
                args: args.to_string(),
                connected_at,
            })
        }));
    }

    let shown = pinned
        .iter()
        .chain(&recent)
        .map(|profile| profile.id)
        .collect::<Vec<_>>();
    let others = profiles
        .iter()
        .filter(|profile| !shown.contains(&profile.id))
        .take(MAX_ALL_HOSTS)
        .collect::<Vec<_>>();
    if !profiles.is_empty() {
        items.push(Item::Header {
            title: format!("All hosts ({})", profiles.len()),
            detail: String::new(),
        });
        items.extend(others.into_iter().map(host_row));
        items.push(Item::Row(Row::ShowAll));
    }
    items
}

impl Entity for SshLauncherView {
    type Event = SshLauncherEvent;
}

impl TypedActionView for SshLauncherView {
    type Action = SshLauncherAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SshLauncherAction::Activate(index) => self.activate(*index, ctx),
            SshLauncherAction::SaveQuickConnect(args) => {
                ctx.dispatch_typed_action(&WorkspaceAction::SshManager(
                    SshManagerAction::SaveHost {
                        input: args.clone(),
                        connect: false,
                    },
                ));
            }
        }
    }
}

impl View for SshLauncherView {
    fn ui_name() -> &'static str {
        "SshLauncherView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let items = self.items(app);

        let input = Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(10.)
                .with_child(
                    ConstrainedBox::new(
                        Icon::Search
                            .to_warpui_icon(theme.sub_text_color(theme.background()))
                            .finish(),
                    )
                    .with_width(16.)
                    .with_height(16.)
                    .finish(),
                )
                .with_child(
                    Shrinkable::new(1., warpui::elements::ChildView::new(&self.editor).finish())
                        .finish(),
                )
                .finish(),
        )
        .with_horizontal_padding(ROW_PADDING_H)
        .with_vertical_padding(12.)
        .with_border(Border::bottom(1.).with_border_fill(theme.outline()))
        .finish();

        let mut list = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        let mut row_index = 0;
        for item in &items {
            match item {
                Item::Header { title, detail } => {
                    list = list.with_child(Self::render_header(title, detail, appearance));
                }
                Item::Row(row) => {
                    list = list.with_child(self.render_row(row_index, row, appearance, app));
                    row_index += 1;
                }
            }
        }
        if items.is_empty() {
            list = list.with_child(
                Container::new(
                    Text::new_inline(
                        "No saved hosts yet. Type user@host to connect.",
                        appearance.ui_font_family(),
                        12.,
                    )
                    .with_color(theme.sub_text_color(theme.background()).into())
                    .finish(),
                )
                .with_uniform_padding(ROW_PADDING_H)
                .finish(),
            );
        }

        let list = ConstrainedBox::new(
            ClippedScrollable::vertical(
                self.scroll_state.clone(),
                Container::new(list.finish())
                    .with_padding_bottom(6.)
                    .finish(),
                ScrollbarWidth::Custom(4.),
                theme.nonactive_ui_detail().into(),
                theme.active_ui_detail().into(),
                ElementFill::None,
            )
            .with_overlayed_scrollbar()
            .finish(),
        )
        .with_max_height(LIST_MAX_HEIGHT)
        .finish();

        ConstrainedBox::new(
            Container::new(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_child(input)
                    .with_child(list)
                    .with_child(Self::render_footer(appearance))
                    .finish(),
            )
            .with_background(ThemeFill::Solid(internal_colors::neutral_2(theme)))
            .finish(),
        )
        .with_width(LAUNCHER_WIDTH)
        .finish()
    }
}

#[cfg(test)]
#[path = "launcher_tests.rs"]
mod tests;
