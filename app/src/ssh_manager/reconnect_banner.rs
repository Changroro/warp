use warp_core::ui::theme::AnsiColorIdentifier;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Empty, Expanded, Flex,
    Hoverable, MainAxisSize, MouseStateHandle, ParentElement, Radius, Shrinkable, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::platform::Cursor;
use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use crate::Appearance;
use crate::ssh_manager::reconnect::{AUTO_RECONNECT_DELAY, MAX_AUTO_RECONNECT_ATTEMPTS};
use crate::ssh_manager::ui::{ansi_color, render_dot};
use crate::themes::theme::Fill as ThemeFill;
use crate::ui_components::icons::Icon;

#[derive(Clone, Debug)]
pub enum SshReconnectBannerAction {
    Reconnect,
    StopAutoReconnect,
    Dismiss,
}

#[derive(Clone, Debug)]
pub enum SshReconnectBannerEvent {
    Reconnect,
    StopAutoReconnect,
    Dismissed,
}

/// Countdown to the next automatic reconnect.
#[derive(Clone, Copy, Debug)]
pub struct AutoReconnectCountdown {
    pub seconds_left: u64,
    pub attempt: u32,
}

/// Shown in the block list after an SSH connection opened from the SSH manager ends.
pub struct SshReconnectBanner {
    target: String,
    reason: Option<String>,
    is_network_failure: bool,
    disconnected_at: String,
    countdown: Option<AutoReconnectCountdown>,
    reconnect_mouse_state: MouseStateHandle,
    stop_mouse_state: MouseStateHandle,
    close_mouse_state: MouseStateHandle,
}

impl SshReconnectBanner {
    pub fn new(
        target: String,
        reason: Option<String>,
        is_network_failure: bool,
        disconnected_at: String,
    ) -> Self {
        Self {
            target,
            reason,
            is_network_failure,
            disconnected_at,
            countdown: None,
            reconnect_mouse_state: Default::default(),
            stop_mouse_state: Default::default(),
            close_mouse_state: Default::default(),
        }
    }

    pub fn set_countdown(
        &mut self,
        countdown: Option<AutoReconnectCountdown>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.countdown = countdown;
        ctx.notify();
    }
}

impl Entity for SshReconnectBanner {
    type Event = SshReconnectBannerEvent;
}

fn render_button(
    label: String,
    primary: bool,
    mouse_state: MouseStateHandle,
    action: SshReconnectBannerAction,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    Hoverable::new(mouse_state, move |state| {
        let (background, text) = if primary {
            (
                if state.is_hovered() {
                    internal_colors::accent_overlay_4(theme)
                } else {
                    theme.accent()
                },
                theme.background().into_solid(),
            )
        } else {
            (
                if state.is_hovered() {
                    internal_colors::fg_overlay_3(theme)
                } else {
                    internal_colors::fg_overlay_2(theme)
                },
                theme.active_ui_text_color().into_solid(),
            )
        };
        Container::new(
            Text::new_inline(label.clone(), appearance.ui_font_family(), 12.5)
                .with_color(text)
                .with_style(Properties::default().weight(if primary {
                    Weight::Semibold
                } else {
                    Weight::Normal
                }))
                .finish(),
        )
        .with_background(background)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(5.)))
        .with_horizontal_padding(12.)
        .with_vertical_padding(5.)
        .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
    .finish()
}

impl View for SshReconnectBanner {
    fn ui_name() -> &'static str {
        "SshReconnectBanner"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let red = ansi_color(AnsiColorIdentifier::Red, appearance);
        let yellow = ansi_color(AnsiColorIdentifier::Yellow, appearance);
        let muted = theme.sub_text_color(theme.background()).into_solid();
        let accent_color = if self.is_network_failure { red } else { muted };

        let title = if self.is_network_failure {
            format!("Connection to {} lost", self.target)
        } else {
            format!("Connection to {} closed", self.target)
        };
        let mut detail = self.reason.clone().into_iter().collect::<Vec<_>>();
        detail.push(self.disconnected_at.clone());
        if let Some(countdown) = self.countdown {
            detail.push(format!(
                "reconnecting in {}s ({}/{MAX_AUTO_RECONNECT_ATTEMPTS})",
                countdown.seconds_left, countdown.attempt
            ));
        }

        let text = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(2.)
            .with_child(
                Text::new_inline(title, appearance.ui_font_family(), 13.)
                    .with_color(if self.is_network_failure {
                        red
                    } else {
                        theme.active_ui_text_color().into_solid()
                    })
                    .with_style(Properties::default().weight(Weight::Semibold))
                    .finish(),
            )
            .with_child(
                Text::new(detail.join(" · "), appearance.monospace_font_family(), 11.5)
                    .soft_wrap(true)
                    .with_color(muted)
                    .finish(),
            )
            .finish();

        let mut row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(12.)
            .with_child(render_dot(accent_color, 10.))
            .with_child(Expanded::new(1., text).finish());

        if let Some(countdown) = self.countdown {
            let total = AUTO_RECONNECT_DELAY.as_secs().max(1) as f32;
            let elapsed = (total - countdown.seconds_left as f32).clamp(0., total);
            row = row.with_child(
                ConstrainedBox::new(
                    Container::new(
                        ConstrainedBox::new(
                            Container::new(Empty::new().finish())
                                .with_background(ThemeFill::Solid(yellow))
                                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(2.)))
                                .finish(),
                        )
                        .with_width(90. * elapsed / total)
                        .with_height(4.)
                        .finish(),
                    )
                    .with_background(internal_colors::fg_overlay_3(theme))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(2.)))
                    .finish(),
                )
                .with_width(90.)
                .with_height(4.)
                .finish(),
            );
        }

        row = row.with_child(render_button(
            "Reconnect".to_string(),
            true,
            self.reconnect_mouse_state.clone(),
            SshReconnectBannerAction::Reconnect,
            appearance,
        ));
        if self.countdown.is_some() {
            row = row.with_child(render_button(
                "Stop auto-reconnect".to_string(),
                false,
                self.stop_mouse_state.clone(),
                SshReconnectBannerAction::StopAutoReconnect,
                appearance,
            ));
        }
        row = row.with_child(
            Hoverable::new(self.close_mouse_state.clone(), move |_| {
                ConstrainedBox::new(Icon::X.to_warpui_icon(muted.into()).finish())
                    .with_width(16.)
                    .with_height(16.)
                    .finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(SshReconnectBannerAction::Dismiss))
            .finish(),
        );

        let border = if self.is_network_failure {
            ThemeFill::Solid(red).with_opacity(50)
        } else {
            theme.outline()
        };
        let background = if self.is_network_failure {
            ThemeFill::Solid(red).with_opacity(8)
        } else {
            internal_colors::fg_overlay_1(theme)
        };
        Container::new(
            Container::new(Shrinkable::new(1., row.finish()).finish())
                .with_background(background)
                .with_border(Border::all(1.).with_border_fill(border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .with_horizontal_padding(14.)
                .with_vertical_padding(12.)
                .finish(),
        )
        .with_horizontal_padding(12.)
        .with_vertical_padding(8.)
        .finish()
    }
}

impl TypedActionView for SshReconnectBanner {
    type Action = SshReconnectBannerAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SshReconnectBannerAction::Reconnect => ctx.emit(SshReconnectBannerEvent::Reconnect),
            SshReconnectBannerAction::StopAutoReconnect => {
                ctx.emit(SshReconnectBannerEvent::StopAutoReconnect)
            }
            SshReconnectBannerAction::Dismiss => ctx.emit(SshReconnectBannerEvent::Dismissed),
        }
    }
}
