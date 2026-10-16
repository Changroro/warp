use super::*;

fn started(terminal_view_id: EntityId, agent: CLIAgent) -> CLIAgentSessionsModelEvent {
    CLIAgentSessionsModelEvent::Started {
        terminal_view_id,
        agent,
    }
}

fn model(path: PathBuf) -> LastCliAgents {
    LastCliAgents {
        by_pane: HashMap::new(),
        pane_by_view: HashMap::new(),
        path: Some(path),
    }
}

#[test]
fn the_last_agent_of_a_pane_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pane-cli-agents.json");
    let (claude_view, codex_view) = (EntityId::new(), EntityId::new());

    let mut before = model(path.clone());
    before.pane_by_view.insert(claude_view, hex(&[0xab, 0x01]));
    before.pane_by_view.insert(codex_view, hex(&[0xab, 0x02]));
    before.handle_session_event(&started(claude_view, CLIAgent::Claude));
    before.handle_session_event(&started(codex_view, CLIAgent::Codex));
    before.handle_session_event(&CLIAgentSessionsModelEvent::Ended {
        terminal_view_id: claude_view,
        agent: CLIAgent::Claude,
    });
    assert_eq!(before.agent(claude_view), Some(CLIAgent::Claude));

    let mut after = model(path.clone());
    after.by_pane = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let restored_view = EntityId::new();
    after.pane_by_view.insert(restored_view, hex(&[0xab, 0x01]));
    assert_eq!(after.agent(restored_view), Some(CLIAgent::Claude));
}

#[test]
fn unknown_agents_and_unregistered_views_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = model(dir.path().join("pane-cli-agents.json"));
    let view = EntityId::new();
    model.handle_session_event(&started(view, CLIAgent::Claude));
    assert!(model.by_pane.is_empty());

    model.pane_by_view.insert(view, hex(&[1]));
    model.handle_session_event(&started(view, CLIAgent::Unknown));
    assert_eq!(model.agent(view), None);
}
