//! The producer credential follows the contract `image_sources.rs` and
//! `registry.rs` already use: digests on disk, plaintext returned exactly once,
//! constant-time comparison, and revocation independent of every other
//! credential the producer holds.

use server::producer_credentials::ProducerCredentialStore;

fn store() -> (tempfile::TempDir, ProducerCredentialStore) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = ProducerCredentialStore::open(dir.path().to_path_buf()).expect("open");
    (dir, store)
}

#[test]
fn a_minted_credential_authenticates_to_its_integration() {
    let (_dir, store) = store();
    let minted = store.mint("google").expect("mint");
    assert_eq!(store.authenticate(&minted.token).as_deref(), Some("google"));
}

#[test]
fn an_unknown_credential_authenticates_to_nothing() {
    let (_dir, store) = store();
    store.mint("google").expect("mint");
    assert!(store.authenticate("not-a-real-token").is_none());
}

#[test]
fn what_lands_on_disk_is_the_sha256_of_the_credential() {
    // Asserting only that the plaintext is ABSENT is too weak, and a mutation
    // probe proved it: a `token_digest` that copied the token's bytes verbatim
    // instead of hashing them passed that check, because the bytes are
    // hex-encoded on the way to disk and the substring never appears. So pin
    // the contract positively -- the stored value must BE the hash.
    use sha2::{Digest, Sha256};

    let (dir, store) = store();
    let minted = store.mint("google").expect("mint");
    let on_disk =
        std::fs::read_to_string(dir.path().join("producer-credentials.json")).expect("store file");

    let digest: [u8; 32] = Sha256::digest(minted.token.as_bytes()).into();
    // `digest_hex` is only the encoding; the assertion that carries weight is
    // that the bytes are SHA-256 of the token.
    let expected = protocol::digest_hex(&digest);
    assert!(
        on_disk.contains(&expected),
        "the stored value is not SHA-256(token)"
    );
    assert!(
        !on_disk.contains(&minted.token),
        "the plaintext credential was persisted"
    );
}

#[test]
fn minting_again_rotates_and_retires_the_previous_credential() {
    let (_dir, store) = store();
    let first = store.mint("google").expect("first mint");
    let second = store.mint("google").expect("second mint");
    assert_ne!(first.token, second.token);
    assert!(
        store.authenticate(&first.token).is_none(),
        "a rotated credential must stop working"
    );
    assert_eq!(store.authenticate(&second.token).as_deref(), Some("google"));
}

#[test]
fn revoking_stops_the_credential_and_reports_whether_one_existed() {
    let (_dir, store) = store();
    let minted = store.mint("google").expect("mint");
    assert!(store.revoke("google").expect("revoke"));
    assert!(store.authenticate(&minted.token).is_none());
    assert!(!store.revoke("google").expect("second revoke"));
}

#[test]
fn revoking_one_integration_leaves_another_alone() {
    // Revocation coupling (Task 5) revokes by integration id; if that reached
    // wider, disconnecting one integration would silently break every producer.
    let (_dir, store) = store();
    let google = store.mint("google").expect("mint google");
    let other = store.mint("dropbox").expect("mint dropbox");
    store.revoke("google").expect("revoke google");
    assert!(store.authenticate(&google.token).is_none());
    assert_eq!(
        store.authenticate(&other.token).as_deref(),
        Some("dropbox"),
        "revoking one integration must not touch another's credential"
    );
}

#[test]
fn a_credential_survives_a_restart() {
    let dir = tempfile::tempdir().expect("tempdir");
    let minted = {
        let store = ProducerCredentialStore::open(dir.path().to_path_buf()).expect("open");
        store.mint("google").expect("mint")
    };
    let reopened = ProducerCredentialStore::open(dir.path().to_path_buf()).expect("reopen");
    assert_eq!(
        reopened.authenticate(&minted.token).as_deref(),
        Some("google")
    );
}

#[test]
fn opening_a_fresh_root_yields_an_empty_store() {
    let (_dir, store) = store();
    assert!(store.authenticate("anything").is_none());
    assert!(!store.revoke("google").expect("revoke on empty store"));
}
