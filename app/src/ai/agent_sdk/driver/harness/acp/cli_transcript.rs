//! The CLI behind an ACP adapter writes the same session transcript it writes when run in a
//! terminal, so the terminal runners' capture code applies unchanged once that session is
//! identified. ACP treats session ids as opaque, so how an adapter's session id relates to the
//! CLI's is decided per harness in [`CliTranscriptKind::cli_session_id`]. A transcript that
//! cannot be identified costs the run its CLI transcript upload and nothing else.
//!
//! One gap: when claude-agent-acp restarts a session to clear its context after a plan is
//! approved, the SDK continues under a fresh id that the adapter never reports. Captures keep
//! reading the transcript under the ACP session id, which stops at the restart.
use std::path::{Path, PathBuf};

use anyhow::Result;
use uuid::Uuid;
use warp_cli::agent::Harness;

use super::super::claude_transcript::{claude_config_dir, session_transcript_path};
use super::super::codex_transcript::{codex_sessions_root, find_session_file};
use super::super::transcript_persistence::{CapturedTranscript, UploadedTranscriptUsage};
use super::super::usage_reporting::CaptureIdentity;
use super::super::{claude_code, codex};

/// The CLI transcript format an ACP harness produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CliTranscriptKind {
    Codex,
    Claude,
}

impl CliTranscriptKind {
    /// The harnesses whose CLI transcript is uploaded for an ACP run.
    pub(super) fn for_harness(harness: Harness) -> Option<Self> {
        match harness {
            Harness::Codex => Some(Self::Codex),
            Harness::Claude => Some(Self::Claude),
            Harness::Gemini | Harness::Oz | Harness::OpenCode | Harness::Unknown => None,
        }
    }

    /// The CLI's own session id behind an ACP session, which names its transcript on disk.
    /// `None` when the adapter's id cannot be mapped to one.
    pub(super) fn cli_session_id(self, acp_session_id: &str) -> Option<Uuid> {
        match self {
            // codex-acp returns the Codex thread id from `session/new`, and claude-agent-acp
            // passes its session id to the Agent SDK as the Claude session id.
            Self::Codex | Self::Claude => Uuid::parse_str(acp_session_id).ok(),
        }
    }

    /// The directory the CLI keeps its transcripts under, from the same environment the adapter
    /// inherits.
    pub(super) fn root(self) -> Result<PathBuf> {
        match self {
            Self::Codex => codex_sessions_root(),
            Self::Claude => claude_config_dir(),
        }
    }

    /// The transcript file, once the CLI has written it. `root` is [`Self::root`].
    pub(super) fn locate(self, root: &Path, cwd: &Path, session_id: Uuid) -> Option<PathBuf> {
        match self {
            Self::Codex => find_session_file(root, session_id),
            Self::Claude => {
                Some(session_transcript_path(root, cwd, session_id)).filter(|path| path.exists())
            }
        }
    }

    /// Captures the transcript at `path` exactly as the terminal runner for this CLI does.
    pub(super) fn capture(
        self,
        session_id: Uuid,
        path: &Path,
        root: &Path,
        cwd: &Path,
        is_final: bool,
        identity: Option<CaptureIdentity>,
    ) -> Result<CapturedTranscript> {
        match self {
            Self::Codex => {
                codex::capture_transcript_with_usage(session_id, path, is_final, identity)
            }
            Self::Claude => claude_code::capture_transcript_with_usage(
                session_id, cwd, root, None, is_final, identity,
            ),
        }
    }

    /// The save result when the transcript does not exist. The CLI writes it only after the
    /// first prompt, so its absence during the run is expected. Its absence at the final save is
    /// not, but the CLI transcript supplements the native conversation the run already uploads,
    /// so it never fails the save.
    pub(super) fn missing_transcript(self, is_final: bool) -> UploadedTranscriptUsage {
        if is_final {
            log::warn!(
                "{self:?} transcript still unavailable at final save; it was never uploaded"
            );
        } else {
            log::debug!("{self:?} transcript for the ACP session is not available yet");
        }
        UploadedTranscriptUsage::empty()
    }
}

#[cfg(test)]
#[path = "cli_transcript_tests.rs"]
mod tests;
