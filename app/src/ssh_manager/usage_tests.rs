use chrono::Duration;
use uuid::Uuid;

use super::*;

#[test]
fn record_connection_moves_to_front_dedups_and_caps_length() {
    let now = Utc::now();
    let ids = (0..MAX_HISTORY + 1)
        .map(|_| Uuid::new_v4())
        .collect::<Vec<_>>();
    let mut usage = SshHostUsage::default();
    for id in &ids {
        usage.record_connection(SshDestination::Profile(*id), now);
    }
    assert_eq!(usage.history.len(), MAX_HISTORY);
    assert_eq!(usage.last_connected(ids[0]), None);

    usage.record_connection(SshDestination::Profile(ids[2]), now + Duration::seconds(5));
    assert_eq!(usage.history.len(), MAX_HISTORY);
    assert_eq!(
        usage.recent_profiles().next().map(|(id, _)| id),
        Some(ids[2])
    );
    assert_eq!(
        usage.last_connected(ids[2]),
        Some((now + Duration::seconds(5)).timestamp())
    );
}

#[test]
fn quick_connects_are_tracked_apart_from_profiles() {
    let now = Utc::now();
    let id = Uuid::new_v4();
    let mut usage = SshHostUsage::default();
    usage.record_connection(SshDestination::Profile(id), now);
    usage.record_connection(SshDestination::QuickConnect("root@box".to_string()), now);

    assert_eq!(usage.recent_profiles().count(), 1);
    assert_eq!(
        usage
            .recent_quick_connects()
            .map(|(args, _)| args)
            .collect::<Vec<_>>(),
        ["root@box"]
    );
    usage.forget_quick_connect("root@box");
    assert_eq!(usage.recent_quick_connects().count(), 0);
    assert_eq!(usage.recent_profiles().count(), 1);
}

#[test]
fn toggle_pinned_adds_then_removes() {
    let id = Uuid::new_v4();
    let mut usage = SshHostUsage::default();
    usage.toggle_pinned(id);
    assert!(usage.is_pinned(id));
    usage.toggle_pinned(id);
    assert!(!usage.is_pinned(id));
}

#[test]
fn format_relative_time_buckets() {
    let now = Utc::now();
    let ago = |seconds: i64| format_relative_time(now.timestamp() - seconds, now);
    assert_eq!(ago(10), "just now");
    assert_eq!(ago(120), "2m ago");
    assert_eq!(ago(3 * 3_600), "3h ago");
    assert_eq!(ago(90_000), "yesterday");
    assert_eq!(ago(3 * 86_400), "3d ago");
    assert!(!ago(30 * 86_400).is_empty());
}
