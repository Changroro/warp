use settings::macros::define_settings_group;
use settings::{RespectUserSyncSetting, SupportedPlatforms, SyncToCloud};

use crate::terminal::ssh::util::SshHostProfile;

define_settings_group!(SshSettings,
    settings: [
        reuse_existing_control_master: ReuseExistingSshControlMaster {
            type: bool,
            default: false,
            supported_platforms: SupportedPlatforms::ALL,
            sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
            surface: settings::SettingSurfaces::GUI,
            private: false,
            storage_key: "ReuseExistingSshControlMaster",
            toml_path: "warpify.ssh.reuse_existing_control_master",
            description: "Whether the legacy SSH wrapper attaches to an existing SSH ControlMaster for the destination host instead of always creating its own.",
        },
        saved_ssh_host_profiles: SavedSshHostProfiles {
            type: Vec<SshHostProfile>,
            default: Vec::new(),
            supported_platforms: SupportedPlatforms::ALL,
            // SSH profile metadata can reveal private infrastructure. Keep it private and out of
            // cloud sync; passwords are stored separately in secure storage.
            sync_to_cloud: SyncToCloud::Never,
            private: true,
            storage_key: "SavedSSHHostProfiles",
            toml_path: "ssh.saved_profiles",
            description: "Saved SSH host profiles. Passwords are stored separately in secure storage.",
        },
    ]
);
