//! Fork: in the Tabs view, small agent logos under a tab's icon for its other split panes.

use std::cell::RefCell;
use std::collections::HashMap;

use warp_core::ui::theme::WarpTheme;
use warpui::AppContext;
use warpui::elements::{
    Container, CrossAxisAlignment, Element, Flex, Hoverable, MainAxisSize, MouseStateHandle,
    ParentElement,
};
use warpui::platform::Cursor;

use super::TypedPane;
use crate::pane_group::{PaneGroup, PaneId};
use crate::ui_components::agent_icon::terminal_view_agent_icon_variant;
use crate::ui_components::icon_with_status::{IconWithStatusVariant, render_icon_with_status};
use crate::workspace::PaneViewLocator;
use crate::workspace::action::WorkspaceAction;

const MAX_DOTS: usize = 3;
const DOT_SIZE: f32 = 16.;

pub(super) struct PaneAgent {
    pane_id: PaneId,
    variant: IconWithStatusVariant,
    mouse_state: MouseStateHandle,
}

/// The agents of `visible_pane_ids` other than `shown_pane_id`, in pane order.
pub(super) fn other_pane_agents(
    pane_group: &PaneGroup,
    visible_pane_ids: &[PaneId],
    shown_pane_id: PaneId,
    mouse_states: &RefCell<HashMap<PaneId, MouseStateHandle>>,
    app: &AppContext,
) -> Vec<PaneAgent> {
    visible_pane_ids
        .iter()
        .filter(|pane_id| **pane_id != shown_pane_id)
        .filter_map(|pane_id| {
            let TypedPane::Terminal(terminal_pane) = pane_group.resolve_pane_type(*pane_id, app)
            else {
                return None;
            };
            // Status badges are unreadable at this size, so only the logo is kept.
            let variant = match terminal_view_agent_icon_variant(
                terminal_pane.terminal_view(app).as_ref(app),
                app,
            )? {
                IconWithStatusVariant::CLIAgent {
                    agent, is_ambient, ..
                } => IconWithStatusVariant::CLIAgent {
                    agent,
                    status: None,
                    is_ambient,
                },
                IconWithStatusVariant::OzAgent { is_ambient, .. } => {
                    IconWithStatusVariant::OzAgent {
                        status: None,
                        is_ambient,
                    }
                }
                IconWithStatusVariant::Neutral { .. }
                | IconWithStatusVariant::NeutralElement { .. }
                | IconWithStatusVariant::CustomAvatar { .. } => return None,
            };
            Some(PaneAgent {
                pane_id: *pane_id,
                variant,
                mouse_state: mouse_states
                    .borrow_mut()
                    .entry(*pane_id)
                    .or_default()
                    .clone(),
            })
        })
        .take(MAX_DOTS)
        .collect()
}

/// `icon` with a column of `agents` under it; clicking one focuses its pane.
pub(super) fn with_other_pane_agents(
    icon: Box<dyn Element>,
    agents: Vec<PaneAgent>,
    pane_group_id: warpui::EntityId,
    theme: &WarpTheme,
) -> Box<dyn Element> {
    if agents.is_empty() {
        return icon;
    }
    let mut column = Flex::column()
        .with_main_axis_size(MainAxisSize::Min)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(3.)
        .with_child(icon);
    for agent in agents {
        let PaneAgent {
            pane_id,
            variant,
            mouse_state,
        } = agent;
        column.add_child(
            Container::new(
                Hoverable::new(mouse_state, |_| {
                    render_icon_with_status(variant, DOT_SIZE, 0., theme, theme.background())
                })
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(WorkspaceAction::FocusPane(PaneViewLocator {
                        pane_group_id,
                        pane_id,
                    }));
                })
                .with_cursor(Cursor::PointingHand)
                .finish(),
            )
            .finish(),
        );
    }
    column.finish()
}
