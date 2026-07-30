use std::sync::mpsc::SyncSender;

use chrono::{NaiveDateTime, Utc};
use uuid::Uuid;
use warp_errors::report_error;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::persistence::ModelEvent;
use crate::persistence::model::ArchivedSession;
use crate::terminal::cli_agent::CLIAgent;

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
        model.sort_most_recent_first();
        model
    }

    pub fn sessions(&self) -> &[ArchivedSession] {
        &self.sessions
    }

    pub fn archive(&mut self, entry: NewArchiveEntry, ctx: &mut ModelContext<Self>) {
        let archived_at = Utc::now().naive_utc();
        let session = ArchivedSession {
            id: Uuid::new_v4().to_string(),
            title: entry.title,
            cwd: entry.cwd,
            cli_agent: entry.cli_agent.map(serialize_cli_agent),
            branch: entry.branch,
            last_used_at: entry.last_used_at.unwrap_or(archived_at),
            archived_at,
        };

        self.send(ModelEvent::UpsertArchivedSession {
            session: session.clone(),
        });
        self.sessions.push(session);
        self.sort_most_recent_first();
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

    fn sort_most_recent_first(&mut self) {
        self.sessions
            .sort_by(|left, right| right.last_used_at.cmp(&left.last_used_at));
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
