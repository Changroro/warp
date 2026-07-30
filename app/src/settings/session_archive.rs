use settings::macros::define_settings_group;
use settings::{SupportedPlatforms, SyncToCloud};

define_settings_group!(SessionArchiveSettings,
    settings: [
        archive_section_collapsed: SessionArchiveSectionCollapsed {
            type: bool,
            default: true,
            supported_platforms: SupportedPlatforms::ALL,
            sync_to_cloud: SyncToCloud::Never,
            surface: settings::SettingSurfaces::GUI,
            private: false,
            storage_key: "SessionArchiveSectionCollapsed",
            toml_path: "session_archive.section_collapsed",
            description: "Whether the archive section in the vertical tabs panel is collapsed.",
        },
    ]
);
