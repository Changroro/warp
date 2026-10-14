use super::*;
use crate::ssh_manager::login_prompt::parse_ssh_login_prompt;
use crate::ssh_manager::profile::SshHostProfile;

fn state(command: &str) -> SshAutoInjectState {
    SshAutoInjectState::new(Uuid::new_v4(), command.to_string(), BlockId::new())
        .expect("ssh command target should parse")
}

fn decide(state: &SshAutoInjectState, output_tail: &str) -> SshAutoInjectDecision {
    let prompt = parse_ssh_login_prompt(output_tail).expect("output tail should be a prompt");
    state.evaluate_login_prompt(&prompt)
}

fn injects(state: &SshAutoInjectState, output_tail: &str) -> bool {
    decide(state, output_tail) == SshAutoInjectDecision::Inject
}

#[test]
fn matches_command_handles_command_wrapper_and_double_dash() {
    let state = state("command ssh -p 5022 -- root@118.38.20.101");

    assert!(state.matches_command("ssh -p 5022 root@118.38.20.101"));
}

#[test]
fn matches_command_accepts_the_rendered_profile_command() {
    let mut profile = SshHostProfile::new("Production".to_string(), "db.example.com".to_string());
    profile.user = Some("deploy".to_string());
    profile.port = 5022;
    let rendered = profile.to_ssh_command();
    let state = state(&rendered);

    assert!(state.matches_command(&rendered));
    assert!(state.matches_command("ssh -p 5022 deploy@db.example.com"));
    assert!(!state.matches_command("env ssh -p 5022 deploy@db.example.com"));
    assert!(!state.matches_command("ssh -p 5022 other@db.example.com"));
}

#[test]
fn matches_command_rejects_different_ssh_target() {
    let state = state("command ssh -p 5022 -- root@118.38.20.101");

    assert!(!state.matches_command("ssh -p 5022 root@118.38.20.102"));
    assert!(!state.matches_command("ssh -p 22 root@118.38.20.101"));
}

#[test]
fn matches_command_compares_login_user() {
    let state = state("command ssh -l root -- example.test");

    assert!(state.matches_command("ssh -l root example.test"));
    assert!(state.matches_command("ssh root@example.test"));
    assert!(!state.matches_command("ssh -l guest example.test"));
    assert!(!state.matches_command("ssh guest@example.test"));
    assert!(!state.matches_command("ssh example.test"));
}

#[test]
fn matches_command_rejects_commands_without_a_target() {
    let state = state("command ssh -- example.test");

    assert!(!state.matches_command("gcloud compute ssh --zone us-west1-a"));
    assert!(!state.matches_command("ls -la"));
}

#[test]
fn new_rejects_commands_without_a_resolvable_target() {
    assert!(
        SshAutoInjectState::new(Uuid::new_v4(), "ls -la".to_string(), BlockId::new()).is_none()
    );
    assert!(
        SshAutoInjectState::new(
            Uuid::new_v4(),
            "gcloud compute ssh --zone us-west1-a".to_string(),
            BlockId::new(),
        )
        .is_none()
    );
}

#[test]
fn evaluate_login_prompt_accepts_the_armed_destination() {
    let state = state("command ssh -p 5022 -- root@example.test");

    assert!(injects(
        &state,
        "Warning: banner\r\nroot@example.test's password: "
    ));
    assert!(injects(&state, "root@EXAMPLE.TEST's password:"));
    assert!(injects(
        &state,
        "(root@example.test) root@example.test's password: "
    ));
}

#[test]
fn evaluate_login_prompt_checks_pam_prompt_principals() {
    let state = state("command ssh -p 5022 -- root@example.test");

    assert!(injects(&state, "(root@example.test) Password: "));
    assert_eq!(
        decide(&state, "(root@other.test) Password: "),
        SshAutoInjectDecision::Skip(SshAutoInjectSkipReason::TargetMismatch)
    );
    assert_eq!(
        decide(&state, "(guest@example.test) Password:"),
        SshAutoInjectDecision::Skip(SshAutoInjectSkipReason::TargetMismatch)
    );
}

#[test]
fn evaluate_login_prompt_skips_prompts_without_a_host() {
    let state = state("command ssh -p 5022 -- root@example.test");

    for output_tail in ["Password:", "Password: ", "(root) Password: "] {
        assert_eq!(
            decide(&state, output_tail),
            SshAutoInjectDecision::Skip(SshAutoInjectSkipReason::UnverifiableHost)
        );
    }
}

#[test]
fn evaluate_login_prompt_skips_key_passphrase_prompts() {
    let state = state("command ssh -p 5022 -i /home/user/.ssh/id_ed25519 -- root@example.test");

    assert_eq!(
        decide(
            &state,
            "Enter passphrase for key '/home/user/.ssh/id_ed25519': "
        ),
        SshAutoInjectDecision::Skip(SshAutoInjectSkipReason::KeyPassphrasePrompt)
    );
}

#[test]
fn evaluate_login_prompt_rejects_other_hosts_and_users() {
    let state = state("command ssh -p 5022 -- root@example.test");

    for output_tail in [
        "admin@other.test's password: ",
        "guest@example.test's password: ",
        "example.test's password: ",
    ] {
        assert_eq!(
            decide(&state, output_tail),
            SshAutoInjectDecision::Skip(SshAutoInjectSkipReason::TargetMismatch)
        );
    }
}

#[test]
fn evaluate_login_prompt_accepts_any_user_when_the_target_has_none() {
    let state = state("command ssh -- example.test");

    assert!(injects(&state, "bch@example.test's password: "));
    assert_eq!(
        decide(&state, "bch@other.test's password: "),
        SshAutoInjectDecision::Skip(SshAutoInjectSkipReason::TargetMismatch)
    );
}

#[test]
fn password_has_control_chars_detects_injectable_bytes() {
    assert!(!password_has_control_chars("plain-value"));
    assert!(!password_has_control_chars("päßwört"));
    assert!(password_has_control_chars("value\n"));
    assert!(password_has_control_chars("value\r\nrm -rf /"));
    assert!(password_has_control_chars("value\0"));
    assert!(password_has_control_chars("value\ttab"));
}

#[test]
fn state_tracks_a_single_target_block() {
    let block_id = BlockId::new();
    let mut state = state("command ssh -- example.test");

    assert!(!state.has_target_block());

    state.activate(block_id.clone());

    assert_eq!(state.target_block_id(), Some(&block_id));
    assert!(state.has_target_block());
}
