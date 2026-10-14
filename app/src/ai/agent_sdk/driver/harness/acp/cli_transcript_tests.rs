use std::fs;
use std::path::Path;

use tempfile::TempDir;
use uuid::Uuid;

use super::super::super::claude_transcript::encode_cwd;
use super::CliTranscriptKind;

fn write(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "{\"type\":\"session_meta\"}\n").unwrap();
}

#[test]
fn codex_transcript_is_the_rollout_named_after_the_acp_session_id() {
    let sessions_root = TempDir::new().unwrap();
    let cwd = Path::new("/workspace");
    let session_id = Uuid::new_v4();
    let kind = CliTranscriptKind::Codex;
    assert_eq!(kind.locate(sessions_root.path(), cwd, session_id), None);

    let rollout = sessions_root
        .path()
        .join("2026/10/09")
        .join(format!("rollout-2026-10-09T17-00-00-{session_id}.jsonl"));
    write(&rollout);
    write(&sessions_root.path().join("2026/10/09").join(format!(
        "rollout-2026-10-09T16-00-00-{}.jsonl",
        Uuid::new_v4()
    )));

    assert_eq!(
        kind.locate(sessions_root.path(), cwd, session_id),
        Some(rollout)
    );
}

#[test]
fn claude_transcript_is_the_project_session_file_named_after_the_acp_session_id() {
    let config_dir = TempDir::new().unwrap();
    let cwd = Path::new("/workspace/repo");
    let session_id = Uuid::new_v4();
    let kind = CliTranscriptKind::Claude;
    assert_eq!(kind.locate(config_dir.path(), cwd, session_id), None);

    let transcript = config_dir
        .path()
        .join("projects")
        .join(encode_cwd(cwd))
        .join(format!("{session_id}.jsonl"));
    write(&transcript);

    assert_eq!(
        kind.locate(config_dir.path(), cwd, session_id),
        Some(transcript)
    );
    assert_eq!(
        kind.locate(config_dir.path(), Path::new("/elsewhere"), session_id),
        None,
        "the transcript is keyed by the session's working directory"
    );
}
