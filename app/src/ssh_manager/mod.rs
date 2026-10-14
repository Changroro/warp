//! Fork-owned SSH host manager. Upstream files only hold thin hooks into this module; see
//! `FORK.md` for the list.

pub mod auto_inject;
pub mod config_file;
pub mod control_master;
pub mod host_editor;
pub mod launcher;
pub mod login_prompt;
pub mod open_connections;
pub mod panel;
pub mod profile;
pub mod reconnect;
pub mod reconnect_banner;
pub mod ui;
pub mod usage;

use uuid::Uuid;
use warpui::AppContext;
use warpui::keymap::{BindingDescription, EditableBinding};

use crate::util::bindings;
use crate::workspace::WorkspaceAction;

pub const TOGGLE_PANEL_BINDING_NAME: &str = "workspace:toggle_ssh_profiles_panel";
pub const LAUNCHER_BINDING_NAME: &str = "workspace:ssh_launcher";

#[derive(Debug, Clone)]
pub enum SshManagerAction {
    TogglePanel,
    OpenPanel,
    /// Opens the launcher palette, which searches hosts and takes `ssh` arguments.
    OpenLauncher,
    /// Opens a new terminal tab and runs the saved SSH profile command.
    Connect(Uuid),
    /// Opens a new terminal tab and runs `ssh` with the given arguments.
    QuickConnect(String),
    /// Saves a `[user@]host[:port]` quick-connect entry as a profile.
    SaveHost {
        input: String,
        connect: bool,
    },
}

impl SshManagerAction {
    pub fn should_save_app_state_on_action(&self) -> bool {
        match self {
            SshManagerAction::Connect(_)
            | SshManagerAction::QuickConnect(_)
            | SshManagerAction::SaveHost { .. } => true,
            SshManagerAction::TogglePanel
            | SshManagerAction::OpenPanel
            | SshManagerAction::OpenLauncher => false,
        }
    }
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    open_connections::SshOpenConnections::handle_or_init(app);

    app.register_editable_bindings([
        EditableBinding::new(
            TOGGLE_PANEL_BINDING_NAME,
            BindingDescription::new("Toggle SSH profiles panel")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "Toggle SSH Profiles Panel"),
            WorkspaceAction::SshManager(SshManagerAction::TogglePanel),
        )
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_context_predicate(id!("Workspace")),
        EditableBinding::new(
            LAUNCHER_BINDING_NAME,
            BindingDescription::new("Connect to SSH host")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "Connect to SSH Host"),
            WorkspaceAction::SshManager(SshManagerAction::OpenLauncher),
        )
        .with_key_binding("cmdorctrl-shift-H")
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_context_predicate(id!("Workspace")),
    ]);
}
