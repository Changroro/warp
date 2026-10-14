//! Rendering helpers shared by the SSH launcher, hosts panel, and session UI.

use pathfinder_color::ColorU;
use warp_core::ui::theme::AnsiColorIdentifier;
use warp_core::ui::theme::color::internal_colors;
use warpui::Element;
use warpui::elements::{Border, ConstrainedBox, Container, CornerRadius, Empty, Radius, Text};
use warpui::fonts::{Properties, Weight};

use crate::appearance::Appearance;
use crate::ssh_manager::profile::SshHostProfile;
use crate::themes::theme::Fill as ThemeFill;
use crate::ui_components::color_dot::TAB_COLOR_OPTIONS;

pub const BADGE_SIZE: f32 = 22.;

// ponytail: the color is derived from the group name; add a per-profile color field if users
// want to pick it.
/// The color shared by every host in `profile`'s group, also used for its terminal tabs.
pub fn host_color(profile: &SshHostProfile) -> Option<AnsiColorIdentifier> {
    let group = profile.group()?;
    // FNV-1a, so that short names spread across the few colors.
    let hash = group.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    Some(TAB_COLOR_OPTIONS[(hash % TAB_COLOR_OPTIONS.len() as u64) as usize])
}

pub fn ansi_color(color: AnsiColorIdentifier, appearance: &Appearance) -> ColorU {
    color
        .to_ansi_color(&appearance.theme().terminal_colors().normal)
        .into()
}

/// Two letters naming the host's group, or the host itself when it has no group.
pub fn badge_label(profile: &SshHostProfile) -> String {
    profile
        .group()
        .unwrap_or(&profile.name)
        .chars()
        .filter(|c| c.is_alphanumeric())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}

/// `user@host:port`, plus the first jump host.
pub fn host_subtitle(profile: &SshHostProfile) -> String {
    let mut subtitle = format!("{}:{}", profile.target(), profile.port);
    if let Some(jump_host) = profile.jump_hosts.first() {
        subtitle.push_str(" · via ");
        subtitle.push_str(&jump_host.host);
    }
    subtitle
}

pub fn render_badge(
    label: &str,
    color: Option<AnsiColorIdentifier>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let (background, text_color) = match color {
        Some(color) => (
            ansi_color(color, appearance),
            theme.background().into_solid(),
        ),
        None => (
            internal_colors::fg_overlay_3(theme).into_solid(),
            theme.sub_text_color(theme.background()).into_solid(),
        ),
    };
    ConstrainedBox::new(
        Container::new(
            Text::new_inline(label.to_string(), appearance.monospace_font_family(), 10.)
                .with_color(text_color)
                .with_style(Properties::default().weight(Weight::Semibold))
                .finish(),
        )
        .with_background(ThemeFill::Solid(background))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(5.)))
        .with_horizontal_padding(4.)
        .with_vertical_padding(4.)
        .finish(),
    )
    .with_width(BADGE_SIZE)
    .with_height(BADGE_SIZE)
    .finish()
}

pub fn render_chip(text: &str, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    Container::new(
        Text::new_inline(text.to_string(), appearance.ui_font_family(), 11.)
            .with_color(theme.sub_text_color(theme.background()).into())
            .finish(),
    )
    .with_background(internal_colors::fg_overlay_2(theme))
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(9.)))
    .with_horizontal_padding(7.)
    .with_vertical_padding(1.)
    .finish()
}

pub fn render_kbd(text: &str, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    Container::new(
        Text::new_inline(text.to_string(), appearance.monospace_font_family(), 11.)
            .with_color(theme.sub_text_color(theme.background()).into())
            .finish(),
    )
    .with_background(internal_colors::fg_overlay_1(theme))
    .with_border(Border::all(1.).with_border_fill(theme.outline()))
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
    .with_horizontal_padding(5.)
    .finish()
}

pub fn render_dot(color: ColorU, size: f32) -> Box<dyn Element> {
    ConstrainedBox::new(
        Container::new(Empty::new().finish())
            .with_background(ThemeFill::Solid(color))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(size / 2.)))
            .finish(),
    )
    .with_width(size)
    .with_height(size)
    .finish()
}

#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;
