//! Fork: keeps the saved window and tab layout from being lost.
//!
//! Upstream deletes the session database on logout. Here logout keeps it, the same account gets its
//! windows back after logging in again, and layout-only backups are kept next to the database.

use std::sync::mpsc::SyncSender;

use parking_lot::Mutex;
use warpui::{AppContext, SingletonEntity as _, WindowId};

use crate::auth::AuthStateProvider;
use crate::persistence::{self, ModelEvent};
use crate::root_view::{NewWorkspaceSource, quake_mode_window_id};
use crate::workspace::Workspace;

/// The account that was signed in when the user last logged out.
static LOGGED_OUT_USER: Mutex<Option<String>> = parking_lot::const_mutex(None);

fn current_user(app: &AppContext) -> Option<String> {
    AuthStateProvider::as_ref(app)
        .get()
        .user_id()
        .map(|uid| uid.as_string())
}

/// Whether the layout saved for `owner` must not be shown to `user`.
fn is_other_account(owner: Option<&str>, user: &str) -> bool {
    owner
        .map(str::trim)
        .is_some_and(|owner| !owner.is_empty() && owner != user)
}

/// Runs on logout in place of deleting the session database.
pub fn on_log_out(app: &AppContext) {
    *LOGGED_OUT_USER.lock() = current_user(app);
    #[cfg(feature = "local_fs")]
    backup::back_up_on_log_out();
}

/// Runs whenever a user is fetched. A different account than the one that owns the session
/// database starts from an empty database, as upstream does on logout; the old layout stays in a
/// backup.
pub fn on_user_fetched(sender: &Option<SyncSender<ModelEvent>>, app: &AppContext) {
    #[cfg(feature = "local_fs")]
    if let Some(user) = current_user(app)
        && backup::claim_database_for(&user)
    {
        persistence::remove(sender);
    }
    #[cfg(not(feature = "local_fs"))]
    let _ = app;
    persistence::reconstruct(sender);
}

/// The workspace a window opens once someone logs back in: the layout it has now.
pub fn workspace_source_after_log_out(
    workspace: &Workspace,
    window_id: WindowId,
    app: &AppContext,
) -> NewWorkspaceSource {
    let window_snapshot =
        workspace.snapshot(window_id, quake_mode_window_id() == Some(window_id), app);
    if window_snapshot.tabs.is_empty() {
        return empty_workspace();
    }
    NewWorkspaceSource::Restored {
        window_snapshot,
        block_lists: Default::default(),
    }
}

/// Drops a layout kept across logout when a different account logs in.
pub fn forget_layout_for_other_account(source: &mut NewWorkspaceSource, app: &AppContext) {
    if !matches!(source, NewWorkspaceSource::Restored { .. }) {
        return;
    }
    let logged_out = LOGGED_OUT_USER.lock().clone();
    if current_user(app).is_some_and(|user| is_other_account(logged_out.as_deref(), &user)) {
        *source = empty_workspace();
    }
}

fn empty_workspace() -> NewWorkspaceSource {
    NewWorkspaceSource::Empty {
        previous_active_window: None,
        shell: None,
    }
}

#[cfg(feature = "local_fs")]
pub use backup::start_auto_backups;

#[cfg(feature = "local_fs")]
mod backup {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use anyhow::{Context as _, Result};
    use chrono::Local;
    use diesel::connection::SimpleConnection as _;
    use diesel::{Connection as _, SqliteConnection};
    use warp_errors::report_if_error;

    use super::is_other_account;
    use crate::persistence::database_file_path_for_current_scope;

    const BACKUP_DIR: &str = "session-backups";
    /// Holds the user id of the account the session database belongs to.
    const OWNER_FILE: &str = "session-owner";
    const AUTO_PREFIX: &str = "auto-";
    const LOGOUT_PREFIX: &str = "logout-";
    const AUTO_BACKUPS_KEPT: usize = 30;
    const LOGOUT_BACKUPS_KEPT: usize = 10;
    const AUTO_BACKUP_INTERVAL: Duration = Duration::from_secs(10 * 60);

    /// The tables that hold the saved window, tab, and pane layout. Terminal output lives in
    /// other tables, which keeps each backup to a few kilobytes.
    const LAYOUT_TABLES: &[&str] = &[
        "app",
        "windows",
        "tab_groups",
        "tabs",
        "panels",
        "pane_nodes",
        "pane_branches",
        "pane_leaves",
        "terminal_panes",
        "notebook_panes",
        "code_panes",
        "code_pane_tabs",
        "env_var_collection_panes",
        "workflow_panes",
        "settings_panes",
        "ai_memory_panes",
        "ai_document_panes",
        "mcp_server_panes",
        "code_review_panes",
        "ambient_agent_panes",
        "active_mcp_servers",
        "archived_sessions",
    ];

    /// Backs up the layout now and then every [`AUTO_BACKUP_INTERVAL`] for the life of the app.
    pub fn start_auto_backups(database_path: PathBuf) {
        let spawned = std::thread::Builder::new()
            .name("Session layout backup".into())
            .spawn(move || {
                loop {
                    report_if_error!(back_up_layout(
                        &database_path,
                        AUTO_PREFIX,
                        AUTO_BACKUPS_KEPT
                    ));
                    std::thread::sleep(AUTO_BACKUP_INTERVAL);
                }
            });
        report_if_error!(spawned.context("Failed to start session layout backups"));
    }

    pub(super) fn back_up_on_log_out() {
        report_if_error!(back_up_layout(
            &database_file_path_for_current_scope(),
            LOGOUT_PREFIX,
            LOGOUT_BACKUPS_KEPT
        ));
    }

    /// Records `user` as the owner of the session database. Returns true when the database
    /// belonged to another account, after backing up its layout.
    pub(super) fn claim_database_for(user: &str) -> bool {
        let database_path = database_file_path_for_current_scope();
        let owner_path = database_path.with_file_name(OWNER_FILE);
        let owner = fs::read_to_string(&owner_path).ok();
        if owner.as_deref().map(str::trim) == Some(user) {
            return false;
        }
        let other_account = is_other_account(owner.as_deref(), user);
        if other_account {
            report_if_error!(back_up_layout(
                &database_path,
                LOGOUT_PREFIX,
                LOGOUT_BACKUPS_KEPT
            ));
        }
        report_if_error!(
            fs::write(&owner_path, user).context("Failed to record the session database owner")
        );
        other_account
    }

    /// Copies the layout tables of `database_path` into a new backup file named with `prefix`,
    /// unless the newest backup with that prefix already holds the same layout, and keeps the
    /// newest `keep` backups with that prefix.
    pub(super) fn back_up_layout(
        database_path: &Path,
        prefix: &str,
        keep: usize,
    ) -> Result<Option<PathBuf>> {
        if !database_path.exists() {
            return Ok(None);
        }
        let dir = database_path.with_file_name(BACKUP_DIR);
        fs::create_dir_all(&dir).context("Failed to create the session backup directory")?;
        let name = format!("{prefix}{}.sqlite", Local::now().format("%Y%m%d-%H%M%S"));
        let partial = dir.join(format!("{name}.partial"));
        let _ = fs::remove_file(&partial);
        copy_layout(database_path, &partial)?;

        let newest = backups(&dir, prefix).pop();
        if newest.is_some_and(|newest| fs::read(newest).ok() == fs::read(&partial).ok()) {
            fs::remove_file(&partial)?;
            return Ok(None);
        }
        let backup = dir.join(name);
        fs::rename(&partial, &backup).context("Failed to finish the session backup")?;
        let mut existing = backups(&dir, prefix);
        let excess = existing.len().saturating_sub(keep);
        for old in existing.drain(..excess) {
            let _ = fs::remove_file(old);
        }
        Ok(Some(backup))
    }

    fn copy_layout(database_path: &Path, target: &Path) -> Result<()> {
        let mut conn = SqliteConnection::establish(&target.to_string_lossy())
            .context("Failed to create the session backup")?;
        let source = database_path.to_string_lossy().replace('\'', "''");
        conn.batch_execute(&format!(
            "PRAGMA busy_timeout = 1000; ATTACH DATABASE '{source}' AS source;"
        ))
        .context("Failed to open the session database")?;
        let mut present = |table: &str| {
            conn.batch_execute(&format!("SELECT 1 FROM source.{table} LIMIT 0;"))
                .is_ok()
        };
        let tables: Vec<_> = LAYOUT_TABLES
            .iter()
            .filter(|table| present(table))
            .collect();
        let copies: String = tables
            .iter()
            .map(|table| format!("CREATE TABLE main.{table} AS SELECT * FROM source.{table};"))
            .collect();
        conn.batch_execute(&format!("BEGIN; {copies} COMMIT; DETACH DATABASE source;"))
            .context("Failed to copy the session layout")
    }

    /// Backups with `prefix` in `dir`, oldest first.
    fn backups(dir: &Path, prefix: &str) -> Vec<PathBuf> {
        let mut backups: Vec<_> = fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(prefix) && name.ends_with(".sqlite"))
            })
            .collect();
        backups.sort();
        backups
    }
}

#[cfg(test)]
#[path = "session_guard_tests.rs"]
mod tests;
