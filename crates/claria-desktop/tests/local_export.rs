use std::fs;

use claria_desktop::local_export::{set_private_permissions, write_private_atomic};

#[test]
fn export_replaces_atomically_with_private_permissions() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let destination = directory.path().join("report.docx");
    fs::write(&destination, b"old").expect("seed destination");

    write_private_atomic(&destination, b"PK genuine docx bytes").expect("atomic write");
    assert_eq!(
        fs::read(&destination).expect("read export"),
        b"PK genuine docx bytes"
    );
    let leftovers: Vec<_> = fs::read_dir(directory.path())
        .expect("read directory")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&destination)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[cfg(unix)]
#[test]
fn set_private_permissions_narrows_a_group_and_world_readable_file() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("transcription-temporary");
    fs::write(&path, b"patient audio scratch").expect("seed file");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("widen permissions");

    set_private_permissions(&path).expect("restrict permissions");

    let mode = fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

/// The restriction is reported honestly: a file it could not touch is an
/// error, never a silent success.
#[test]
fn set_private_permissions_fails_on_a_missing_file() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let missing = directory.path().join("never-written");

    assert!(set_private_permissions(&missing).is_err());
}
