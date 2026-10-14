use super::*;

fn profile(user: Option<&str>, host: &str) -> SshHostProfile {
    let mut profile = SshHostProfile::new("box".to_string(), host.to_string());
    profile.user = user.map(str::to_string);
    profile
}

#[test]
fn password_prompts_must_name_the_profile_principal() {
    let expected = expected_principal(&profile(Some("Root"), "Box.Example.com"));
    assert_eq!(expected, "root@box.example.com");

    assert!(is_password_prompt_for(
        "root@box.example.com's password: ",
        &expected
    ));
    assert!(is_password_prompt_for(
        "(root@box.example.com) Password: ",
        &expected
    ));

    assert!(!is_password_prompt_for(
        "admin@box.example.com's password: ",
        &expected
    ));
    assert!(!is_password_prompt_for(
        "xroot@box.example.com's password: ",
        &expected
    ));
    assert!(!is_password_prompt_for(
        "root@jump.example.com's password: ",
        &expected
    ));
    assert!(!is_password_prompt_for(
        "(root@box.example.com) Verification code: ",
        &expected
    ));
    assert!(!is_password_prompt_for(
        "Enter passphrase for key '/home/me/.ssh/id_ed25519': ",
        &expected
    ));
    assert!(!is_password_prompt_for(
        "Are you sure you want to continue connecting (yes/no/[fingerprint])? ",
        &expected
    ));
}

#[test]
fn profiles_without_a_user_accept_any_user_on_the_host() {
    let expected = expected_principal(&profile(None, "10.0.0.5"));
    assert_eq!(expected, "@10.0.0.5");

    assert!(is_password_prompt_for(
        "me@10.0.0.5's password: ",
        &expected
    ));
    assert!(!is_password_prompt_for(
        "me@10.0.0.55's password: ",
        &expected
    ));
    assert!(!is_password_prompt_for("@10.0.0.5's password: ", &expected));
}

#[test]
fn the_env_prefix_is_stripped_back_to_the_ssh_command() {
    let prefix = env_prefix(&profile(Some("root"), "10.0.0.5")).unwrap();
    let command = format!("{prefix}ssh -p 2222 -- root@10.0.0.5");

    assert!(command.starts_with("SSH_ASKPASS="));
    assert_eq!(strip_env_prefix(&command), "ssh -p 2222 -- root@10.0.0.5");
    assert_eq!(strip_env_prefix("ssh root@10.0.0.5"), "ssh root@10.0.0.5");
    assert_eq!(
        strip_env_prefix("SSH_ASKPASS=/bin/x ssh host"),
        "SSH_ASKPASS=/bin/x ssh host"
    );
}
