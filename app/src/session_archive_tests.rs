use chrono::NaiveDate;
use enum_iterator::all;

use super::{SessionArchiveModel, deserialize_cli_agent, serialize_cli_agent};
use crate::persistence::model::ArchivedSession;
use crate::terminal::cli_agent::CLIAgent;

fn ordered_session(id: &str, order: i32) -> ArchivedSession {
    let mut session = session(id, 1);
    session.sort_order = order;
    session
}

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
        sort_order: 0,
    }
}

fn model(sessions: Vec<ArchivedSession>) -> SessionArchiveModel {
    let mut model = SessionArchiveModel {
        sessions,
        model_event_sender: None,
    };
    model.sort_by_user_order();
    model
}

fn ids(model: &SessionArchiveModel) -> Vec<&str> {
    model
        .sessions()
        .iter()
        .map(|session| session.id.as_str())
        .collect()
}

#[test]
fn sessions_follow_the_order_the_user_arranged() {
    let model = model(vec![
        ordered_session("third", 2),
        ordered_session("first", 0),
        ordered_session("second", 1),
    ]);

    assert_eq!(ids(&model), vec!["first", "second", "third"]);
}

#[test]
fn sessions_sharing_an_order_fall_back_to_most_recently_used() {
    let model = model(vec![session("older", 10), session("newer", 28)]);

    assert_eq!(ids(&model), vec!["newer", "older"]);
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
