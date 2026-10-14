use super::*;
use crate::ssh_manager::profile::SshJumpHost;

#[test]
fn hosts_in_the_same_group_share_a_color_and_badge() {
    let mut web = SshHostProfile::new("web".to_string(), "10.0.0.1".to_string());
    let mut db = SshHostProfile::new("db".to_string(), "10.0.0.2".to_string());
    web.tags = vec!["prod".to_string()];
    db.tags = vec!["prod".to_string(), "db".to_string()];
    assert_eq!(host_color(&web), host_color(&db));
    assert_eq!(badge_label(&web), "PR");

    let ungrouped = SshHostProfile::new("gpu-box".to_string(), "10.0.0.3".to_string());
    assert_eq!(host_color(&ungrouped), None);
    assert_eq!(badge_label(&ungrouped), "GP");
}

#[test]
fn host_subtitle_shows_port_and_first_jump_host() {
    let mut profile = SshHostProfile::new("db".to_string(), "10.0.3.30".to_string());
    profile.user = Some("admin".to_string());
    assert_eq!(host_subtitle(&profile), "admin@10.0.3.30:22");
    profile.jump_hosts.push(SshJumpHost {
        source_profile_id: None,
        host: "bastion".to_string(),
        user: None,
        port: 22,
        identity_file: None,
    });
    assert_eq!(host_subtitle(&profile), "admin@10.0.3.30:22 · via bastion");
}

#[test]
fn common_group_names_get_different_colors() {
    let color = |group: &str| {
        let mut profile = SshHostProfile::new("h".to_string(), "h".to_string());
        profile.tags = vec![group.to_string()];
        host_color(&profile)
    };
    assert_ne!(color("prod"), color("staging"));
    assert_ne!(color("prod"), color("home"));
    assert_ne!(color("staging"), color("home"));
}
