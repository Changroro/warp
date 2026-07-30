use super::*;

#[test]
fn ssh_gcloud_ssh_parsing() {
    assert!(parse_interactive_ssh_command("gcloud").is_none());
    assert!(parse_interactive_ssh_command("gcloud compute").is_none());
    assert!(parse_interactive_ssh_command("gcloud compute ss").is_none());
    assert!(parse_interactive_ssh_command("gcloud compute ssh").is_none());
    assert!(parse_interactive_ssh_command("command gcloud compute ssh").is_none());

    assert!(
        parse_interactive_ssh_command("command gcloud compute ssh --zone us-west1-a").is_some()
    );
    assert!(parse_interactive_ssh_command("gcloud compute ssh --zone us-west1-a").is_some());
    assert!(
        parse_interactive_ssh_command("gcloud compute ssh --zone us-west1-a my-instance").is_some()
    );
    assert!(
        parse_interactive_ssh_command(
            "gcloud compute ssh --zone us-west1-a my-instance --project my-project"
        )
        .is_some()
    );
}

#[test]
fn ssh_elastic_beanstalk_parsing() {
    assert!(parse_interactive_ssh_command("eb").is_none());
    assert!(parse_interactive_ssh_command("eb ss").is_none());
    assert!(parse_interactive_ssh_command("eb ssh").is_none());
    assert!(parse_interactive_ssh_command("command eb ssh").is_none());

    assert!(parse_interactive_ssh_command("command eb ssh --profile my-profile").is_some());
    assert!(parse_interactive_ssh_command("eb ssh --profile my-profile").is_some());
    assert!(parse_interactive_ssh_command("eb ssh --profile my-profile my-env").is_some());
}

#[test]
fn ssh_digital_ocean_droplet_parsing() {
    assert!(parse_interactive_ssh_command("doctl").is_none());
    assert!(parse_interactive_ssh_command("doctl compute").is_none());
    assert!(parse_interactive_ssh_command("doctl compute ss").is_none());
    assert!(parse_interactive_ssh_command("doctl compute ssh").is_none());
    assert!(parse_interactive_ssh_command("command doctl compute ssh").is_none());

    assert!(parse_interactive_ssh_command("command doctl compute ssh --region nyc1").is_some());
    assert!(parse_interactive_ssh_command("doctl compute ssh --region nyc1").is_some());
    assert!(parse_interactive_ssh_command("doctl compute ssh --region nyc1 my-droplet").is_some());
}

/// Verifies that commands resulting from shell alias expansion are correctly
/// detected as interactive SSH commands. When a user types an alias (e.g.
/// `myssh`), the terminal view expands it to the alias value before passing
/// it to `parse_interactive_ssh_command`. These tests cover representative
/// expanded forms.
#[test]
fn ssh_alias_expanded_commands() {
    // Simple alias: alias myssh='ssh user@host'
    assert_eq!(
        parse_interactive_ssh_command("ssh user@host").unwrap().host,
        Some("user@host".to_string())
    );

    // Alias with key and user: alias company1='ssh -i /path/to/key user@server'
    assert_eq!(
        parse_interactive_ssh_command("ssh -i /path/to/key user@server")
            .unwrap()
            .host,
        Some("user@server".to_string())
    );

    // Alias with extra args appended by the user: alias myssh='ssh -i key'
    // then the user types `myssh user@host` which expands to `ssh -i key user@host`
    assert_eq!(
        parse_interactive_ssh_command("ssh -i key user@host")
            .unwrap()
            .host,
        Some("user@host".to_string())
    );

    // Alias that isn't SSH should not match
    assert!(parse_interactive_ssh_command("ls -la").is_none());
}

#[test]
fn ssh_interactive_shell_parsing() {
    assert!(parse_interactive_ssh_command("").is_none());
    assert!(parse_interactive_ssh_command("ls").is_none());
    assert!(parse_interactive_ssh_command("ssh-add-key").is_none());

    // Basic interactive command
    assert!(
        parse_interactive_ssh_command("ssh localhost").unwrap().host
            == Some("localhost".to_string())
    );
    assert!(
        parse_interactive_ssh_command("command ssh localhost")
            .unwrap()
            .host
            == Some("localhost".to_string())
    );
    assert!(
        parse_interactive_ssh_command("ssh root@127.14.80.1 -p 2222")
            .unwrap()
            .host
            == Some("root@127.14.80.1".to_string())
    );
    assert!(
        parse_interactive_ssh_command("ssh -4vw root@127.14.80.1 -p 2222")
            .unwrap()
            .host
            == Some("root@127.14.80.1".to_string())
    );

    // Commands with -T or -W, which are non-interactive
    assert!(parse_interactive_ssh_command("ssh -T user@host").is_none());
    assert!(parse_interactive_ssh_command("ssh -v user@host -W localhost:22").is_none());
    assert!(parse_interactive_ssh_command("ssh -o IdentityFile=/etc/file -T user@host").is_none());

    // Commands with multiple positional arguments, implying non-interactive
    assert!(parse_interactive_ssh_command("ssh user@host ls").is_none());
    assert!(parse_interactive_ssh_command("ssh user@host echo 'Hello, World!'").is_none());

    // Weird spacing and shell characters shouldn't matter
    assert!(
        parse_interactive_ssh_command("ssh     user@host")
            .unwrap()
            .host
            == Some("user@host".to_string())
    );
    assert!(
        parse_interactive_ssh_command("ssh -4 -- localhost")
            .unwrap()
            .host
            == Some("localhost".to_string())
    );
}

#[test]
fn ssh_login_user_parsing() {
    let user_of = |command: &str| parse_interactive_ssh_command(command).unwrap().user;

    assert_eq!(user_of("ssh localhost"), None);
    assert_eq!(user_of("ssh root@localhost"), Some("root".to_string()));
    assert_eq!(user_of("ssh -l root localhost"), Some("root".to_string()));
    assert_eq!(
        user_of("command ssh -l root -p 5022 -- localhost"),
        Some("root".to_string())
    );
    assert_eq!(
        user_of("ssh -l ignored root@localhost"),
        Some("root".to_string())
    );
    assert_eq!(
        parse_interactive_ssh_command("ssh -l root localhost")
            .unwrap()
            .host,
        Some("localhost".to_string())
    );
    assert!(parse_interactive_ssh_command("ssh -l").is_none());
}

fn jump_host(
    host: &str,
    user: Option<&str>,
    port: u16,
    identity_file: Option<&str>,
) -> SshJumpHost {
    SshJumpHost {
        source_profile_id: None,
        host: host.to_string(),
        user: user.map(str::to_string),
        port,
        identity_file: identity_file.map(str::to_string),
    }
}

fn proxy_command_value(command: &str) -> String {
    let argv = shell_words::split(command).expect("rendered command is not shell parseable");
    argv.iter()
        .find_map(|arg| arg.strip_prefix("ProxyCommand="))
        .expect("rendered command has no ProxyCommand")
        .to_string()
}

/// Applies the OpenSSH `%` expansion that runs before the ProxyCommand value reaches `/bin/sh -c`.
fn expand_openssh_percent(value: &str) -> String {
    let mut expanded = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            expanded.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => expanded.push('%'),
            Some(token) => panic!("unescaped OpenSSH token %{token} in {value}"),
            None => panic!("dangling % in {value}"),
        }
    }
    expanded
}

#[test]
fn ssh_profile_command_quotes_arguments() {
    let mut profile =
        SshHostProfile::new("Production DB".to_string(), "db.example.com".to_string());
    profile.user = Some("deploy user".to_string());
    profile.port = 2222;
    profile.identity_file = Some("/Users/me/.ssh/prod key".to_string());

    assert_eq!(
        profile.to_ssh_command(),
        "command ssh -i '/Users/me/.ssh/prod key' -p 2222 -- 'deploy user@db.example.com'"
    );
}

#[test]
fn ssh_profile_command_does_not_vary_with_warpify_settings() {
    let mut profile = SshHostProfile::new("Production".to_string(), "db.example.com".to_string());
    profile.user = Some("deploy".to_string());

    assert_eq!(
        profile.to_ssh_command(),
        "command ssh -- deploy@db.example.com"
    );
    assert!(parse_interactive_ssh_command(&profile.to_ssh_command()).is_some());
}

#[test]
fn ssh_profile_command_renders_jump_hosts() {
    let mut profile = SshHostProfile::new("Private".to_string(), "10.0.0.5".to_string());
    profile.user = Some("ubuntu".to_string());
    profile.jump_hosts = vec![
        jump_host("bastion.example.com", Some("jump"), 2200, None),
        jump_host("inner.example.com", None, 22, None),
    ];

    assert_eq!(
        profile.to_ssh_command(),
        "command ssh -J jump@bastion.example.com:2200,inner.example.com -- ubuntu@10.0.0.5"
    );
}

#[test]
fn ssh_profile_command_renders_jump_host_identity_file() {
    let mut profile = SshHostProfile::new("Private".to_string(), "10.0.0.5".to_string());
    profile.user = Some("ubuntu".to_string());
    profile.jump_hosts = vec![jump_host(
        "bastion.example.com",
        Some("jump"),
        2200,
        Some("~/.ssh/bastion"),
    )];

    assert_eq!(
        profile.to_ssh_command(),
        "command ssh -o 'ProxyCommand=ssh -i ~/.ssh/bastion -p 2200 -W 10.0.0.5:22 -- jump@bastion.example.com' -- ubuntu@10.0.0.5"
    );
}

#[test]
fn ssh_profile_command_renders_proxy_chain_when_a_jump_host_has_identity_file() {
    let mut profile = SshHostProfile::new("Private".to_string(), "10.0.0.5".to_string());
    profile.user = Some("ubuntu".to_string());
    profile.jump_hosts = vec![
        jump_host("edge.example.com", Some("edge"), 2200, Some("~/.ssh/edge")),
        jump_host("inner.example.com", Some("inner"), 2222, None),
    ];

    let argv = shell_words::split(&profile.to_ssh_command()).unwrap();
    assert_eq!(argv[0], "command");
    assert_eq!(argv[1], "ssh");
    assert_eq!(argv[2], "-o");
    assert_eq!(argv[4], "--");
    assert_eq!(argv[5], "ubuntu@10.0.0.5");

    let proxy_command = &argv[3];
    assert!(proxy_command.starts_with("ProxyCommand=ssh "));
    assert!(proxy_command.contains("-i ~/.ssh/edge"));
    assert!(proxy_command.contains("-p 2200"));
    assert!(proxy_command.contains("-W inner.example.com:2222"));
    assert!(proxy_command.contains("-- edge@edge.example.com"));
    assert!(proxy_command.contains("-p 2222"));
    assert!(proxy_command.contains("-W 10.0.0.5:22"));
    assert!(proxy_command.contains("-- inner@inner.example.com"));
}

#[test]
fn ssh_profile_command_separates_options_from_target() {
    let profile = SshHostProfile::new("Host".to_string(), "-oProxyCommand=bad".to_string());

    assert_eq!(
        profile.to_ssh_command(),
        "command ssh -- '-oProxyCommand=bad'"
    );
}

#[test]
fn ssh_profile_password_key_uses_stable_id() {
    let id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
    assert_eq!(
        ssh_profile_password_key(id),
        "ssh-profile-password:550e8400-e29b-41d4-a716-446655440000"
    );
}

#[test]
fn normalize_ssh_profiles_migrates_missing_or_duplicate_ids() {
    let id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
    let mut profiles = vec![
        SshHostProfile {
            id: Uuid::nil(),
            name: "missing".to_string(),
            host: "one.example.com".to_string(),
            user: None,
            port: 0,
            identity_file: None,
            jump_hosts: vec![jump_host("jump.example.com", None, 0, None)],
            tags: Vec::new(),
        },
        SshHostProfile {
            id,
            name: "first".to_string(),
            host: "two.example.com".to_string(),
            user: None,
            port: 22,
            identity_file: None,
            jump_hosts: Vec::new(),
            tags: Vec::new(),
        },
        SshHostProfile {
            id,
            name: "duplicate".to_string(),
            host: "three.example.com".to_string(),
            user: None,
            port: 22,
            identity_file: None,
            jump_hosts: Vec::new(),
            tags: Vec::new(),
        },
    ];

    let reassignments = normalize_ssh_host_profiles(&mut profiles);

    assert!(!profiles[0].id.is_nil());
    assert_eq!(profiles[0].port, 22);
    assert_eq!(profiles[0].jump_hosts[0].port, 22);
    assert_eq!(profiles[1].id, id);
    assert_ne!(profiles[2].id, id);
    assert_ne!(profiles[0].id, profiles[2].id);

    assert_eq!(
        reassignments,
        vec![
            SshProfileIdReassignment {
                previous_id: Uuid::nil(),
                new_id: profiles[0].id,
            },
            SshProfileIdReassignment {
                previous_id: id,
                new_id: profiles[2].id,
            },
        ]
    );
}

#[test]
fn normalize_ssh_profiles_reports_no_reassignment_for_unique_ids() {
    let mut profiles = vec![
        SshHostProfile::new("one".to_string(), "one.example.com".to_string()),
        SshHostProfile::new("two".to_string(), "two.example.com".to_string()),
    ];

    assert!(normalize_ssh_host_profiles(&mut profiles).is_empty());
}

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

#[test]
fn proxy_command_keeps_hostile_host_as_a_single_argument() {
    let mut profile =
        SshHostProfile::new("Hostile".to_string(), "h'; touch /tmp/pwn; '".to_string());
    profile.jump_hosts = vec![jump_host("bastion.example.com", None, 22, Some("/k"))];

    let proxy_command = expand_openssh_percent(&proxy_command_value(&profile.to_ssh_command()));
    let proxy_argv = shell_words::split(&proxy_command).expect("proxy command is not parseable");

    assert_eq!(
        proxy_argv,
        vec![
            "ssh",
            "-i",
            "/k",
            "-W",
            "h'; touch /tmp/pwn; ':22",
            "--",
            "bastion.example.com",
        ]
    );
}

#[test]
fn proxy_command_neutralizes_shell_metacharacters_in_user_values() {
    for hostile in [
        "h$(touch /tmp/pwn)",
        "h`touch /tmp/pwn`",
        "h; touch /tmp/pwn",
        "h\ntouch /tmp/pwn",
        "h && touch /tmp/pwn",
        "h | touch /tmp/pwn",
    ] {
        let mut profile = SshHostProfile::new("Hostile".to_string(), hostile.to_string());
        profile.jump_hosts = vec![jump_host(hostile, Some(hostile), 22, Some(hostile))];

        let proxy_command = expand_openssh_percent(&proxy_command_value(&profile.to_ssh_command()));
        let proxy_argv =
            shell_words::split(&proxy_command).expect("proxy command is not parseable");

        assert_eq!(
            proxy_argv,
            vec![
                "ssh".to_string(),
                "-i".to_string(),
                hostile.to_string(),
                "-W".to_string(),
                format!("{hostile}:22"),
                "--".to_string(),
                format!("{hostile}@{hostile}"),
            ]
        );
    }
}

#[test]
fn proxy_command_escapes_percent_tokens_in_user_values() {
    let mut profile = SshHostProfile::new("Tokens".to_string(), "target.example.com".to_string());
    profile.jump_hosts = vec![jump_host("%h", Some("%r"), 22, Some("%d/.ssh/id_%u"))];

    let proxy_command = proxy_command_value(&profile.to_ssh_command());
    assert!(proxy_command.contains("%%d/.ssh/id_%%u"));
    assert!(proxy_command.contains("%%r@%%h"));

    let expanded = expand_openssh_percent(&proxy_command);
    let proxy_argv = shell_words::split(&expanded).expect("proxy command is not parseable");
    assert_eq!(
        proxy_argv,
        vec![
            "ssh",
            "-i",
            "%d/.ssh/id_%u",
            "-W",
            "target.example.com:22",
            "--",
            "%r@%h",
        ]
    );
}

#[test]
fn nested_proxy_command_escapes_percent_tokens_for_each_expansion() {
    let mut profile = SshHostProfile::new("Tokens".to_string(), "target.example.com".to_string());
    profile.jump_hosts = vec![
        jump_host("edge.example.com", None, 22, Some("/keys/id_%d")),
        jump_host("inner.example.com", None, 22, Some("/keys/id_inner")),
    ];

    let outer = proxy_command_value(&profile.to_ssh_command());
    assert!(outer.contains("/keys/id_%%%%d"));

    let inner = proxy_command_value(&expand_openssh_percent(&outer));
    let inner_argv = shell_words::split(&expand_openssh_percent(&inner))
        .expect("proxy command is not parseable");
    assert_eq!(
        inner_argv,
        vec![
            "ssh",
            "-i",
            "/keys/id_%d",
            "-W",
            "inner.example.com:22",
            "--",
            "edge.example.com",
        ]
    );
}

#[test]
fn ssh_profile_command_brackets_ipv6_jump_hosts() {
    let mut profile = SshHostProfile::new("V6".to_string(), "10.0.0.5".to_string());
    profile.jump_hosts = vec![
        jump_host("::1", Some("u"), 2222, None),
        jump_host("fe80::2", None, 22, None),
    ];

    assert_eq!(
        profile.to_ssh_command(),
        "command ssh -J 'u@[::1]:2222,[fe80::2]' -- 10.0.0.5"
    );
}

#[test]
fn proxy_command_brackets_ipv6_destinations() {
    let mut profile = SshHostProfile::new("V6".to_string(), "fe80::2".to_string());
    profile.port = 2022;
    profile.jump_hosts = vec![
        jump_host("::1", None, 22, Some("/k")),
        jump_host("2001:db8::5", None, 2200, None),
    ];

    let outer = proxy_command_value(&profile.to_ssh_command());
    let inner = proxy_command_value(&expand_openssh_percent(&outer));
    let inner_argv = shell_words::split(&expand_openssh_percent(&inner))
        .expect("proxy command is not parseable");

    assert_eq!(
        inner_argv,
        vec!["ssh", "-i", "/k", "-W", "[2001:db8::5]:2200", "--", "::1"]
    );
    assert!(
        shell_words::split(&expand_openssh_percent(&outer))
            .unwrap()
            .contains(&"[fe80::2]:2022".to_string())
    );
}

#[test]
fn jump_host_keeps_source_profile_id_through_serde_roundtrip() {
    let id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
    let mut jump = jump_host("bastion.example.com", Some("jump"), 2200, None);
    jump.source_profile_id = Some(id);

    let encoded = serde_json::to_string(&jump).unwrap();
    let decoded: SshJumpHost = serde_json::from_str(&encoded).unwrap();

    assert_eq!(decoded, jump);
    assert_eq!(decoded.source_profile_id, Some(id));
}

#[test]
fn legacy_jump_host_without_source_profile_id_deserializes() {
    let decoded: SshHostProfile = serde_json::from_str(
        r#"{
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "name": "legacy",
            "host": "10.0.0.5",
            "user": "ubuntu",
            "identity_file": null,
            "jump_hosts": [
                {
                    "host": "bastion.example.com",
                    "user": "jump",
                    "port": 2200,
                    "identity_file": null
                }
            ]
        }"#,
    )
    .unwrap();

    assert_eq!(decoded.port, 22);
    assert_eq!(decoded.jump_hosts[0].source_profile_id, None);
    assert_eq!(decoded.jump_hosts[0].port, 2200);
}
