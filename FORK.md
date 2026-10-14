# Fork notes

This checkout is a personal fork of `warpdotdev/warp`. `origin` is the fork, `upstream` is Warp.
Fork work lands on `dev`; `master` tracks upstream.

## Syncing with upstream

```bash
./script/fork-sync
```

The script fetches `upstream/master`, merges it into the current branch, then runs
`cargo check -p warp --all-targets` and the fork's tests. It stops at the first failure and never
pushes. On a merge conflict, resolve it, `git commit`, and rerun the script.

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
| `app/src/workspace/view.rs` | state field, constructor, toolbar button/panel arms, action arm, modal |
| `app/src/workspace/header_toolbar_item.rs` | `HeaderToolbarItemKind::SshProfiles` |
| `app/src/terminal/view.rs` | state field, warpify-suppression call, auto-inject activation, clear on warpify |
| `app/src/terminal/model/terminal_model.rs` | `ignore_bootstrapping_messages_until_command_finished` and its checks in the bootstrap hooks |
| `app/src/terminal/ssh/util.rs` | `-l` login name parsing into `InteractiveSshCommand::user`, `split_ssh_destination` |
| `app/src/settings/ssh.rs` | `saved_ssh_host_profiles` setting |
| `app/src/editor/view/mod.rs` | `set_is_password` / `is_password` |
| `Cargo.toml`, `app/Cargo.toml` | `zeroize` dependency |

## Session archive

Not yet split out; its code still sits inside upstream files.

Code: `app/src/session_archive.rs`. Hooks: `app/src/lib.rs`, `app/src/tab.rs`,
`app/src/workspace/action.rs`, `app/src/workspace/view.rs`, `app/src/workspace/view/vertical_tabs.rs`,
`app/src/persistence/{mod,sqlite}.rs`, `crates/persistence/src/{model,schema}.rs`,
`crates/persistence/migrations/2026-07-30-*`, and the test setup in `app/src/test_util/terminal.rs`
and `app/src/workspace/view_tests.rs`.
