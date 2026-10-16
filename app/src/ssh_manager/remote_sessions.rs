//! Reads the tabs of the WarpOss app running on an SSH host from that host's session database, so
//! they can be reopened here as SSH tabs with the same titles, colors, splits, and agent logos.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Context as _;
use serde::Deserialize;

use crate::launch_configs::launch_config::{PaneMode, PaneTemplateType, SplitDirection};
use crate::tab::SelectedTabColor;
use crate::terminal::cli_agent::CLIAgent;
use crate::themes::theme::AnsiColorIdentifier;

/// Prints the path of the most recently written WarpOss session database on the host, the last
/// CLI agent of each of its panes as one line of JSON, then its pane tree as JSON. Each desktop
/// workspace can have its own database under `~/.local/state`.
const REMOTE_SCRIPT: &str = r#"
command -v sqlite3 >/dev/null 2>&1 || { echo "sqlite3 is not installed on this host" >&2; exit 3; }
newest=""
newest_mtime=0
for db in "$HOME/.local/state/warp-oss/warp.sqlite" "$HOME"/.local/state/*/warp-oss/warp.sqlite "$HOME/Library/Application Support/dev.warp.WarpOss/warp.sqlite"; do
  [ -f "$db" ] || continue
  for file in "$db" "$db-wal"; do
    [ -f "$file" ] || continue
    mtime=$(stat -c %Y "$file" 2>/dev/null || stat -f %m "$file")
    if [ "$mtime" -gt "$newest_mtime" ]; then newest_mtime=$mtime; newest=$db; fi
  done
done
[ -n "$newest" ] || { echo "WarpOss has no session database on this host" >&2; exit 4; }
printf "%s\n" "$newest"
agents="$(dirname "$newest")/pane-cli-agents.json"
if [ -f "$agents" ]; then tr -d '\n' < "$agents"; fi
printf "\n"
exec sqlite3 -readonly -json "$newest" "SELECT n.id, n.tab_id, n.parent_pane_node_id AS parent, n.is_leaf, b.horizontal, l.kind, p.cwd, lower(hex(p.uuid)) AS uuid, t.custom_title AS title, t.color FROM pane_nodes n JOIN tabs t ON t.id = n.tab_id LEFT JOIN pane_branches b ON b.pane_node_id = n.id LEFT JOIN pane_leaves l ON l.pane_node_id = n.id LEFT JOIN terminal_panes p ON p.id = n.id ORDER BY t.window_id, t.id, n.id"
"#;

#[cfg(not(target_family = "wasm"))]
const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

pub struct RemoteSessions {
    /// Path of the session database on the host.
    pub database: String,
    pub tabs: Vec<RemoteTab>,
}

pub struct RemoteTab {
    pub title: Option<String>,
    pub color: Option<AnsiColorIdentifier>,
    /// Terminal panes only. Every leaf opens in the local default directory.
    pub layout: PaneTemplateType,
    /// Each leaf of `layout`, left to right and top to bottom.
    pub panes: Vec<RemotePane>,
}

#[derive(Debug, PartialEq)]
pub struct RemotePane {
    pub directory: Option<String>,
    /// The last CLI agent the pane ran on the host.
    pub agent: Option<CLIAgent>,
}

#[derive(Deserialize)]
struct NodeRow {
    id: i64,
    tab_id: i64,
    parent: Option<i64>,
    is_leaf: i64,
    horizontal: Option<i64>,
    kind: Option<String>,
    cwd: Option<String>,
    uuid: Option<String>,
    title: Option<String>,
    color: Option<String>,
}

pub fn parse_remote_output(output: &str) -> anyhow::Result<RemoteSessions> {
    let (database, rest) = output
        .split_once('\n')
        .context("the host printed no session database path")?;
    let (agents, json) = rest
        .split_once('\n')
        .context("the host printed no pane agents")?;
    // An unreadable agents file only costs the logos.
    let agents: HashMap<String, CLIAgent> = serde_json::from_str(agents).unwrap_or_default();
    let rows: Vec<NodeRow> = if json.trim().is_empty() {
        Vec::new()
    } else {
        serde_json::from_str(json).context("the session database query returned malformed JSON")?
    };

    let mut tabs = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        if row.parent.is_some() || rows[..index].iter().any(|seen| seen.tab_id == row.tab_id) {
            continue;
        }
        let tab_rows = rows
            .iter()
            .filter(|node| node.tab_id == row.tab_id)
            .collect::<Vec<_>>();
        let mut panes = Vec::new();
        if let Some(layout) = build_pane(row, &tab_rows, &agents, &mut panes) {
            tabs.push(RemoteTab {
                title: row.title.clone().filter(|title| !title.is_empty()),
                color: row.color.as_deref().and_then(parse_tab_color),
                layout,
                panes,
            });
        }
    }
    Ok(RemoteSessions {
        database: database.to_string(),
        tabs,
    })
}

fn build_pane(
    node: &NodeRow,
    tab_rows: &[&NodeRow],
    agents: &HashMap<String, CLIAgent>,
    panes: &mut Vec<RemotePane>,
) -> Option<PaneTemplateType> {
    if node.is_leaf != 0 {
        if node.kind.as_deref() != Some("terminal") {
            return None;
        }
        panes.push(RemotePane {
            directory: node.cwd.clone().filter(|cwd| !cwd.is_empty()),
            agent: node
                .uuid
                .as_ref()
                .and_then(|uuid| agents.get(uuid))
                .copied(),
        });
        return Some(PaneTemplateType::PaneTemplate {
            cwd: PathBuf::new(),
            commands: Vec::new(),
            is_focused: None,
            pane_mode: PaneMode::Terminal,
            shell: None,
        });
    }

    let mut panes = tab_rows
        .iter()
        .filter(|child| child.parent == Some(node.id))
        .filter_map(|child| build_pane(child, tab_rows, agents, panes))
        .collect::<Vec<_>>();
    match panes.len() {
        0 => None,
        1 => panes.pop(),
        _ => Some(PaneTemplateType::PaneBranchTemplate {
            split_direction: if node.horizontal == Some(1) {
                SplitDirection::Horizontal
            } else {
                SplitDirection::Vertical
            },
            panes,
        }),
    }
}

/// Tab colors are stored as YAML, either a `SelectedTabColor` or a bare color from older versions.
fn parse_tab_color(value: &str) -> Option<AnsiColorIdentifier> {
    match serde_yaml::from_str::<SelectedTabColor>(value) {
        Ok(SelectedTabColor::Color(color)) => Some(color),
        Ok(SelectedTabColor::Unset | SelectedTabColor::Cleared) => None,
        Err(_) => serde_yaml::from_str(value).ok(),
    }
}

/// Runs [`REMOTE_SCRIPT`] on the host through the ControlMaster of a live connection to it, so
/// nothing has to authenticate again.
#[cfg(not(target_family = "wasm"))]
pub async fn fetch(control_path: PathBuf) -> anyhow::Result<RemoteSessions> {
    use anyhow::{anyhow, bail};
    use warpui::r#async::FutureExt as _;

    use super::control_master::escape_ssh_tokens;
    use super::profile::shell_quote_arg;

    let output = command::r#async::Command::new("ssh")
        .args([
            "-o",
            &format!("ControlPath={}", escape_ssh_tokens(&control_path)),
        ])
        .args(["-o", "ControlMaster=no"])
        .args(["-o", "BatchMode=yes"])
        .args(["-o", "ProxyCommand=none"])
        .args(["-o", "ProxyJump=none"])
        .args(["-o", "ConnectTimeout=15"])
        .arg("--")
        // The host is ignored while the ControlMaster is up, and `.invalid` never resolves if not.
        .arg("warp-control-master.invalid")
        .arg(format!("sh -c {}", shell_quote_arg(REMOTE_SCRIPT)))
        .kill_on_drop(true)
        .output()
        .with_timeout(FETCH_TIMEOUT)
        .await
        .map_err(|_| anyhow!("ssh timed out after {FETCH_TIMEOUT:?}"))?
        .context("failed to run ssh")?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    parse_remote_output(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(target_family = "wasm")]
pub async fn fetch(_: PathBuf) -> anyhow::Result<RemoteSessions> {
    anyhow::bail!("Reading remote Warp sessions is not supported on this platform")
}

#[cfg(test)]
#[path = "remote_sessions_tests.rs"]
mod tests;
