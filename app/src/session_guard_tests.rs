use super::*;

#[test]
fn only_a_known_different_owner_is_another_account() {
    assert!(is_other_account(Some("alice\n"), "bob"));
    assert!(!is_other_account(Some("bob\n"), "bob"));
    assert!(!is_other_account(Some(""), "bob"));
    assert!(!is_other_account(None, "bob"));
}

#[cfg(feature = "local_fs")]
mod backups {
    use std::fs;
    use std::path::Path;

    use diesel::connection::SimpleConnection as _;
    use diesel::{Connection as _, SqliteConnection};

    use super::super::backup::back_up_layout;

    fn database(dir: &Path) -> (SqliteConnection, std::path::PathBuf) {
        let path = dir.join("warp.sqlite");
        let mut conn = SqliteConnection::establish(&path.to_string_lossy()).unwrap();
        conn.batch_execute(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE tabs (id INTEGER PRIMARY KEY, custom_title TEXT);
             CREATE TABLE blocks (id INTEGER PRIMARY KEY, output TEXT);
             INSERT INTO tabs (custom_title) VALUES ('server'), ('ios');
             INSERT INTO blocks (output) VALUES ('a long terminal transcript');",
        )
        .unwrap();
        (conn, path)
    }

    #[test]
    fn backs_up_only_the_layout_and_skips_unchanged_layouts() {
        let dir = tempfile::tempdir().unwrap();
        let (mut conn, path) = database(dir.path());

        let first = back_up_layout(&path, "auto-", 30).unwrap().unwrap();
        let mut backup = SqliteConnection::establish(&first.to_string_lossy()).unwrap();
        backup
            .batch_execute("SELECT custom_title FROM tabs;")
            .expect("the layout is copied");
        assert!(
            backup.batch_execute("SELECT output FROM blocks;").is_err(),
            "terminal output stays out of the backup"
        );

        assert_eq!(
            back_up_layout(&path, "auto-", 30).unwrap(),
            None,
            "an unchanged layout adds no backup"
        );

        conn.batch_execute("UPDATE tabs SET custom_title = 'renamed' WHERE id = 1;")
            .unwrap();
        assert!(back_up_layout(&path, "auto-", 30).unwrap().is_some());
    }

    #[test]
    fn keeps_only_the_newest_backups_per_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let (mut conn, path) = database(dir.path());
        let backup_dir = dir.path().join("session-backups");
        fs::create_dir_all(&backup_dir).unwrap();
        for stamp in ["20200101-000000", "20200102-000000", "20200103-000000"] {
            fs::write(backup_dir.join(format!("auto-{stamp}.sqlite")), stamp).unwrap();
        }
        fs::write(backup_dir.join("logout-20200101-000000.sqlite"), "logout").unwrap();

        conn.batch_execute("UPDATE tabs SET custom_title = 'changed';")
            .unwrap();
        let newest = back_up_layout(&path, "auto-", 2).unwrap().unwrap();

        let mut left: Vec<_> = fs::read_dir(&backup_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "auto-20200103-000000.sqlite".to_string(),
                newest.file_name().unwrap().to_string_lossy().into_owned(),
                "logout-20200101-000000.sqlite".to_string(),
            ]
        );
    }

    #[test]
    fn a_missing_database_is_not_backed_up() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            back_up_layout(&dir.path().join("warp.sqlite"), "auto-", 30).unwrap(),
            None
        );
    }
}
