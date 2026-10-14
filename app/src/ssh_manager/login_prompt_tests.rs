use super::*;

fn host_password_prompt(user: Option<&str>, host: &str) -> Option<SshLoginPromptKind> {
    Some(SshLoginPromptKind::HostPassword {
        principal: Some(SshPromptPrincipal {
            user: user.map(ToString::to_string),
            host: host.to_string(),
        }),
    })
}

fn key_passphrase_prompt(identity_file: &str) -> Option<SshLoginPromptKind> {
    Some(SshLoginPromptKind::KeyPassphrase {
        identity_file: identity_file.to_string(),
    })
}

#[test]
fn strict_ssh_password_prompt_detection_rejects_sudo() {
    assert_eq!(
        parse_ssh_login_prompt("user@example.com's password: "),
        host_password_prompt(Some("user"), "example.com")
    );
    assert_eq!(
        parse_ssh_login_prompt("Enter passphrase for key '/Users/me/.ssh/id_ed25519': "),
        key_passphrase_prompt("/Users/me/.ssh/id_ed25519")
    );
    assert_eq!(
        parse_ssh_login_prompt("ssh user@example.com\ruser@example.com's password: "),
        host_password_prompt(Some("user"), "example.com")
    );
    assert_eq!(
        parse_ssh_login_prompt("Enter passphrase for key \"/Users/me/.ssh/id_ed25519\": "),
        key_passphrase_prompt("/Users/me/.ssh/id_ed25519")
    );
    assert!(parse_ssh_login_prompt("[sudo] password for user: ").is_none());
    assert!(parse_ssh_login_prompt("Are you sure you want to continue connecting?").is_none());
}

#[test]
fn strict_ssh_password_prompt_detection_accepts_pam_prompts() {
    let without_principal = Some(SshLoginPromptKind::HostPassword { principal: None });

    assert_eq!(parse_ssh_login_prompt("Password: "), without_principal);
    assert_eq!(parse_ssh_login_prompt("Password:"), without_principal);
    assert_eq!(
        parse_ssh_login_prompt("(root@10.0.0.5) Password: "),
        host_password_prompt(Some("root"), "10.0.0.5")
    );
    assert_eq!(
        parse_ssh_login_prompt("ssh root@10.0.0.5\r\n(root@10.0.0.5) Password: "),
        host_password_prompt(Some("root"), "10.0.0.5")
    );
    assert_eq!(
        parse_ssh_login_prompt("10.0.0.5's password: "),
        host_password_prompt(None, "10.0.0.5")
    );
}

#[test]
fn strict_ssh_password_prompt_detection_rejects_lookalike_prompts() {
    assert!(parse_ssh_login_prompt("").is_none());
    assert!(parse_ssh_login_prompt("password: ").is_none());
    assert!(parse_ssh_login_prompt("Enter your Password: ").is_none());
    assert!(parse_ssh_login_prompt("Password: hunter2").is_none());
    assert!(parse_ssh_login_prompt("Password for root: ").is_none());
    assert!(parse_ssh_login_prompt("(root@10.0.0.5) password: ").is_none());
    assert!(parse_ssh_login_prompt("Vault Password: ").is_none());
    assert!(
        parse_ssh_login_prompt(
            "user@example.com's password: \nPermission denied, please try again."
        )
        .is_none()
    );
    assert!(
        parse_ssh_login_prompt("Enter passphrase for key '/Users/me/.ssh/id_ed25519': done")
            .is_none()
    );
}
