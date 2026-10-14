//! Fork-owned SSH host manager. Upstream files only hold thin hooks into this module; see
//! `FORK.md` for the list.

pub mod auto_inject;
pub mod config_file;
pub mod login_prompt;
pub mod panel;
pub mod profile;

use uuid::Uuid;
use warpui::AppContext;
use warpui::keymap::{BindingDescription, EditableBinding};

use crate::util::bindings;
use crate::workspace::WorkspaceAction;

pub const TOGGLE_PANEL_BINDING_NAME: &str = "workspace:toggle_ssh_profiles_panel";

#[derive(Debug, Clone)]
pub enum SshManagerAction {
    TogglePanel,
    /// Opens a new terminal tab and runs the saved SSH profile command.
    Connect(Uuid),
}

impl SshManagerAction {
    pub fn should_save_app_state_on_action(&self) -> bool {
        match self {
            SshManagerAction::Connect(_) => true,
            SshManagerAction::TogglePanel => false,
        }
    }
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_editable_bindings([EditableBinding::new(
        TOGGLE_PANEL_BINDING_NAME,
        BindingDescription::new("Toggle SSH profiles panel")
            .with_custom_description(bindings::MAC_MENUS_CONTEXT, "Toggle SSH Profiles Panel"),
        WorkspaceAction::SshManager(SshManagerAction::TogglePanel),
    )
    .with_group(bindings::BindingGroup::Navigation.as_str())
    .with_context_predicate(id!("Workspace"))]);
}
