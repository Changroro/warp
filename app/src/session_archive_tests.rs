use chrono::NaiveDate;
use enum_iterator::all;
use uuid::Uuid;
use warpui::App;

use super::{
    ArchivedLeaf, ArchivedPane, SessionArchiveModel, deserialize_cli_agent, serialize_cli_agent,
    session_agent, session_layout,
};
use crate::launch_configs::launch_config::{PaneTemplateType, SplitDirection};
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
        layout: None,
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

fn leaf(cwd: &str, agent: Option<CLIAgent>) -> ArchivedPane {
    ArchivedPane::Leaf {
        cwd: Some(cwd.to_string()),
        agent,
        ssh_profile: None,
    }
}

fn shape(template: &PaneTemplateType) -> String {
    match template {
        PaneTemplateType::PaneTemplate { cwd, .. } => format!("{}", cwd.display()),
        PaneTemplateType::PaneBranchTemplate {
            split_direction,
            panes,
        } => format!(
            "{}[{}]",
            match split_direction {
                SplitDirection::Horizontal => "row",
                SplitDirection::Vertical => "column",
            },
            panes.iter().map(shape).collect::<Vec<_>>().join(", ")
        ),
    }
}

#[test]
fn an_archived_layout_reopens_with_the_same_splits_and_pane_order() {
    let layout = ArchivedPane::from_panes(
        true,
        vec![
            leaf("/srv/api", Some(CLIAgent::Claude)),
            ArchivedPane::from_panes(
                false,
                vec![
                    leaf("/srv/web", None),
                    leaf("/srv/db", Some(CLIAgent::Codex)),
                ],
            )
            .unwrap(),
        ],
    )
    .unwrap();

    assert_eq!(
        shape(&layout.template()),
        "row[/srv/api, column[/srv/web, /srv/db]]"
    );
    let agents: Vec<_> = layout.leaves().iter().map(|leaf| leaf.agent).collect();
    assert_eq!(
        agents,
        [Some(CLIAgent::Claude), None, Some(CLIAgent::Codex)]
    );
    assert_eq!(ArchivedPane::from_panes(true, vec![]), None);
    assert_eq!(
        ArchivedPane::from_panes(true, vec![leaf("/a", None)]),
        Some(leaf("/a", None))
    );
}

#[test]
fn ssh_panes_open_locally_and_keep_their_remote_directory_for_the_connection() {
    let profile = Uuid::new_v4();
    let layout = leaf("/home/remote/app", Some(CLIAgent::Claude)).on_ssh_host(profile);

    assert_eq!(shape(&layout.template()), "");
    assert_eq!(
        layout.leaves(),
        [ArchivedLeaf {
            cwd: Some("/home/remote/app"),
            agent: Some(CLIAgent::Claude),
            ssh_profile: Some(profile),
        }]
    );
}

#[test]
fn a_stored_layout_round_trips_and_supplies_the_agent_when_none_was_recorded() {
    let mut stored = session("split", 1);
    let layout = ArchivedPane::from_panes(
        false,
        vec![leaf("/a", None), leaf("/b", Some(CLIAgent::Codex))],
    )
    .unwrap();
    stored.layout = Some(serde_json::to_string(&layout).unwrap());

    assert_eq!(session_layout(&stored), Some(layout));
    assert_eq!(session_agent(&stored), Some(CLIAgent::Codex));

    stored.cli_agent = Some(serialize_cli_agent(CLIAgent::Claude));
    assert_eq!(session_agent(&stored), Some(CLIAgent::Claude));

    stored.layout = Some(r#"{"kind":"leaf"}"#.to_string());
    assert_eq!(
        session_layout(&stored),
        Some(ArchivedPane::Leaf {
            cwd: None,
            agent: None,
            ssh_profile: None
        }),
        "fields added later default when an older layout is read"
    );
}

#[test]
fn importing_replaces_only_the_entries_from_the_same_host() {
    App::test((), |mut app| async move {
        let archive = app.add_singleton_model(|ctx| {
            SessionArchiveModel::new(
                vec![
                    ordered_session("local", 0),
                    ordered_session("ssh:a:kept", 1),
                    ordered_session("ssh:a:gone", 2),
                    ordered_session("ssh:b:other", 3),
                ],
                None,
                ctx,
            )
        });

        archive.update(&mut app, |archive, ctx| {
            let mut kept = session("ssh:a:kept", 5);
            kept.title = "renamed".to_string();
            archive.replace_imported("ssh:a:", vec![kept, session("ssh:a:new", 6)], ctx);
        });

        archive.read(&app, |archive, _| {
            assert_eq!(
                ids(archive),
                vec!["ssh:a:new", "local", "ssh:a:kept", "ssh:b:other"]
            );
            assert_eq!(archive.session("ssh:a:kept").unwrap().title, "renamed");
        });
    })
}
