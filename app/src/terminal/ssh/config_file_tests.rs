use super::*;

/// Mirrors a real hand-written config: header comment, inline comment, unknown directive
/// (`IdentitiesOnly`), blank lines between stanzas.
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

fn host_blocks(segments: &[Segment]) -> Vec<&HostBlock> {
    segments
        .iter()
        .filter_map(|segment| match segment {
            Segment::Host(block) => Some(block),
            Segment::Raw(_) => None,
        })
        .collect()
}

#[test]
fn parses_hand_written_hosts_into_profiles() {
    let segments = parse(HAND_WRITTEN);
    let blocks = host_blocks(&segments);
    assert_eq!(blocks.len(), 2);

    let pangyo = blocks[0].to_profile();
    assert_eq!(pangyo.name, "PangyoA6000");
    assert_eq!(pangyo.host, "10.0.0.1"); // inline comment stripped from the value
    assert_eq!(pangyo.port, 50000);
    assert_eq!(pangyo.user.as_deref(), Some("bch"));

    let oracle = blocks[1].to_profile();
    assert_eq!(oracle.host, "138.2.125.242");
    assert_eq!(oracle.port, 22);
    assert_eq!(oracle.identity_file.as_deref(), Some("~/.ssh/oracle_free_a1"));
}

#[test]
fn roundtrip_is_byte_identical() {
    assert_eq!(serialize(&parse(HAND_WRITTEN)), HAND_WRITTEN);
}

#[test]
fn updating_a_hand_written_host_preserves_unknown_directives_and_comments() {
    let mut segments = parse(HAND_WRITTEN);
    let mut profile = host_blocks(&segments)[1].to_profile();
    profile.user = Some("root".to_string());

    match &mut segments[..] {
        [_, _, Segment::Host(block)] => block.apply_profile(&profile),
        _ => panic!("expected the oracle stanza to be the last segment"),
    }

    let updated = serialize(&segments);
    assert!(updated.contains("    User root"));
    // The directive Warp does not model must survive untouched.
    assert!(updated.contains("    IdentitiesOnly yes"));
    // Other stanzas and their inline comments are left alone.
    assert!(updated.contains("    HostName 10.0.0.1 #HostName 106.245.149.181"));
    assert!(updated.contains("# Read more about SSH config files"));
    // Saving from Warp is what claims ownership of the stanza.
    assert!(updated.contains(&format!("{WARP_ID_PREFIX} {}", profile.id)));
}

#[test]
fn warp_metadata_roundtrips_through_comments() {
    let id = Uuid::new_v4();
    let source = format!(
        "{WARP_ID_PREFIX} {id}\n{WARP_TAGS_PREFIX} prod,web\nHost app\n    HostName 10.0.0.9\n"
    );

    let profile = host_blocks(&parse(&source))[0].to_profile();
    assert_eq!(profile.id, id);
    assert_eq!(profile.tags, vec!["prod".to_string(), "web".to_string()]);

    assert_eq!(serialize(&parse(&source)), source);
}

#[test]
fn proxy_jump_roundtrips_including_per_hop_identity_files() {
    let mut profile = SshHostProfile::new("app".to_string(), "10.0.0.9".to_string());
    profile.jump_hosts = vec![
        SshJumpHost {
            host: "bastion".to_string(),
            user: Some("ops".to_string()),
            port: 2222,
            identity_file: Some("~/.ssh/bastion_key".to_string()),
        },
        SshJumpHost {
            host: "relay".to_string(),
            user: None,
            port: default_ssh_port(),
            identity_file: None,
        },
    ];

    let mut block = HostBlock::new_for(&profile);
    let rendered = serialize(&[Segment::Host(block.clone())]);
    assert!(rendered.contains("    ProxyJump ops@bastion:2222,relay"));

    // Identity files have no ProxyJump syntax, so they ride along in a comment.
    let parsed = host_blocks(&parse(&rendered))[0].to_profile();
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

    // Clearing the jump hosts removes both the directive and its metadata comment.
    profile.jump_hosts.clear();
    block.apply_profile(&profile);
    let cleared = serialize(&[Segment::Host(block)]);
    assert!(!cleared.contains("ProxyJump"));
    assert!(!cleared.contains(WARP_JUMP_IDENTITIES_PREFIX));
}

#[test]
fn default_port_is_not_written_out() {
    let profile = SshHostProfile::new("app".to_string(), "10.0.0.9".to_string());
    let rendered = serialize(&[Segment::Host(HostBlock::new_for(&profile))]);
    assert!(!rendered.contains("Port"));
}

#[test]
fn removing_only_drops_warp_owned_stanzas() {
    let id = Uuid::new_v4();
    let source = format!("Host manual\n    HostName 10.0.0.1\n\n{WARP_ID_PREFIX} {id}\nHost owned\n    HostName 10.0.0.2\n");
    let mut segments = parse(&source);

    segments.retain(
        |segment| !matches!(segment, Segment::Host(block) if block.warp_id == Some(id)),
    );

    let updated = serialize(&segments);
    assert!(updated.contains("Host manual"));
    assert!(!updated.contains("Host owned"));
    assert!(!updated.contains(WARP_ID_PREFIX));
}
