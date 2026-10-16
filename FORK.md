# Fork notes

This checkout is a personal fork of `warpdotdev/warp`. `origin` is the fork, `upstream` is Warp.

## Branches

- `master` mirrors `upstream/master`. Never commit to it.
- `feat/*` each hold one fork feature. Branch new features from `master`, commit there, then
  merge the branch into `dev`.
- `dev` is the integration branch we run: `master` plus every feature branch. Don't commit to it
  directly.

Active feature branches (keep `FEATURE_BRANCHES` in `script/fork-sync` in sync):

| Branch | Feature |
| --- | --- |
| `feat/session-archive` | Archive closed sessions in the vertical tabs panel |
| `feat/ssh-manager` | SSH host manager. It was branched from `dev` and already contains `feat/session-archive` and `feat/agent-logos` (remote tabs keep their agent logos), so branch an upstream PR from `master` instead. |
| `feat/session-guard` | Keep the saved session layout across logout, plus layout-only backups |
| `feat/agent-logos` | Remember each pane's last CLI agent and show split panes' agents in vertical tabs |

## Syncing with upstream

```bash
./script/fork-sync
```

The script fast-forwards `master` to `upstream/master`, merges `master` into each feature branch,
merges each feature branch into `dev`, then runs `cargo check -p warp --all-targets` and the fork's
tests on `dev`. It stops at the first failure and never pushes. On a merge conflict, resolve it,
`git commit`, and rerun the script. When it finishes, push the branches it prints.

## Releases

Push a `fork-vYYYY.MM.DD` tag (add `.N` for another release the same day) on `dev`.
`.github/workflows/fork_release.yml` builds the OSS channel as a Linux `.deb` and `.AppImage`
(x86_64) and an ad-hoc signed Apple Silicon `.dmg`, then publishes them as a GitHub prerelease.
rpm is skipped because upstream's bundle script signs it with Warp's key on CI. Upstream's own
workflows are disabled in this fork's Actions settings so syncing `master` doesn't run Warp's CI.

Fork tooling (`FORK.md`, `script/fork-sync`, the release workflow) and the fork READMEs (the intro
block above `## About` in `README.md`, and `README.ko.md`) live on `feat/ssh-manager`.

## Layout rule

Fork features live in fork-owned files. Upstream files only get hooks: a field, a constructor
call, a match arm, or a one-line delegation into the fork module. New files never conflict on
merge, so the cost of a sync is the number of hook lines, not the size of the feature.

When adding to a fork feature, put the code in its module and extend an existing hook before
adding a new one. List any new upstream file you touch below.

## SSH manager

Code: `app/src/ssh_manager/`, plus the child modules `app/src/terminal/view/ssh_manager.rs` and
`app/src/workspace/view/ssh_manager.rs`, which hold the `impl TerminalView` / `impl Workspace`
methods so they can reach private upstream fields without widening their visibility.

Hooks in upstream files (`rg -n 'ssh_manager|ssh_profile|SshManager|SshProfiles' <file>`):

| File | Hook |
| --- | --- |
| `app/src/lib.rs` | `mod ssh_manager;` and the `run_ssh_askpass_if_requested` re-export |
| `app/src/workspace/mod.rs` | `crate::ssh_manager::init(app)` registers bindings |
| `app/src/workspace/action.rs` | `WorkspaceAction::SshManager` variant and its save-state arm |
| `app/src/workspace/view.rs` | state field, constructor, toolbar button/panel arms, action arm, modal, SSH section in `unified_new_session_menu_items` |
| `app/src/workspace/header_toolbar_item.rs` | `HeaderToolbarItemKind::SshProfiles` |
| `app/src/terminal/view.rs` | state field, `handle_ssh_manager_block_started` on `AfterBlockStarted`, `handle_ssh_manager_precmd` on `BlockMetadataReceived`, `paste_image_over_ssh` in `paste`, `wrap_with_ssh_session_bar` in `render`, `askpass::strip_env_prefix` on both `warpify_command`s so ssh behind the password helper's env prefix is still detected |
| `app/src/pane_group/pane/terminal_pane.rs` | `set_ssh_manager_attached` on attach and detach, so closed tabs kept for undo don't count as open |
| `app/src/terminal/model/session.rs` | `Session::ssh_wrapper_socket_path` accessor for image uploads over the SSH wrapper's ControlMaster |
| `app/src/bin/oss.rs` | enables `FeatureFlag::SshRemoteServer` (remote file browsing) without `release_bundle`; exits early as ssh's `SSH_ASKPASS` helper |
| `app/src/terminal/model/terminal_model.rs` | `ignore_bootstrapping_messages_until_command_finished` and its checks in the bootstrap hooks |
| `app/src/terminal/ssh/util.rs` | `-l` login name parsing into `InteractiveSshCommand::user`, `split_ssh_destination` |
| `app/src/settings/ssh.rs` | `saved_ssh_host_profiles` and `ssh_host_usage` settings |
| `app/src/editor/view/mod.rs` | `set_is_password` / `is_password` |
| `crates/warpui_extras/src/secure_storage/mod.rs` | `platform()` opens secure storage without an app, for the password helper |
| `Cargo.toml`, `app/Cargo.toml` | `zeroize` dependency |

## Session archive

Not yet split out; its code still sits inside upstream files.

Code: `app/src/session_archive.rs`. Hooks: `app/src/lib.rs`, `app/src/tab.rs`,
`app/src/workspace/action.rs`, `app/src/workspace/view.rs`, `app/src/workspace/view/vertical_tabs.rs`,
`app/src/persistence/{mod,sqlite}.rs`, `crates/persistence/src/{model,schema}.rs`,
`crates/persistence/migrations/2026-07-30-*`, and the test setup in `app/src/test_util/terminal.rs`
and `app/src/workspace/view_tests.rs`.

## Session guard

Code: `app/src/session_guard.rs`. Backups go to `session-backups/` and the owning account's user id
to `session-owner`, both next to `warp.sqlite`. To restore a backup, quit Warp, then copy each
table from the backup into `warp.sqlite` (`ATTACH` the backup, then `DELETE` and `INSERT ... SELECT`
per table in one transaction).

| File | Hook |
| --- | --- |
| `app/src/lib.rs` | `mod session_guard;` |
| `app/src/auth/mod.rs` | `session_guard::on_log_out` replaces `persistence::remove` in `log_out` |
| `app/src/auth/auth_manager.rs` | `session_guard::on_user_fetched` replaces `persistence::reconstruct` |
| `app/src/root_view.rs` | `workspace_source_after_log_out` in `AuthOnboardingState::log_out`, `forget_layout_for_other_account` in `complete_auth_and_create_workspace` |
| `app/src/workspace/global_actions.rs` | `save_app` skips snapshots without windows |
| `app/src/persistence/sqlite.rs` | `start_auto_backups` in `initialize` for the app scope |

## Agent logos

Code: `app/src/last_cli_agent.rs` and `app/src/workspace/view/vertical_tabs/agent_dots.rs`. The
last agent per pane is stored in `pane-cli-agents.json` next to `warp.sqlite`.

| File | Hook |
| --- | --- |
| `app/src/lib.rs` | `mod last_cli_agent;` and `last_cli_agent::init` after `CLIAgentSessionsModel` |
| `app/src/pane_group/pane/terminal_pane.rs` | `register_pane` in `TerminalPane::new` |
| `app/src/ui_components/agent_icon.rs` | `last_cli_agent::icon_variant` fallback in `terminal_view_agent_icon_variant` |
| `app/src/workspace/view/vertical_tabs.rs` | `mod agent_dots;`, `PaneProps::other_pane_agents`, `agent_dot_mouse_states`, the Tabs-view fill in `render_tab_group`, and the icon wrap in `render_pane_row` |
