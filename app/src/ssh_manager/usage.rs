use chrono::{DateTime, Local, TimeZone as _, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const MAX_HISTORY: usize = 20;

/// Which saved SSH hosts the user pinned, and the SSH connections they opened most recently.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[schemars(description = "Pinned SSH host profiles and recent SSH connections.")]
pub struct SshHostUsage {
    #[serde(default)]
    pub pinned: Vec<Uuid>,
    /// Most recent first, at most one entry per destination.
    #[serde(default)]
    pub history: Vec<SshConnectionRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[schemars(description = "One SSH connection the user opened.")]
pub struct SshConnectionRecord {
    pub destination: SshDestination,
    /// Unix seconds.
    pub connected_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[schemars(description = "What an SSH connection was opened to.")]
pub enum SshDestination {
    Profile(Uuid),
    /// The `ssh` arguments of a quick connect.
    QuickConnect(String),
}

impl settings_value::SettingsValue for SshHostUsage {}

impl SshHostUsage {
    pub fn is_pinned(&self, profile_id: Uuid) -> bool {
        self.pinned.contains(&profile_id)
    }

    pub fn toggle_pinned(&mut self, profile_id: Uuid) {
        if self.is_pinned(profile_id) {
            self.pinned.retain(|id| *id != profile_id);
        } else {
            self.pinned.push(profile_id);
        }
    }

    pub fn record_connection(&mut self, destination: SshDestination, now: DateTime<Utc>) {
        self.history
            .retain(|record| record.destination != destination);
        self.history.insert(
            0,
            SshConnectionRecord {
                destination,
                connected_at: now.timestamp(),
            },
        );
        self.history.truncate(MAX_HISTORY);
    }

    pub fn forget_quick_connect(&mut self, args: &str) {
        self.history.retain(|record| {
            !matches!(&record.destination, SshDestination::QuickConnect(other) if other == args)
        });
    }

    pub fn last_connected(&self, profile_id: Uuid) -> Option<i64> {
        self.history.iter().find_map(|record| {
            (record.destination == SshDestination::Profile(profile_id))
                .then_some(record.connected_at)
        })
    }

    /// Saved profiles by most recent connection.
    pub fn recent_profiles(&self) -> impl Iterator<Item = (Uuid, i64)> + '_ {
        self.history
            .iter()
            .filter_map(|record| match record.destination {
                SshDestination::Profile(id) => Some((id, record.connected_at)),
                SshDestination::QuickConnect(_) => None,
            })
    }

    /// Quick connects by most recent connection.
    pub fn recent_quick_connects(&self) -> impl Iterator<Item = (&str, i64)> + '_ {
        self.history
            .iter()
            .filter_map(|record| match &record.destination {
                SshDestination::QuickConnect(args) => Some((args.as_str(), record.connected_at)),
                SshDestination::Profile(_) => None,
            })
    }
}

/// Formats a past unix time relative to `now`, e.g. "2m ago" or "Oct 11".
pub fn format_relative_time(timestamp: i64, now: DateTime<Utc>) -> String {
    let seconds = (now.timestamp() - timestamp).max(0);
    match seconds {
        0..60 => "just now".to_string(),
        60..3_600 => format!("{}m ago", seconds / 60),
        3_600..86_400 => format!("{}h ago", seconds / 3_600),
        86_400..172_800 => "yesterday".to_string(),
        172_800..604_800 => format!("{}d ago", seconds / 86_400),
        _ => Local
            .timestamp_opt(timestamp, 0)
            .single()
            .map(|time| time.format("%b %-d").to_string())
            .unwrap_or_default(),
    }
}

#[cfg(test)]
#[path = "usage_tests.rs"]
mod tests;
