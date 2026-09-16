use std::sync::mpsc::SyncSender;

use chrono::{NaiveDateTime, Utc};
use uuid::Uuid;
use warp_errors::report_error;
use warpui::{Entity, ModelContext, SingletonEntity};

use warp_core::user_preferences::GetUserPreferences as _;
use warpui::AppContext;

use crate::persistence::ModelEvent;
use crate::persistence::model::ArchivedSession;
use crate::terminal::cli_agent::CLIAgent;

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
        };

        self.send(ModelEvent::UpsertArchivedSession {
            session: session.clone(),
        });
        self.sessions.push(session);
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
