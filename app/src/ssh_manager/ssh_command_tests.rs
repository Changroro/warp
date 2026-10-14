use super::*;
use crate::ssh_manager::profile::SshHostProfile;

#[test]
fn same_ssh_command_handles_command_wrapper_and_double_dash() {
    assert!(same_ssh_command(
        "command ssh -p 5022 -- root@118.38.20.101",
        "ssh -p 5022 root@118.38.20.101"
    ));
}

#[test]
fn same_ssh_command_accepts_the_rendered_profile_command() {
    let mut profile = SshHostProfile::new("Production".to_string(), "db.example.com".to_string());
    profile.user = Some("deploy".to_string());
    profile.port = 5022;
    let rendered = profile.to_ssh_command();

    assert!(same_ssh_command(&rendered, &rendered));
    assert!(same_ssh_command(
        &rendered,
        "ssh -p 5022 deploy@db.example.com"
    ));
    assert!(!same_ssh_command(
        &rendered,
        "env ssh -p 5022 deploy@db.example.com"
    ));
    assert!(!same_ssh_command(
        &rendered,
        "ssh -p 5022 other@db.example.com"
    ));
}

#[test]
fn same_ssh_command_rejects_different_ssh_targets() {
    let command = "command ssh -p 5022 -- root@118.38.20.101";

    assert!(!same_ssh_command(command, "ssh -p 5022 root@118.38.20.102"));
    assert!(!same_ssh_command(command, "ssh -p 22 root@118.38.20.101"));
}

#[test]
fn same_ssh_command_compares_login_user() {
    let command = "command ssh -l root -- example.test";

    assert!(same_ssh_command(command, "ssh -l root example.test"));
    assert!(same_ssh_command(command, "ssh root@example.test"));
    assert!(!same_ssh_command(command, "ssh -l guest example.test"));
    assert!(!same_ssh_command(command, "ssh guest@example.test"));
    assert!(!same_ssh_command(command, "ssh example.test"));
}

#[test]
fn same_ssh_command_rejects_commands_without_a_target() {
    let command = "command ssh -- example.test";

    assert!(!same_ssh_command(
        command,
        "gcloud compute ssh --zone us-west1-a"
    ));
    assert!(!same_ssh_command(command, "ls -la"));
    assert!(!same_ssh_command(
        "ls -la",
        "gcloud compute ssh --zone us-west1-a"
    ));
}
