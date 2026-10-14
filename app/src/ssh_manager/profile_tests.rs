use super::*;
use crate::terminal::ssh::util::parse_interactive_ssh_command;

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

#[test]
fn matches_query_checks_name_host_user_and_tags() {
    let mut profile = SshHostProfile::new("Web".to_string(), "10.0.0.5".to_string());
    profile.user = Some("deploy".to_string());
    profile.tags = vec!["Prod".to_string()];
    for query in ["web", "10.0", "DEPLOY", "prod", " web "] {
        assert!(profile.matches_query(query), "{query}");
    }
    assert!(!profile.matches_query("staging"));
}

#[test]
fn quick_connect_args_accepts_input_with_or_without_ssh() {
    assert_eq!(
        quick_connect_args("user@host").as_deref(),
        Some("user@host")
    );
    assert_eq!(
        quick_connect_args(" ssh  -p 2222 host ").as_deref(),
        Some("-p 2222 host")
    );
    assert_eq!(quick_connect_args("sshbox").as_deref(), Some("sshbox"));
    assert_eq!(quick_connect_args("ssh"), None);
    assert_eq!(quick_connect_args("   "), None);
}

#[test]
fn quick_connect_destination_parses_user_host_and_port() {
    let parse = QuickConnectDestination::parse;
    assert_eq!(
        parse("deploy@10.0.3.45:2222"),
        Some(QuickConnectDestination {
            user: Some("deploy".to_string()),
            host: "10.0.3.45".to_string(),
            port: 2222,
        })
    );
    assert_eq!(parse("box").map(|d| (d.user, d.port)), Some((None, 22)));
    assert_eq!(
        parse("[::1]:2200").map(|d| (d.host, d.port)),
        Some(("::1".to_string(), 2200))
    );
    assert_eq!(
        parse("fe80::1").map(|d| d.host),
        Some("fe80::1".to_string())
    );
    assert_eq!(parse("-p 22 host"), None);
    assert_eq!(parse("host:notaport"), None);
    assert_eq!(parse("@host"), None);
}

#[test]
fn quick_connect_args_turns_host_port_into_a_port_flag() {
    assert_eq!(
        quick_connect_args("deploy@10.0.3.45:2222").as_deref(),
        Some("deploy@10.0.3.45 -p 2222")
    );
    assert_eq!(quick_connect_args("ssh box:22").as_deref(), Some("box"));
}

#[test]
fn quick_connect_input_never_reaches_the_shell_as_syntax() {
    assert_eq!(QuickConnectDestination::parse("a;id@host"), None);
    assert_eq!(QuickConnectDestination::parse("$(id)@host"), None);
    assert_eq!(QuickConnectDestination::parse("host\"x"), None);
    assert_eq!(
        quick_connect_args("host; touch /tmp/pwned").as_deref(),
        Some("'host;' touch /tmp/pwned")
    );
    assert_eq!(
        quick_connect_args("-p 2222 $(id)@host").as_deref(),
        Some("-p 2222 '$(id)@host'")
    );
}
