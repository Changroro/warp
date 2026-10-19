//! Fork: remembers the last CLI agent each terminal pane ran, keyed by the pane's persisted uuid,
//! so the pane keeps that agent's logo after the agent exits and after Warp restarts.

use std::collections::HashMap;
use std::path::PathBuf;

use warpui::{AppContext, Entity, EntityId, ModelContext, SingletonEntity};

use crate::terminal::cli_agent::CLIAgent;
use crate::terminal::cli_agent_sessions::{CLIAgentSessionsModel, CLIAgentSessionsModelEvent};
use crate::ui_components::icon_with_status::IconWithStatusVariant;

pub struct LastCliAgents {
    /// Hex pane uuid to the last agent that ran in it.
    by_pane: HashMap<String, CLIAgent>,
    /// Terminal view to its pane's hex uuid, for panes created in this run.
    pane_by_view: HashMap<EntityId, String>,
    path: Option<PathBuf>,
}

impl Entity for LastCliAgents {
    type Event = ();
}

impl SingletonEntity for LastCliAgents {}

pub fn init(app: &mut AppContext) {
    app.add_singleton_model(LastCliAgents::new);
}

/// Ties a terminal view to the persisted uuid of the pane that holds it.
pub fn register_pane(terminal_view_id: EntityId, pane_uuid: &[u8], app: &mut AppContext) {
    if !app.has_singleton_model::<LastCliAgents>() {
        return;
    }
    LastCliAgents::handle(app).update(app, |model, _| {
        model.pane_by_view.insert(terminal_view_id, hex(pane_uuid));
    });
}

/// Records `agent` as the last agent of the terminal's pane, as if it had run there.
pub fn remember(terminal_view_id: EntityId, agent: CLIAgent, app: &mut AppContext) {
    if !app.has_singleton_model::<LastCliAgents>() {
        return;
    }
    LastCliAgents::handle(app).update(app, |model, _| model.record(terminal_view_id, agent));
}

/// The last agent the terminal ran.
pub fn agent(terminal_view_id: EntityId, app: &AppContext) -> Option<CLIAgent> {
    if !app.has_singleton_model::<LastCliAgents>() {
        return None;
    }
    LastCliAgents::as_ref(app).agent(terminal_view_id)
}

/// The logo of the last agent the terminal ran, shown when no agent is running now.
pub fn icon_variant(terminal_view_id: EntityId, app: &AppContext) -> Option<IconWithStatusVariant> {
    let agent = agent(terminal_view_id, app)?;
    Some(IconWithStatusVariant::CLIAgent {
        agent,
        status: None,
        is_ambient: false,
    })
}

impl LastCliAgents {
    fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&CLIAgentSessionsModel::handle(ctx), |me, _, event, _| {
            me.handle_session_event(event);
        });
        let path = storage_path();
        let by_pane = path
            .as_ref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            by_pane,
            pane_by_view: HashMap::new(),
            path,
        }
    }

    fn agent(&self, terminal_view_id: EntityId) -> Option<CLIAgent> {
        self.by_pane
            .get(self.pane_by_view.get(&terminal_view_id)?)
            .copied()
    }

    fn handle_session_event(&mut self, event: &CLIAgentSessionsModelEvent) {
        let agent = match event {
            CLIAgentSessionsModelEvent::Started { agent, .. }
            | CLIAgentSessionsModelEvent::StatusChanged { agent, .. }
            | CLIAgentSessionsModelEvent::InputSessionChanged { agent, .. }
            | CLIAgentSessionsModelEvent::SessionUpdated { agent, .. } => *agent,
            CLIAgentSessionsModelEvent::Ended { .. } => return,
        };
        self.record(event.terminal_view_id(), agent);
    }

    fn record(&mut self, terminal_view_id: EntityId, agent: CLIAgent) {
        if matches!(agent, CLIAgent::Unknown) {
            return;
        }
        let Some(pane) = self.pane_by_view.get(&terminal_view_id) else {
            return;
        };
        // shortcut: entries for closed panes are never pruned; they cost a few dozen bytes each.
        if self.by_pane.insert(pane.clone(), agent) != Some(agent) {
            self.save();
        }
    }

    fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };
        match serde_json::to_vec(&self.by_pane) {
            Ok(bytes) => {
                if let Err(err) = std::fs::write(path, bytes) {
                    log::warn!("Failed to save the last CLI agent of each pane: {err}");
                }
            }
            Err(err) => log::warn!("Failed to encode the last CLI agent of each pane: {err}"),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn storage_path() -> Option<PathBuf> {
    #[cfg(feature = "local_fs")]
    {
        Some(
            crate::persistence::database_file_path_for_current_scope()
                .with_file_name("pane-cli-agents.json"),
        )
    }
    #[cfg(not(feature = "local_fs"))]
    {
        None
    }
}

#[cfg(test)]
#[path = "last_cli_agent_tests.rs"]
mod tests;
