//! The atomic 0600 write helpers, and specifically the difference between the
//! text path and the binary one.

use app_core::secure_file;

#[test]
fn a_binary_write_appends_no_trailing_newline() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("frame.bin");
    // A canonical frame is binary and its reader checks the length exactly, so
    // one extra byte is silent corruption rather than an error. Pin the length.
    let blob: Vec<u8> = (0u8..=255).cycle().take(4_096).collect();

    secure_file::write_and_replace_binary(&path, &blob).expect("binary write");

    let read_back = std::fs::read(&path).expect("read back");
    assert_eq!(read_back.len(), blob.len());
    assert_eq!(read_back, blob);
}

#[test]
fn the_text_write_still_appends_its_newline() {
    // Every configuration writer in this crate expects the newline. Changing
    // that is out of scope, so pin it against an accidental "cleanup".
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("config.json");

    secure_file::write_and_replace(&path, b"{}").expect("text write");

    assert_eq!(std::fs::read(&path).expect("read back"), b"{}\n");
}

#[cfg(unix)]
#[test]
fn a_binary_write_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("frame.bin");

    secure_file::write_and_replace_binary(&path, b"bytes").expect("binary write");

    let mode = std::fs::metadata(&path)
        .expect("metadata")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn a_binary_write_replaces_an_existing_file_whole() {
    // Atomic replacement, not a truncating rewrite: a shorter payload must not
    // leave a tail of the longer one behind it.
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("frame.bin");

    secure_file::write_and_replace_binary(&path, &[0xAAu8; 512]).expect("first write");
    secure_file::write_and_replace_binary(&path, &[0xBBu8; 8]).expect("second write");

    assert_eq!(std::fs::read(&path).expect("read back"), vec![0xBBu8; 8]);
}
