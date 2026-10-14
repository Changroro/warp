use super::*;

#[test]
fn network_drops_are_retryable() {
    let output =
        "WARN slow query\npacket_write_wait: Connection to 10.0.3.21 port 22: Broken pipe\n";
    assert_eq!(
        disconnect_reason(output),
        Some(DisconnectReason {
            message: "packet_write_wait: Connection to 10.0.3.21 port 22: Broken pipe".to_string(),
            is_network_failure: true,
        })
    );
    let reason = disconnect_reason("Connection to web closed by remote host.").unwrap();
    assert!(reason.is_network_failure);
}

#[test]
fn logouts_and_auth_failures_are_not_retried() {
    let logout = disconnect_reason("logout\nConnection to web closed.\n").unwrap();
    assert!(!logout.is_network_failure);
    let denied = disconnect_reason("root@web: Permission denied (publickey).").unwrap();
    assert!(!denied.is_network_failure);
}

#[test]
fn unrelated_output_has_no_reason() {
    assert_eq!(disconnect_reason("ls\nfoo bar\n"), None);
}
