use std::path::Path;

use super::*;
use crate::terminal::ssh::util::parse_interactive_ssh_command;

#[test]
fn with_control_master_inserts_options_after_ssh() {
    let path = Path::new("/home/me/.ssh/warp-cm/abc");
    assert_eq!(
        with_control_master("command ssh -p 2222 -- root@host", path),
        "command ssh -o ControlMaster=auto -o ControlPath=/home/me/.ssh/warp-cm/abc -o ControlPersist=no -p 2222 -- root@host"
    );
    assert_eq!(
        with_control_master("ssh host", path),
        "ssh -o ControlMaster=auto -o ControlPath=/home/me/.ssh/warp-cm/abc -o ControlPersist=no host"
    );
}

#[test]
fn ssh_parser_still_finds_the_target_with_control_master_options() {
    let command = with_control_master(
        "command ssh -p 2222 -- root@10.0.0.5",
        Path::new("/home/me/.ssh/warp-cm/abc"),
    );
    let parsed = parse_interactive_ssh_command(&command).expect("ssh command");
    assert_eq!(parsed.host.as_deref(), Some("root@10.0.0.5"));
}

#[test]
fn remote_image_path_uses_mime_extension() {
    assert!(remote_image_path("image/png").ends_with(".png"));
    assert!(remote_image_path("image/jpeg").ends_with(".jpg"));
    assert!(remote_image_path("image/tiff").ends_with(".png"));
    assert!(remote_image_path("image/png").starts_with("/tmp/warp-image-"));
}

#[test]
fn long_socket_paths_are_refused() {
    assert!(fits_socket_path_limit(Path::new(
        "/home/me/.ssh/warp-cm/0123456789ab"
    )));
    let long = format!("/tmp/{}/.ssh/warp-cm/0123456789ab", "x".repeat(80));
    assert!(!fits_socket_path_limit(Path::new(&long)));
}
