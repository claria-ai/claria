use std::fs;

use claria_desktop::local_export::write_private_atomic;

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
