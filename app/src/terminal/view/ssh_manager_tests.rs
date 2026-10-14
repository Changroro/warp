use warpui::App;

use super::*;
use crate::terminal::model::ansi::{Handler as _, InitShellValue};
use crate::terminal::model_events::ModelEvent;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

const SSH_PROFILE_TEST_COMMAND: &str = "command ssh -p 2222 -- root@10.0.0.5";

fn start_block_with_command(
    view: &mut TerminalView,
    command: &str,
    ctx: &mut ViewContext<TerminalView>,
) {
    let block_id = view.model.lock().block_list().active_block_id().clone();
    view.handle_terminal_event(
        &ModelEvent::AfterBlockStarted {
            block_id,
            command: command.to_string(),
            is_for_in_band_command: false,
        },
        ctx,
    );
}

/// Drives the remote `InitShell` hook and reports whether the model acted on it. An honored hook
/// re-initializes the shell and appends a block; a suppressed one returns before that. This
/// observes suppression through public block-list state instead of the private model flag.
fn model_honors_remote_init_shell(view: &TerminalView) -> bool {
    let mut model = view.model.lock();
    let blocks_before = model.block_list().blocks().len();
    model.init_shell(InitShellValue {
        session_id: Default::default(),
        shell: "bash".to_string(),
        is_subshell: true,
        user: "root".to_string(),
        hostname: "remote".to_string(),
        wsl_name: None,
    });
    model.block_list().blocks().len() > blocks_before
}

#[test]
fn arming_an_ssh_profile_with_warpify_disabled_records_the_rendered_command() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            view.arm_ssh_profile_connection(SSH_PROFILE_TEST_COMMAND.to_string(), false, ctx);

            assert!(!view.ssh_profile.warpification_enabled_at_arm);
            assert_eq!(
                view.ssh_profile.warpify_disabled_command.as_deref(),
                Some(SSH_PROFILE_TEST_COMMAND)
            );
        });
    })
}

#[test]
fn arming_an_ssh_profile_with_warpify_enabled_records_no_suppression_command() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            view.arm_ssh_profile_connection(SSH_PROFILE_TEST_COMMAND.to_string(), true, ctx);

            assert!(view.ssh_profile.warpification_enabled_at_arm);
            assert_eq!(view.ssh_profile.warpify_disabled_command, None);
        });
    })
}

#[test]
fn observing_the_armed_command_suppresses_bootstrap_when_warpify_is_disabled() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            assert!(
                view.model.lock().block_list().is_bootstrapped(),
                "the InitShell probe only re-initializes the shell on a bootstrapped block list"
            );

            view.arm_ssh_profile_connection(SSH_PROFILE_TEST_COMMAND.to_string(), false, ctx);
            start_block_with_command(view, SSH_PROFILE_TEST_COMMAND, ctx);

            assert!(
                !model_honors_remote_init_shell(view),
                "the armed profile command must suppress remote bootstrap hooks"
            );
        });
    })
}

#[test]
fn observing_the_armed_command_does_not_suppress_bootstrap_when_warpify_is_enabled() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            view.arm_ssh_profile_connection(SSH_PROFILE_TEST_COMMAND.to_string(), true, ctx);
            start_block_with_command(view, SSH_PROFILE_TEST_COMMAND, ctx);

            assert!(
                model_honors_remote_init_shell(view),
                "Warpify-enabled profile connections must bootstrap normally"
            );
        });
    })
}

#[test]
fn observing_an_unrelated_command_does_not_suppress_bootstrap() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            view.arm_ssh_profile_connection(SSH_PROFILE_TEST_COMMAND.to_string(), false, ctx);
            start_block_with_command(view, "ls -la", ctx);

            assert!(
                model_honors_remote_init_shell(view),
                "suppression must not leak onto commands the profile did not launch"
            );
        });
    })
}

const SSH_PROFILE_BARE_COMMAND: &str = "ssh -p 2222 -- root@10.0.0.5";

fn arm_warpify_disabled_profile(
    view: &mut TerminalView,
    command: &str,
    ctx: &mut ViewContext<TerminalView>,
) {
    view.arm_ssh_profile_connection(command.to_string(), false, ctx);
}

#[test]
fn suppression_matches_the_armed_command_through_the_command_builtin_prefix() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            arm_warpify_disabled_profile(view, SSH_PROFILE_BARE_COMMAND, ctx);
            start_block_with_command(view, "command ssh -p 2222 -- root@10.0.0.5", ctx);

            assert!(
                !model_honors_remote_init_shell(view),
                "the `command` builtin prefix is normalized away before matching"
            );
        });
    })
}

#[test]
fn suppression_falls_back_to_the_parsed_ssh_target() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            arm_warpify_disabled_profile(view, SSH_PROFILE_BARE_COMMAND, ctx);
            start_block_with_command(view, "ssh  -p 2222  --  root@10.0.0.5", ctx);

            assert!(
                !model_honors_remote_init_shell(view),
                "re-spaced commands still match through the parsed SSH target"
            );
        });
    })
}

#[test]
fn suppression_matches_commands_with_the_password_helper_prefix() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let mut profile = SshHostProfile::new("box".to_string(), "10.0.0.5".to_string());
        profile.user = Some("root".to_string());
        let command = format!(
            "{}{SSH_PROFILE_BARE_COMMAND}",
            askpass::env_prefix(&profile).unwrap()
        );

        terminal.update(&mut app, |view, ctx| {
            arm_warpify_disabled_profile(view, &command, ctx);
            start_block_with_command(view, &command, ctx);

            assert!(!model_honors_remote_init_shell(view));
        });
    })
}

/// Neither matcher recognizes an absolute `ssh` path: the string differs and the parser requires
/// a bare `ssh` argv[0].
#[test]
fn suppression_misses_absolute_ssh_paths() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            arm_warpify_disabled_profile(view, SSH_PROFILE_BARE_COMMAND, ctx);
            start_block_with_command(view, "/usr/bin/ssh -p 2222 -- root@10.0.0.5", ctx);

            assert!(
                model_honors_remote_init_shell(view),
                "an absolute ssh path matches neither the string nor the parsed target"
            );
        });
    })
}

fn watch_quick_connect(view: &mut TerminalView) {
    watch_quick_connect_in(view, None);
}

fn watch_quick_connect_in(view: &mut TerminalView, directory: Option<&str>) {
    view.ssh_profile.connection = Some(SshConnectionWatch::new(
        "ssh root@remote".to_string(),
        None,
        "root@remote".to_string(),
        None,
        directory.map(str::to_string),
    ));
}

#[test]
fn startup_directory_is_entered_once_per_connection_on_the_first_remote_prompt() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let local = SessionId::from(1);
        let remote = SessionId::from(2);
        let sent = |view: &TerminalView| {
            view.ssh_profile
                .connection
                .as_ref()
                .is_some_and(|connection| connection.startup_directory_sent)
        };

        terminal.update(&mut app, |view, ctx| {
            watch_quick_connect_in(view, Some("/srv/app"));
            view.active_block_metadata = Some(BlockMetadata::new(Some(local), None));
            view.handle_ssh_manager_precmd(&BlockMetadata::new(Some(remote), None), ctx);
            assert!(!sent(view), "nothing is sent before ssh starts");

            start_block_with_command(view, "ssh root@remote", ctx);
            view.handle_ssh_manager_precmd(&BlockMetadata::new(None, None), ctx);
            assert!(
                !sent(view),
                "a precmd without a session is not the remote shell"
            );

            view.handle_ssh_manager_precmd(&BlockMetadata::new(Some(remote), None), ctx);
            assert!(sent(view));

            view.handle_ssh_manager_precmd(&BlockMetadata::new(Some(local), None), ctx);
            view.active_block_metadata = Some(BlockMetadata::new(Some(local), None));
            start_block_with_command(view, "ssh root@remote", ctx);
            assert!(!sent(view), "a reconnect enters the directory again");
        });
    })
}

#[test]
fn reconnect_banner_shows_when_the_local_shell_prompts_after_a_watched_ssh_command() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let local = SessionId::from(1);
        let remote = SessionId::from(2);

        terminal.update(&mut app, |view, ctx| {
            watch_quick_connect(view);
            view.active_block_metadata = Some(BlockMetadata::new(Some(local), None));
            start_block_with_command(view, "ssh root@remote", ctx);

            view.handle_ssh_manager_precmd(&BlockMetadata::new(Some(remote), None), ctx);
            assert!(view.ssh_profile.reconnect_banner.is_none());

            view.handle_ssh_manager_precmd(&BlockMetadata::new(Some(local), None), ctx);
            assert!(view.ssh_profile.reconnect_banner.is_some());

            start_block_with_command(view, "ssh root@remote", ctx);
            assert!(view.ssh_profile.reconnect_banner.is_none());
        });
    })
}

#[test]
fn reconnect_banner_ignores_local_prompts_before_the_watched_command_starts() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let local = SessionId::from(1);

        terminal.update(&mut app, |view, ctx| {
            watch_quick_connect(view);
            view.active_block_metadata = Some(BlockMetadata::new(Some(local), None));
            start_block_with_command(view, "ls", ctx);

            view.handle_ssh_manager_precmd(&BlockMetadata::new(Some(local), None), ctx);
            assert!(view.ssh_profile.reconnect_banner.is_none());
        });
    })
}

#[test]
fn warpified_connections_go_through_the_ssh_wrapper() {
    let (command, control_path) =
        connection_command("command ssh -p 2222 -- root@host".to_string(), true);
    assert_eq!(command, "ssh -p 2222 -- root@host");
    assert_eq!(control_path, None);
}
