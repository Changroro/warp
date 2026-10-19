use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::SyncSender;

use chrono::{NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use warp_core::user_preferences::GetUserPreferences as _;
use warp_errors::report_error;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use crate::app_state::{self, BranchSnapshot, LeafContents, LeafSnapshot, PaneNodeSnapshot};
use crate::launch_configs::launch_config::{PaneMode, PaneTemplateType, SplitDirection};
use crate::pane_group::PaneGroup;
use crate::persistence::ModelEvent;
use crate::persistence::model::ArchivedSession;
use crate::terminal::cli_agent::CLIAgent;
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;

const ARCHIVE_SECTION_COLLAPSED_KEY: &str = "SessionArchiveSectionCollapsed";

pub fn archive_section_collapsed(ctx: &AppContext) -> bool {
    ctx.private_user_preferences()
        .read_value(ARCHIVE_SECTION_COLLAPSED_KEY)
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or(true)
}

pub fn set_archive_section_collapsed(collapsed: bool, ctx: &AppContext) -> anyhow::Result<()> {
    ctx.private_user_preferences().write_value(
        ARCHIVE_SECTION_COLLAPSED_KEY,
        serde_json::to_string(&collapsed)?,
    )?;
    Ok(())
}

#[derive(Debug)]
pub enum SessionArchiveEvent {
    Changed,
}

/// A tab the user closed on purpose but wants to find again: where it ran, when
/// it was last touched, and which CLI agent drove it.
pub struct SessionArchiveModel {
    sessions: Vec<ArchivedSession>,
    model_event_sender: Option<SyncSender<ModelEvent>>,
}

impl Entity for SessionArchiveModel {
    type Event = SessionArchiveEvent;
}

impl SingletonEntity for SessionArchiveModel {}

impl SessionArchiveModel {
    pub fn new(
        persisted_sessions: Vec<ArchivedSession>,
        model_event_sender: Option<SyncSender<ModelEvent>>,
        _ctx: &mut ModelContext<Self>,
    ) -> Self {
        let mut model = Self {
            sessions: persisted_sessions,
            model_event_sender,
        };
        model.sort_by_user_order();
        model
    }

    pub fn sessions(&self) -> &[ArchivedSession] {
        &self.sessions
    }

    pub fn session(&self, id: &str) -> Option<&ArchivedSession> {
        self.sessions.iter().find(|session| session.id == id)
    }

    pub fn archive(&mut self, entry: NewArchiveEntry, ctx: &mut ModelContext<Self>) {
        let archived_at = Utc::now().naive_utc();
        let top_order = self
            .sessions
            .iter()
            .map(|session| session.sort_order)
            .min()
            .unwrap_or(0);
        let session = ArchivedSession {
            id: Uuid::new_v4().to_string(),
            title: entry.title,
            cwd: entry.cwd,
            cli_agent: entry.cli_agent.map(serialize_cli_agent),
            branch: entry.branch,
            last_used_at: entry.last_used_at.unwrap_or(archived_at),
            archived_at,
            sort_order: top_order - 1,
            layout: entry
                .layout
                .as_ref()
                .and_then(|layout| serde_json::to_string(layout).ok()),
        };

        self.send(ModelEvent::UpsertArchivedSession {
            session: session.clone(),
        });
        self.sessions.push(session);
        self.sort_by_user_order();
        ctx.emit(SessionArchiveEvent::Changed);
        ctx.notify();
    }

    /// Replaces the sessions whose ids start with `id_prefix` with `incoming`, keeping the place
    /// of the ones that were already listed and adding new ones at the top.
    pub fn replace_imported(
        &mut self,
        id_prefix: &str,
        incoming: Vec<ArchivedSession>,
        ctx: &mut ModelContext<Self>,
    ) {
        let incoming_ids: HashSet<_> = incoming.iter().map(|session| session.id.clone()).collect();
        let stale: Vec<_> = self
            .sessions
            .iter()
            .filter(|session| {
                session.id.starts_with(id_prefix) && !incoming_ids.contains(&session.id)
            })
            .map(|session| session.id.clone())
            .collect();
        for id in stale {
            self.sessions.retain(|session| session.id != id);
            self.send(ModelEvent::DeleteArchivedSession { id });
        }

        let mut top_order = self
            .sessions
            .iter()
            .map(|session| session.sort_order)
            .min()
            .unwrap_or(0);
        for mut session in incoming {
            match self
                .sessions
                .iter_mut()
                .find(|existing| existing.id == session.id)
            {
                Some(existing) => {
                    session.sort_order = existing.sort_order;
                    *existing = session.clone();
                }
                None => {
                    top_order -= 1;
                    session.sort_order = top_order;
                    self.sessions.push(session.clone());
                }
            }
            self.send(ModelEvent::UpsertArchivedSession { session });
        }
        self.sort_by_user_order();
        ctx.emit(SessionArchiveEvent::Changed);
        ctx.notify();
    }

    pub fn remove(&mut self, id: &str, ctx: &mut ModelContext<Self>) -> Option<ArchivedSession> {
        let index = self.sessions.iter().position(|session| session.id == id)?;
        let session = self.sessions.remove(index);
        self.send(ModelEvent::DeleteArchivedSession {
            id: session.id.clone(),
        });
        ctx.emit(SessionArchiveEvent::Changed);
        ctx.notify();
        Some(session)
    }

    fn sort_by_user_order(&mut self) {
        self.sessions.sort_by(|left, right| {
            left.sort_order
                .cmp(&right.sort_order)
                .then_with(|| right.last_used_at.cmp(&left.last_used_at))
        });
    }

    /// Moves `id` so it sits at `target_index` in the current list, then
    /// renumbers everything so the order survives a restart.
    pub fn reorder(&mut self, id: &str, target_index: usize, ctx: &mut ModelContext<Self>) {
        let Some(from) = self.sessions.iter().position(|session| session.id == id) else {
            log::warn!("Failed to reorder archived session: no session for id={id}");
            return;
        };
        let to = target_index.min(self.sessions.len().saturating_sub(1));
        if from == to {
            return;
        }

        let session = self.sessions.remove(from);
        self.sessions.insert(to, session);
        self.renumber(ctx);
        ctx.emit(SessionArchiveEvent::Changed);
        ctx.notify();
    }

    fn renumber(&mut self, _ctx: &mut ModelContext<Self>) {
        let mut changed = Vec::new();
        for (index, session) in self.sessions.iter_mut().enumerate() {
            let order = index as i32;
            if session.sort_order != order {
                session.sort_order = order;
                changed.push(session.clone());
            }
        }
        for session in changed {
            self.send(ModelEvent::UpsertArchivedSession { session });
        }
    }

    fn send(&self, event: ModelEvent) {
        if let Some(sender) = &self.model_event_sender
            && let Err(err) = sender.send(event)
        {
            report_error!(
                anyhow::Error::new(err).context("Failed to persist an archived session change")
            );
        }
    }
}

pub struct NewArchiveEntry {
    pub title: String,
    pub cwd: Option<String>,
    pub cli_agent: Option<CLIAgent>,
    pub branch: Option<String>,
    pub last_used_at: Option<NaiveDateTime>,
    pub layout: Option<ArchivedPane>,
}

/// The split layout of an archived tab, with what each terminal pane needs to come back.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ArchivedPane {
    Leaf {
        /// A local directory, or a directory on `ssh_profile`'s host.
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        agent: Option<CLIAgent>,
        /// The SSH manager profile the pane was connected to.
        #[serde(default)]
        ssh_profile: Option<Uuid>,
    },
    Split {
        horizontal: bool,
        panes: Vec<ArchivedPane>,
    },
}

/// One terminal pane of an [`ArchivedPane`] layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArchivedLeaf<'a> {
    pub cwd: Option<&'a str>,
    pub agent: Option<CLIAgent>,
    pub ssh_profile: Option<Uuid>,
}

impl ArchivedPane {
    /// The terminal panes, left to right and top to bottom.
    pub fn leaves(&self) -> Vec<ArchivedLeaf<'_>> {
        match self {
            ArchivedPane::Leaf {
                cwd,
                agent,
                ssh_profile,
            } => vec![ArchivedLeaf {
                cwd: cwd.as_deref(),
                agent: *agent,
                ssh_profile: *ssh_profile,
            }],
            ArchivedPane::Split { panes, .. } => panes.iter().flat_map(Self::leaves).collect(),
        }
    }

    /// A template that opens the same splits. Local panes start in their directory; SSH panes
    /// start locally and reach theirs once connected.
    pub fn template(&self) -> PaneTemplateType {
        match self {
            ArchivedPane::Leaf {
                cwd, ssh_profile, ..
            } => PaneTemplateType::PaneTemplate {
                cwd: cwd
                    .as_ref()
                    .filter(|_| ssh_profile.is_none())
                    .map(PathBuf::from)
                    .unwrap_or_default(),
                commands: Vec::new(),
                is_focused: None,
                pane_mode: PaneMode::Terminal,
                shell: None,
            },
            ArchivedPane::Split { horizontal, panes } => PaneTemplateType::PaneBranchTemplate {
                split_direction: if *horizontal {
                    SplitDirection::Horizontal
                } else {
                    SplitDirection::Vertical
                },
                panes: panes.iter().map(Self::template).collect(),
            },
        }
    }

    /// The same layout with every pane on `profile_id`'s host.
    pub fn on_ssh_host(self, profile_id: Uuid) -> Self {
        match self {
            ArchivedPane::Leaf { cwd, agent, .. } => ArchivedPane::Leaf {
                cwd,
                agent,
                ssh_profile: Some(profile_id),
            },
            ArchivedPane::Split { horizontal, panes } => ArchivedPane::Split {
                horizontal,
                panes: panes
                    .into_iter()
                    .map(|pane| pane.on_ssh_host(profile_id))
                    .collect(),
            },
        }
    }

    /// Collapses splits left with one pane, and returns `None` when no pane is left.
    pub fn from_panes(horizontal: bool, mut panes: Vec<ArchivedPane>) -> Option<Self> {
        match panes.len() {
            0 => None,
            1 => panes.pop(),
            _ => Some(ArchivedPane::Split { horizontal, panes }),
        }
    }
}

pub fn session_layout(session: &ArchivedSession) -> Option<ArchivedPane> {
    serde_json::from_str(session.layout.as_deref()?).ok()
}

/// The agent shown for an archived session: the one it recorded, else its first pane's.
pub fn session_agent(session: &ArchivedSession) -> Option<CLIAgent> {
    session
        .cli_agent
        .as_deref()
        .and_then(deserialize_cli_agent)
        .or_else(|| {
            session_layout(session)?
                .leaves()
                .into_iter()
                .find_map(|leaf| leaf.agent)
        })
}

/// The terminal panes of `pane_group` as an [`ArchivedPane`] layout. Other pane kinds are left out.
pub fn layout_of(pane_group: &PaneGroup, app: &AppContext) -> Option<ArchivedPane> {
    archived_node(&pane_group.snapshot(app), pane_group, app)
}

fn archived_node(
    node: &PaneNodeSnapshot,
    pane_group: &PaneGroup,
    app: &AppContext,
) -> Option<ArchivedPane> {
    match node {
        PaneNodeSnapshot::Branch(BranchSnapshot {
            direction,
            children,
        }) => ArchivedPane::from_panes(
            matches!(direction, app_state::SplitDirection::Horizontal),
            children
                .iter()
                .filter_map(|(_, child)| archived_node(child, pane_group, app))
                .collect(),
        ),
        PaneNodeSnapshot::Leaf(LeafSnapshot {
            contents: LeafContents::Terminal(terminal),
            ..
        }) => {
            let view = pane_group
                .find_terminal_pane_by_session_uuid(&terminal.uuid)
                .and_then(|pane_id| pane_group.terminal_view_from_pane_id(pane_id, app));
            let view = view.as_ref().map(|view| view.as_ref(app));
            let ssh_profile = view.and_then(|view| view.connected_ssh_profile_id());
            let cwd = match ssh_profile {
                Some(_) => view.and_then(|view| view.pwd()),
                None => terminal.cwd.clone(),
            };
            let agent = view.and_then(|view| {
                CLIAgentSessionsModel::as_ref(app)
                    .session(view.id())
                    .map(|session| session.agent)
                    .filter(|agent| !matches!(agent, CLIAgent::Unknown))
                    .or_else(|| crate::last_cli_agent::agent(view.id(), app))
            });
            Some(ArchivedPane::Leaf {
                cwd,
                agent,
                ssh_profile,
            })
        }
        PaneNodeSnapshot::Leaf(_) => None,
    }
}

fn serialize_cli_agent(agent: CLIAgent) -> String {
    serde_json::to_value(agent)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{agent:?}"))
}

pub fn deserialize_cli_agent(value: &str) -> Option<CLIAgent> {
    serde_json::from_value(serde_json::Value::String(value.to_string())).ok()
}

#[cfg(test)]
#[path = "session_archive_tests.rs"]
mod tests;
