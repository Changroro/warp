use chrono::NaiveDate;
use enum_iterator::all;

use super::{SessionArchiveModel, deserialize_cli_agent, serialize_cli_agent};
use crate::persistence::model::ArchivedSession;
use crate::terminal::cli_agent::CLIAgent;

fn session(id: &str, day: u32) -> ArchivedSession {
    let timestamp = NaiveDate::from_ymd_opt(2026, 7, day)
        .unwrap()
        .and_hms_opt(9, 0, 0)
        .unwrap();
    ArchivedSession {
        id: id.to_string(),
        title: id.to_string(),
        cwd: None,
        cli_agent: None,
        branch: None,
        last_used_at: timestamp,
        archived_at: timestamp,
    }
}

fn model(sessions: Vec<ArchivedSession>) -> SessionArchiveModel {
    let mut model = SessionArchiveModel {
        sessions,
        model_event_sender: None,
    };
    model.sort_most_recent_first();
    model
}

#[test]
fn sessions_are_ordered_most_recently_used_first() {
    let model = model(vec![session("older", 10), session("newer", 28)]);

    let ids: Vec<&str> = model
        .sessions()
        .iter()
        .map(|session| session.id.as_str())
        .collect();
    assert_eq!(ids, vec!["newer", "older"]);
}

#[test]
fn every_cli_agent_survives_a_storage_roundtrip() {
    for agent in all::<CLIAgent>() {
        let stored = serialize_cli_agent(agent);
        assert_eq!(
            deserialize_cli_agent(&stored),
            Some(agent),
            "{agent:?} did not survive a roundtrip through {stored}"
        );
    }
}

#[test]
fn an_unknown_stored_agent_is_dropped_instead_of_guessed() {
    assert_eq!(deserialize_cli_agent("Nonexistent"), None);
    assert_eq!(deserialize_cli_agent(""), None);
}
