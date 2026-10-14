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
| `feat/ssh-manager` | SSH host manager. It was branched from `dev` and already contains `feat/session-archive`, so branch an upstream PR from `master` instead. |

## Syncing with upstream

```bash
./script/fork-sync
```

The script fast-forwards `master` to `upstream/master`, merges `master` into each feature branch,
merges each feature branch into `dev`, then runs `cargo check -p warp --all-targets` and the fork's
tests on `dev`. It stops at the first failure and never pushes. On a merge conflict, resolve it,
`git commit`, and rerun the script. When it finishes, push the branches it prints.

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
| `app/src/lib.rs` | `mod ssh_manager;` |
| `app/src/workspace/mod.rs` | `crate::ssh_manager::init(app)` registers bindings |
| `app/src/workspace/action.rs` | `WorkspaceAction::SshManager` variant and its save-state arm |
| `app/src/workspace/view.rs` | state field, constructor, toolbar button/panel arms, action arm, modal, SSH section in `unified_new_session_menu_items` |
| `app/src/workspace/header_toolbar_item.rs` | `HeaderToolbarItemKind::SshProfiles` |
| `app/src/terminal/view.rs` | state field, `handle_ssh_manager_block_started` on `AfterBlockStarted`, `handle_ssh_manager_precmd` on `BlockMetadataReceived`, `paste_image_over_ssh` in `paste`, `wrap_with_ssh_session_bar` in `render`, auto-inject activation, clear on warpify |
| `app/src/terminal/model/terminal_model.rs` | `ignore_bootstrapping_messages_until_command_finished` and its checks in the bootstrap hooks |
| `app/src/terminal/ssh/util.rs` | `-l` login name parsing into `InteractiveSshCommand::user`, `split_ssh_destination` |
| `app/src/settings/ssh.rs` | `saved_ssh_host_profiles` and `ssh_host_usage` settings |
| `app/src/editor/view/mod.rs` | `set_is_password` / `is_password` |
| `Cargo.toml`, `app/Cargo.toml` | `zeroize` dependency |

## Session archive

Not yet split out; its code still sits inside upstream files.

Code: `app/src/session_archive.rs`. Hooks: `app/src/lib.rs`, `app/src/tab.rs`,
`app/src/workspace/action.rs`, `app/src/workspace/view.rs`, `app/src/workspace/view/vertical_tabs.rs`,
`app/src/persistence/{mod,sqlite}.rs`, `crates/persistence/src/{model,schema}.rs`,
`crates/persistence/migrations/2026-07-30-*`, and the test setup in `app/src/test_util/terminal.rs`
and `app/src/workspace/view_tests.rs`.
