//! Pins that the atomic + 0o600 file machinery the secrets store depends on is
//! reachable from the server crate as public `app_core::secure_file` API. If it
//! regresses to `pub(crate)`, this fails to compile -- which is the point.

use app_core::secure_file::{read_bounded, write_and_replace};

#[test]
fn write_and_read_round_trips_through_public_api() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("probe.bin");

    write_and_replace(&path, b"payload").expect("write");
    let read = read_bounded(&path, 1024).expect("read").expect("present");

    // write_and_replace appends a trailing newline; the byte payload survives.
    assert_eq!(&read[..b"payload".len()], b"payload");
}

#[cfg(unix)]
#[test]
fn written_file_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("probe.bin");
    write_and_replace(&path, b"x").expect("write");

    let mode = std::fs::metadata(&path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "secure writes must be owner read/write only");
}
