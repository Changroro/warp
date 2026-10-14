use chrono::Utc;

use super::*;
use crate::ssh_manager::usage::SshDestination;

fn profile(name: &str, host: &str) -> SshHostProfile {
    SshHostProfile::new(name.to_string(), host.to_string())
}

fn row_kinds(items: &[Item]) -> Vec<&'static str> {
    rows(items)
        .map(|row| match row {
            Row::QuickConnect { .. } => "quick",
            Row::Host { .. } => "host",
            Row::RecentQuickConnect { .. } => "recent-quick",
            Row::SaveAndConnect { .. } => "save",
            Row::ShowAll => "all",
        })
        .collect()
}

#[test]
fn destination_input_leads_with_quick_connect_and_offers_save() {
    let profiles = vec![profile("staging-api", "10.0.3.45")];
    let items = build_items("deploy@10.0.3.45:2222", &profiles, &SshHostUsage::default());
    assert_eq!(row_kinds(&items), ["quick", "host", "save"]);
}

#[test]
fn plain_search_lists_matches_before_quick_connect() {
    let profiles = vec![
        profile("staging-api", "10.0.3.45"),
        profile("prod", "10.0.0.1"),
    ];
    let items = build_items("staging", &profiles, &SshHostUsage::default());
    assert_eq!(row_kinds(&items), ["host", "quick", "save"]);
}

#[test]
fn saved_destinations_are_not_offered_for_saving() {
    let profiles = vec![profile("box", "box")];
    let items = build_items("box", &profiles, &SshHostUsage::default());
    assert_eq!(row_kinds(&items), ["host", "quick"]);
}

#[test]
fn empty_input_shows_pinned_recent_quick_connects_and_all_hosts() {
    let pinned = profile("pinned", "a");
    let recent = profile("recent", "b");
    let other = profile("other", "c");
    let mut usage = SshHostUsage::default();
    usage.toggle_pinned(pinned.id);
    let now = Utc::now();
    usage.record_connection(SshDestination::Profile(recent.id), now);
    usage.record_connection(SshDestination::Profile(pinned.id), now);
    usage.record_connection(SshDestination::QuickConnect("root@x".to_string()), now);

    let items = build_items("", &[pinned, recent, other], &usage);
    assert_eq!(
        row_kinds(&items),
        ["host", "host", "recent-quick", "host", "all"]
    );
}
