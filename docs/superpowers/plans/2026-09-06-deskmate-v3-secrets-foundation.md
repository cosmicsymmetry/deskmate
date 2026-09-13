# V3 Secrets-at-Rest Foundation (`IntegrationStore`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **STATUS (recorded 2026-09-08): DELIVERED. The checkboxes below were never ticked
> during execution and are NOT a progress signal — read the commits, not the boxes.**
> Sub-project 1 landed as six commits, `994a422`..`e472113`, with the full companion
> gate clean and a whole-branch review returning APPROVE. Two carry-forward facts the
> boxes do not record: `IntegrationStore` holds a sync `Mutex` across disk I/O, so any
> async Axum handler calling `put`/`remove` must go through `spawn_blocking`; and the
> key-source fail-closed precedence plus the redacted `IntegrationSecret` `Debug` are
> pinned by `e472113`.

**Goal:** Build the server-side `IntegrationStore` — an encrypted-at-rest store for per-integration OAuth secrets (refresh tokens, client secrets) with a keyfile-based master key that fails closed at startup.

**Architecture:** A new `secrets` module in the **server** crate holds an `IntegrationStore` keyed by integration id. Plaintext secrets are a JSON map held in memory; at rest they are one `secrets.enc` file — `MAGIC || 24-byte nonce || XChaCha20-Poly1305 ciphertext`, base64-encoded on a single line, written atomically at `0o600` through `app-core`'s existing `secure_file` machinery (promoted from `pub(crate)` to `pub` so there is one implementation, not a second copy alongside the one `registry.rs` already reinvented). The 32-byte master key is read once at startup from a `0o600` keyfile outside the config dir (env-base64 fallback); it never touches disk again and is zeroized on drop. This sub-project is storage + key custody only — no OAuth, no HTTP, no provider (sub-projects 2–4).

**Tech Stack:** Rust (server crate), `chacha20poly1305` (XChaCha20-Poly1305 AEAD — takes a raw 32-byte key directly, no scrypt/passphrase indirection, stays in the RustCrypto family the crate already pulls via `sha2`), `zeroize` (key wiping), `base64` 0.22 and `serde_json` (already server deps), `app_core::secure_file` (atomic `0o600` writes, bounded reads, fsync-parent).

**Spec:** `docs/superpowers/specs/2026-09-06-deskmate-v3-server-host-design.md` (§1.1 secret storage, §6 security, §8 sub-project 1, §9.1 automated coverage).

## Global Constraints

- **Server crate only.** `IntegrationStore` lives in `companion/crates/server/src/`, which depends on `app-core`. Do **not** put it in `app-core` — that would violate the `app-core ← plugin` dependency direction the project holds (CLAUDE.md).
- **No firmware/wire/schema change.** This sub-project is invisible to the device: no protocol, capability, or config-schema byte moves. It adds no route and no device-facing behaviour.
- **Encrypt at rest, fail closed.** The refresh token is long-lived read access to the owner's calendar; it is never written in plaintext. The process must refuse to start if a `secrets.enc` exists but no key is configured (spec §1.1).
- **Key lives outside the config directory.** A config-directory backup must not carry the key. Reject a keyfile whose canonical path is inside the config dir (spec §1.1).
- **Keyfile must be `0o600`.** Reject a keyfile with any group/other permission bits (unix).
- **Pin exact dependency versions**, mirroring how `server/Cargo.toml` pins `resvg = "=0.45.1"` and `roxmltree = "=0.20.0"`: `chacha20poly1305 = "=0.10.1"`, `zeroize = { version = "=1.9.0", features = ["derive"] }`.
- **Threat model, stated so it is not overclaimed (spec §1.1):** encryption at rest defends against *offline* exposure (stolen disk, backup, snapshot, accidental commit). It does **not** defend against a live-server compromise — a process that can read the keyfile and ciphertext can mint tokens, because it must, to use the calendar. Do not add code claiming otherwise.
- **Verification per `CLAUDE.md`:** from `companion/`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --all-targets`, `cargo test --workspace --doc`. Server tests bind loopback and must be run by the controller, not a sandboxed subagent (project memory).

---

### Task 1: Expose `app-core::secure_file` as public API

**Why option (a), not a server-side wrapper:** the server crate already reimplements this exact atomic + `0o600` + fsync-parent machinery inside `registry.rs` (its own `AtomicWriteFile`, `set_permissions(0o600)`, `usable_parent`, `sync_parent`). Promoting the canonical copy in `app-core` keeps **one** implementation and lets `IntegrationStore` reuse it directly; a wrapper would make a third copy. The change is visibility-only and additive.

**Files:**
- Modify: `companion/crates/app-core/src/lib.rs:10` (`mod secure_file;` → `pub mod secure_file;`)
- Modify: `companion/crates/app-core/src/secure_file.rs` (flip `pub(crate)` → `pub` on the items `IntegrationStore` needs)
- Test: `companion/crates/server/tests/secure_file_reexport.rs` (new)

**Interfaces:**
- Consumes: nothing.
- Produces (now callable as `app_core::secure_file::…`):
  - `pub fn write_and_replace(target: &Path, bytes: &[u8]) -> Result<(), FileIoError>` — atomic `0o600` write, appends a trailing `\n`, fsyncs and renames.
  - `pub fn read_bounded(path: &Path, maximum: usize) -> Result<Option<Vec<u8>>, BoundedReadError>` — `Ok(None)` if absent; `Err(TooLarge)` past `maximum`.
  - `pub fn create_directory(path: &Path) -> Result<(), FileIoError>`
  - `pub fn usable_parent(path: &Path) -> Option<&Path>`
  - `pub fn sync_parent(parent: &Path) -> Result<(), FileIoError>`
  - `pub enum BoundedReadError { Io(FileIoError), TooLarge { maximum: usize } }`
  - `pub struct FileIoError { pub operation: FileOperation, pub source: std::io::Error }` with `pub fn into_strings(self, subject: &str) -> (String, String)`
  - `pub enum FileOperation { … }` with `pub fn describe(self, subject: &str) -> String`

- [ ] **Step 1: Write the failing test**

Create `companion/crates/server/tests/secure_file_reexport.rs`:

```rust
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

    let mode = std::fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "secure writes must be owner read/write only");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd companion && cargo test -p server --test secure_file_reexport`
Expected: FAIL — compile error, `module secure_file is private` / `function write_and_replace is private`.

- [ ] **Step 3: Make the minimal change**

In `companion/crates/app-core/src/lib.rs`, change line 10 from:

```rust
mod secure_file;
```

to:

```rust
pub mod secure_file;
```

In `companion/crates/app-core/src/secure_file.rs`, change the visibility of exactly these items from `pub(crate)` to `pub` (leave `secure_open`, `secure_atomic_options`, and `read_bounded_with_mode_repair` unchanged — they are internal):

```rust
pub enum BoundedReadError {
    Io(FileIoError),
    TooLarge { maximum: usize },
}

pub struct FileIoError {
    pub operation: FileOperation,
    pub source: io::Error,
}
// ... impl FileIoError { pub fn into_strings(...) } is already `pub`; leave it.

pub enum FileOperation { /* variants unchanged */ }
// ... impl FileOperation { pub fn describe(...) } is already `pub`; leave it.

pub fn read_bounded(path: &Path, maximum: usize) -> Result<Option<Vec<u8>>, BoundedReadError> { /* body unchanged */ }

pub fn usable_parent(path: &Path) -> Option<&Path> { /* body unchanged */ }

pub fn create_directory(path: &Path) -> Result<(), FileIoError> { /* body unchanged */ }

pub fn write_and_replace(target: &Path, bytes: &[u8]) -> Result<(), FileIoError> { /* body unchanged */ }

// sync_parent has two cfg arms; make BOTH `pub`:
#[cfg(unix)]
pub fn sync_parent(parent: &Path) -> Result<(), FileIoError> { /* body unchanged */ }

#[cfg(not(unix))]
pub fn sync_parent(_parent: &Path) -> Result<(), FileIoError> { /* body unchanged */ }
```

Only the leading `pub(crate)` tokens change to `pub`; do not touch any function body.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd companion && cargo test -p server --test secure_file_reexport && cargo build -p app-core`
Expected: PASS (both tests), and `app-core` still builds — the internal callers in `store.rs`/`registry` are unaffected by a widening of visibility.

- [ ] **Step 5: Commit**

```bash
git add companion/crates/app-core/src/lib.rs companion/crates/app-core/src/secure_file.rs companion/crates/server/tests/secure_file_reexport.rs
git commit -m "refactor: expose app-core secure_file as public API for the secrets store"
```

---

### Task 2: AEAD envelope and the `SecretsKey` type

Add the crypto dependencies, create the `secrets` module, and implement the sealed-file format (`seal`/`open`) plus the zeroizing 32-byte key type. Pure functions over a key and bytes — no filesystem, no env yet.

**Files:**
- Modify: `companion/crates/server/Cargo.toml` (add deps)
- Create: `companion/crates/server/src/secrets.rs`
- Modify: `companion/crates/server/src/lib.rs` (add `pub mod secrets;` beside the other `pub mod` lines)
- Test: inline `#[cfg(test)] mod tests` in `secrets.rs`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub struct SecretsKey` — wraps `[u8; 32]`, `ZeroizeOnDrop`, redacting `Debug`, no `Clone`.
    - `pub fn from_bytes(bytes: [u8; 32]) -> Self`
    - `pub(crate) fn bytes(&self) -> &[u8; 32]`
  - `pub enum SecretsError { Io { operation: String, detail: String }, TooLarge { maximum: usize }, BadFormat, Decrypt, Encrypt, Serialize(String) }` (derives `Debug`, `thiserror::Error`)
  - `fn seal(key: &SecretsKey, plaintext: &[u8]) -> Result<Vec<u8>, SecretsError>` — returns the base64 line bytes (no trailing newline).
  - `fn open(key: &SecretsKey, file_bytes: &[u8]) -> Result<Vec<u8>, SecretsError>` — trims trailing ASCII whitespace, base64-decodes, checks magic, decrypts.
  - `const SECRETS_MAGIC: &[u8; 4] = b"DMS1";`
  - `const MAX_SECRETS_FILE_BYTES: usize = 262_144;`

- [ ] **Step 1: Add dependencies**

In `companion/crates/server/Cargo.toml`, under `[dependencies]` (keep the existing alphabetical-ish grouping; place near `base64`), add:

```toml
# XChaCha20-Poly1305 AEAD for the at-rest OAuth secrets blob (spec §1.1). Takes a
# raw 32-byte key directly -- no scrypt/passphrase indirection -- and stays in the
# RustCrypto family already pulled via `sha2`. Pinned exactly, matching resvg/roxmltree.
chacha20poly1305 = "=0.10.1"
# Wipes the 32-byte master key from memory on drop.
zeroize = { version = "=1.9.0", features = ["derive"] }
```

- [ ] **Step 2: Write the failing test**

Create `companion/crates/server/src/secrets.rs` with only the test module first (the items it references do not exist yet):

```rust
//! Encrypted-at-rest storage for per-integration OAuth secrets (spec §1.1, §6).
//! Server-crate only; never depended on by `app-core`.

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key(byte: u8) -> SecretsKey {
        SecretsKey::from_bytes([byte; 32])
    }

    #[test]
    fn seal_then_open_round_trips() {
        let key = test_key(7);
        let sealed = seal(&key, b"a refresh token").expect("seal");
        let opened = open(&key, &sealed).expect("open");
        assert_eq!(opened, b"a refresh token");
    }

    #[test]
    fn open_tolerates_a_trailing_newline() {
        // write_and_replace appends `\n`; open must strip trailing whitespace.
        let key = test_key(7);
        let mut sealed = seal(&key, b"payload").expect("seal");
        sealed.push(b'\n');
        assert_eq!(open(&key, &sealed).expect("open"), b"payload");
    }

    #[test]
    fn wrong_key_fails_closed() {
        let sealed = seal(&test_key(1), b"payload").expect("seal");
        let error = open(&test_key(2), &sealed).expect_err("wrong key must fail");
        assert!(matches!(error, SecretsError::Decrypt));
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        use base64::prelude::{BASE64_STANDARD, Engine as _};

        let key = test_key(9);
        let sealed = seal(&key, b"payload").expect("seal");
        let mut raw = BASE64_STANDARD.decode(&sealed).expect("decode");
        let last = raw.len() - 1;
        raw[last] ^= 0xff; // flip a ciphertext/tag byte
        let retampered = BASE64_STANDARD.encode(&raw).into_bytes();
        assert!(matches!(open(&key, &retampered), Err(SecretsError::Decrypt)));
    }

    #[test]
    fn truncated_file_is_rejected() {
        let key = test_key(3);
        let sealed = seal(&key, b"payload").expect("seal");
        let truncated = &sealed[..sealed.len() / 2];
        assert!(open(&key, truncated).is_err());
    }

    #[test]
    fn wrong_magic_is_bad_format() {
        use base64::prelude::{BASE64_STANDARD, Engine as _};

        let key = test_key(4);
        let sealed = seal(&key, b"payload").expect("seal");
        let mut raw = BASE64_STANDARD.decode(&sealed).expect("decode");
        raw[0] ^= 0xff; // corrupt the magic prefix
        let broken = BASE64_STANDARD.encode(&raw).into_bytes();
        assert!(matches!(open(&key, &broken), Err(SecretsError::BadFormat)));
    }

    #[test]
    fn debug_redacts_the_key() {
        assert_eq!(format!("{:?}", test_key(5)), "SecretsKey(redacted)");
    }
}
```

Also add the module declaration in `companion/crates/server/src/lib.rs`, beside the existing `pub mod plugin_host;` etc.:

```rust
pub mod secrets;
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cd companion && cargo test -p server secrets::`
Expected: FAIL — compile error, `cannot find function seal` / `cannot find type SecretsKey`.

- [ ] **Step 4: Write the implementation**

Prepend to `companion/crates/server/src/secrets.rs` (above the test module):

```rust
use base64::prelude::{BASE64_STANDARD, Engine as _};
use chacha20poly1305::aead::{Aead, KeyInit, OsRng, Payload};
use chacha20poly1305::{AeadCore, XChaCha20Poly1305, XNonce};
use zeroize::ZeroizeOnDrop;

/// Format magic + version for `secrets.enc`. The `1` is the format version; a
/// future rotation bumps it and `open` learns to accept both. It is also the
/// AEAD associated data, so flipping it fails authentication as well as the
/// prefix check.
const SECRETS_MAGIC: &[u8; 4] = b"DMS1";
const NONCE_LEN: usize = 24;
/// Generous ceiling: integration secrets are a small JSON map. Bounds a hostile
/// or corrupt file before it is base64-decoded.
const MAX_SECRETS_FILE_BYTES: usize = 262_144;

/// A 32-byte master key, held in memory only, wiped on drop. No `Clone`: the key
/// is moved into the one `IntegrationStore` that owns it.
#[derive(ZeroizeOnDrop)]
pub struct SecretsKey {
    bytes: [u8; 32],
}

impl SecretsKey {
    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self { bytes }
    }

    pub(crate) fn bytes(&self) -> &[u8; 32] {
        &self.bytes
    }
}

impl std::fmt::Debug for SecretsKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SecretsKey(redacted)")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SecretsError {
    #[error("failed to {operation}: {detail}")]
    Io { operation: String, detail: String },
    #[error("secrets file exceeds {maximum} bytes")]
    TooLarge { maximum: usize },
    #[error("secrets file is not a recognized Deskmate secrets envelope")]
    BadFormat,
    #[error("secrets could not be decrypted: wrong key or tampered file")]
    Decrypt,
    #[error("secrets could not be encrypted")]
    Encrypt,
    #[error("secrets could not be (de)serialized: {0}")]
    Serialize(String),
}

/// Encrypts `plaintext` into a single base64 line: `MAGIC || nonce || ciphertext`.
/// The returned bytes carry no trailing newline; `write_and_replace` adds one.
fn seal(key: &SecretsKey, plaintext: &[u8]) -> Result<Vec<u8>, SecretsError> {
    let cipher = XChaCha20Poly1305::new_from_slice(key.bytes())
        .expect("SecretsKey is 32 bytes by construction");
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad: SECRETS_MAGIC,
            },
        )
        .map_err(|_| SecretsError::Encrypt)?;

    let mut buffer = Vec::with_capacity(SECRETS_MAGIC.len() + NONCE_LEN + ciphertext.len());
    buffer.extend_from_slice(SECRETS_MAGIC);
    buffer.extend_from_slice(nonce.as_slice());
    buffer.extend_from_slice(&ciphertext);
    Ok(BASE64_STANDARD.encode(&buffer).into_bytes())
}

/// Inverse of [`seal`]. Trims trailing ASCII whitespace (the persisted newline),
/// base64-decodes, checks the magic prefix, then authenticates and decrypts.
fn open(key: &SecretsKey, file_bytes: &[u8]) -> Result<Vec<u8>, SecretsError> {
    let trimmed = file_bytes.trim_ascii_end();
    let raw = BASE64_STANDARD
        .decode(trimmed)
        .map_err(|_| SecretsError::BadFormat)?;

    let header = SECRETS_MAGIC.len() + NONCE_LEN;
    if raw.len() < header || &raw[..SECRETS_MAGIC.len()] != SECRETS_MAGIC {
        return Err(SecretsError::BadFormat);
    }
    let nonce = XNonce::from_slice(&raw[SECRETS_MAGIC.len()..header]);
    let ciphertext = &raw[header..];

    let cipher = XChaCha20Poly1305::new_from_slice(key.bytes())
        .expect("SecretsKey is 32 bytes by construction");
    cipher
        .decrypt(
            nonce,
            Payload {
                msg: ciphertext,
                aad: SECRETS_MAGIC,
            },
        )
        .map_err(|_| SecretsError::Decrypt)
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd companion && cargo test -p server secrets::`
Expected: PASS — all seven tests.

- [ ] **Step 6: Commit**

```bash
git add companion/crates/server/Cargo.toml companion/crates/server/Cargo.lock companion/crates/server/src/lib.rs companion/crates/server/src/secrets.rs
git commit -m "feat: add XChaCha20-Poly1305 sealed envelope and zeroizing key type for secrets"
```

---

### Task 3: Master-key acquisition (keyfile + env fallback)

Read the 32-byte key from a `0o600` keyfile outside the config dir, or from a base64 env value as fallback, with typed rejection of every misuse. Pure over injected values; a thin `_from_env` wrapper reads the environment.

**Files:**
- Modify: `companion/crates/server/src/secrets.rs`
- Test: inline `#[cfg(test)] mod tests` in `secrets.rs`

**Interfaces:**
- Consumes: `SecretsKey::from_bytes` (Task 2).
- Produces:
  - `pub enum KeyError { NotConfigured, KeyfileNotFound(PathBuf), KeyfileInsideConfigDir { keyfile: PathBuf, config_dir: PathBuf }, KeyfileWorldReadable { mode: u32 }, InvalidBase64, WrongLength { got: usize }, Io { operation: String, detail: String } }` (`Debug`, `thiserror::Error`)
  - `pub const ENV_KEY_FILE: &str = "DESKMATE_SECRETS_KEY_FILE";`
  - `pub const ENV_KEY: &str = "DESKMATE_SECRETS_KEY";`
  - `impl SecretsKey { pub fn from_base64(value: &str) -> Result<Self, KeyError>; pub fn from_keyfile(config_dir: &Path, keyfile: &Path) -> Result<Self, KeyError>; }`
  - `pub fn acquire_key(config_dir: &Path, keyfile: Option<&Path>, inline_base64: Option<&str>) -> Result<SecretsKey, KeyError>`
  - `pub fn acquire_key_from_env(config_dir: &Path) -> Result<SecretsKey, KeyError>`

- [ ] **Step 1: Write the failing test**

Add these tests inside the existing `mod tests` in `secrets.rs`:

```rust
    use std::path::Path;

    fn valid_key_base64() -> String {
        use base64::prelude::{BASE64_STANDARD, Engine as _};
        BASE64_STANDARD.encode([42u8; 32])
    }

    #[test]
    fn from_base64_accepts_thirty_two_bytes() {
        let key = SecretsKey::from_base64(&valid_key_base64()).expect("valid key");
        assert_eq!(key.bytes(), &[42u8; 32]);
    }

    #[test]
    fn from_base64_rejects_wrong_length() {
        use base64::prelude::{BASE64_STANDARD, Engine as _};
        let short = BASE64_STANDARD.encode([1u8; 16]);
        assert!(matches!(
            SecretsKey::from_base64(&short),
            Err(KeyError::WrongLength { got: 16 })
        ));
    }

    #[test]
    fn from_base64_rejects_garbage() {
        assert!(matches!(
            SecretsKey::from_base64("not base64 !!!"),
            Err(KeyError::InvalidBase64)
        ));
    }

    #[test]
    fn acquire_key_without_any_source_is_not_configured() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(matches!(
            acquire_key(dir.path(), None, None),
            Err(KeyError::NotConfigured)
        ));
    }

    #[test]
    fn acquire_key_reads_a_keyfile_outside_the_config_dir() {
        let config = tempfile::tempdir().expect("config dir");
        let keydir = tempfile::tempdir().expect("key dir");
        let keyfile = keydir.path().join("secrets.key");
        write_keyfile(&keyfile, &valid_key_base64(), 0o600);

        let key = acquire_key(config.path(), Some(&keyfile), None).expect("key loads");
        assert_eq!(key.bytes(), &[42u8; 32]);
    }

    #[test]
    fn acquire_key_prefers_the_keyfile_over_inline_base64() {
        use base64::prelude::{BASE64_STANDARD, Engine as _};
        let config = tempfile::tempdir().expect("config dir");
        let keydir = tempfile::tempdir().expect("key dir");
        let keyfile = keydir.path().join("secrets.key");
        write_keyfile(&keyfile, &BASE64_STANDARD.encode([7u8; 32]), 0o600);

        let key = acquire_key(config.path(), Some(&keyfile), Some(&valid_key_base64()))
            .expect("keyfile wins");
        assert_eq!(key.bytes(), &[7u8; 32]);
    }

    #[test]
    fn acquire_key_rejects_a_keyfile_inside_the_config_dir() {
        let config = tempfile::tempdir().expect("config dir");
        let keyfile = config.path().join("secrets.key");
        write_keyfile(&keyfile, &valid_key_base64(), 0o600);

        assert!(matches!(
            acquire_key(config.path(), Some(&keyfile), None),
            Err(KeyError::KeyfileInsideConfigDir { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn acquire_key_rejects_a_world_readable_keyfile() {
        let config = tempfile::tempdir().expect("config dir");
        let keydir = tempfile::tempdir().expect("key dir");
        let keyfile = keydir.path().join("secrets.key");
        write_keyfile(&keyfile, &valid_key_base64(), 0o644);

        assert!(matches!(
            acquire_key(config.path(), Some(&keyfile), None),
            Err(KeyError::KeyfileWorldReadable { mode }) if mode & 0o077 != 0
        ));
    }

    #[test]
    fn acquire_key_reports_a_missing_keyfile() {
        let config = tempfile::tempdir().expect("config dir");
        let missing = config.path().parent().unwrap().join("does-not-exist.key");
        assert!(matches!(
            acquire_key(config.path(), Some(&missing), None),
            Err(KeyError::KeyfileNotFound(_))
        ));
    }

    fn write_keyfile(path: &Path, contents: &str, mode: u32) {
        std::fs::write(path, contents).expect("write keyfile");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
                .expect("chmod keyfile");
        }
        #[cfg(not(unix))]
        let _ = mode;
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd companion && cargo test -p server secrets::`
Expected: FAIL — compile error, `cannot find function acquire_key` / `no variant KeyError`.

- [ ] **Step 3: Write the implementation**

Add to `secrets.rs` (below the Task 2 code, above the test module). Add `use std::path::{Path, PathBuf};` to the module's imports:

```rust
use std::path::{Path, PathBuf};

/// Environment variable naming a `0o600` keyfile that holds the base64 master key.
pub const ENV_KEY_FILE: &str = "DESKMATE_SECRETS_KEY_FILE";
/// Fallback environment variable carrying the base64 master key inline. Documented
/// as second-choice: env values are visible in `ps`/`/proc`/`systemctl show` in a
/// way a keyfile is not (spec §1.1).
pub const ENV_KEY: &str = "DESKMATE_SECRETS_KEY";

const MAX_KEYFILE_BYTES: usize = 1_024;

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error(
        "no secrets key configured: set {ENV_KEY_FILE} to a 0600 keyfile (preferred) \
         or {ENV_KEY} to the base64 key"
    )]
    NotConfigured,
    #[error("secrets keyfile not found: {0}")]
    KeyfileNotFound(PathBuf),
    #[error(
        "secrets keyfile {keyfile} is inside the config directory {config_dir}; the key \
         must live outside it so a config backup does not carry the key"
    )]
    KeyfileInsideConfigDir { keyfile: PathBuf, config_dir: PathBuf },
    #[error("secrets keyfile is group/other-accessible (mode {mode:#o}); must be 0600")]
    KeyfileWorldReadable { mode: u32 },
    #[error("secrets key is not valid base64")]
    InvalidBase64,
    #[error("secrets key must decode to 32 bytes, got {got}")]
    WrongLength { got: usize },
    #[error("failed to {operation}: {detail}")]
    Io { operation: String, detail: String },
}

impl SecretsKey {
    /// Decodes a base64 string into a 32-byte key.
    pub fn from_base64(value: &str) -> Result<Self, KeyError> {
        let decoded = BASE64_STANDARD
            .decode(value.trim())
            .map_err(|_| KeyError::InvalidBase64)?;
        let bytes: [u8; 32] = decoded
            .as_slice()
            .try_into()
            .map_err(|_| KeyError::WrongLength { got: decoded.len() })?;
        Ok(Self::from_bytes(bytes))
    }

    /// Reads a base64 key from a keyfile after checking it is (a) not inside the
    /// config directory and (b) `0o600`.
    pub fn from_keyfile(config_dir: &Path, keyfile: &Path) -> Result<Self, KeyError> {
        let canonical_keyfile = std::fs::canonicalize(keyfile)
            .map_err(|_| KeyError::KeyfileNotFound(keyfile.to_path_buf()))?;

        // Containment check on canonical paths, so a symlink or `..` cannot smuggle
        // the key back inside the config directory.
        if let Ok(canonical_config) = std::fs::canonicalize(config_dir) {
            if canonical_keyfile.starts_with(&canonical_config) {
                return Err(KeyError::KeyfileInsideConfigDir {
                    keyfile: canonical_keyfile,
                    config_dir: canonical_config,
                });
            }
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = std::fs::metadata(&canonical_keyfile).map_err(|error| KeyError::Io {
                operation: "stat the secrets keyfile".to_string(),
                detail: error.to_string(),
            })?;
            let mode = metadata.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                return Err(KeyError::KeyfileWorldReadable { mode });
            }
        }

        let bytes = std::fs::read(&canonical_keyfile).map_err(|error| KeyError::Io {
            operation: "read the secrets keyfile".to_string(),
            detail: error.to_string(),
        })?;
        if bytes.len() > MAX_KEYFILE_BYTES {
            return Err(KeyError::Io {
                operation: "read the secrets keyfile".to_string(),
                detail: format!("keyfile exceeds {MAX_KEYFILE_BYTES} bytes"),
            });
        }
        let text = String::from_utf8(bytes).map_err(|_| KeyError::InvalidBase64)?;
        Self::from_base64(&text)
    }
}

/// Resolves the master key from injected sources: keyfile wins over inline base64;
/// neither present is [`KeyError::NotConfigured`].
pub fn acquire_key(
    config_dir: &Path,
    keyfile: Option<&Path>,
    inline_base64: Option<&str>,
) -> Result<SecretsKey, KeyError> {
    if let Some(path) = keyfile {
        return SecretsKey::from_keyfile(config_dir, path);
    }
    if let Some(value) = inline_base64 {
        return SecretsKey::from_base64(value);
    }
    Err(KeyError::NotConfigured)
}

/// Reads [`ENV_KEY_FILE`]/[`ENV_KEY`] and delegates to [`acquire_key`]. The env is
/// read only here so the resolution logic stays pure and test-parallel-safe.
pub fn acquire_key_from_env(config_dir: &Path) -> Result<SecretsKey, KeyError> {
    let keyfile = std::env::var_os(ENV_KEY_FILE).map(PathBuf::from);
    let inline = std::env::var(ENV_KEY).ok();
    acquire_key(config_dir, keyfile.as_deref(), inline.as_deref())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd companion && cargo test -p server secrets::`
Expected: PASS — all key-acquisition tests plus the Task 2 tests.

- [ ] **Step 5: Commit**

```bash
git add companion/crates/server/src/secrets.rs
git commit -m "feat: acquire the secrets master key from a 0600 keyfile outside the config dir"
```

---

### Task 4: `IntegrationStore` and `IntegrationSecret`

The store: an in-memory `BTreeMap<String, IntegrationSecret>` behind a mutex, persisted as the sealed `secrets.enc` through `app-core`'s atomic writer. `open`/`get`/`put`/`remove`/`integration_ids`.

**Files:**
- Modify: `companion/crates/server/src/secrets.rs`
- Test: inline `#[cfg(test)] mod tests` in `secrets.rs`

**Interfaces:**
- Consumes: `seal`/`open` (Task 2), `SecretsKey` (Task 2), `SecretsError` (Task 2), `app_core::secure_file::{write_and_replace, read_bounded, create_directory, usable_parent, sync_parent}` (Task 1).
- Produces:
  - `pub const SECRETS_STORE_FILE: &str = "secrets.enc";`
  - `pub struct IntegrationSecret { pub provider: String, pub refresh_token: String, pub client_secret: Option<String>, pub scopes: Vec<String>, pub obtained_at: i64 }` (`Debug`, `Clone`, `PartialEq`, `Eq`, `Serialize`, `Deserialize`, `#[serde(deny_unknown_fields)]`)
  - `pub struct IntegrationStore` with:
    - `pub fn open(path: PathBuf, key: SecretsKey) -> Result<Self, SecretsError>`
    - `pub fn get(&self, integration_id: &str) -> Option<IntegrationSecret>`
    - `pub fn put(&self, integration_id: String, secret: IntegrationSecret) -> Result<(), SecretsError>`
    - `pub fn remove(&self, integration_id: &str) -> Result<bool, SecretsError>`
    - `pub fn integration_ids(&self) -> Vec<String>`

- [ ] **Step 1: Write the failing test**

Add these tests inside `mod tests` in `secrets.rs`:

```rust
    fn sample_secret() -> IntegrationSecret {
        IntegrationSecret {
            provider: "google".to_string(),
            refresh_token: "super-secret-refresh-token-value".to_string(),
            client_secret: Some("client-secret-value".to_string()),
            scopes: vec!["https://www.googleapis.com/auth/calendar.events.readonly".to_string()],
            obtained_at: 1_725_600_000,
        }
    }

    #[test]
    fn put_then_get_round_trips_across_reopen() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(SECRETS_STORE_FILE);

        let store = IntegrationStore::open(path.clone(), test_key(11)).expect("open empty");
        store.put("google-primary".to_string(), sample_secret()).expect("put");
        drop(store);

        let reopened = IntegrationStore::open(path, test_key(11)).expect("reopen");
        assert_eq!(reopened.get("google-primary"), Some(sample_secret()));
        assert_eq!(reopened.get("absent"), None);
    }

    #[test]
    fn remove_deletes_and_reports_presence() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(SECRETS_STORE_FILE);
        let store = IntegrationStore::open(path, test_key(12)).expect("open");

        store.put("id".to_string(), sample_secret()).expect("put");
        assert!(store.remove("id").expect("remove existing"));
        assert!(!store.remove("id").expect("remove absent"));
        assert_eq!(store.get("id"), None);
    }

    #[test]
    fn integration_ids_lists_keys_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(SECRETS_STORE_FILE);
        let store = IntegrationStore::open(path, test_key(13)).expect("open");

        store.put("b".to_string(), sample_secret()).expect("put b");
        store.put("a".to_string(), sample_secret()).expect("put a");
        assert_eq!(store.integration_ids(), vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn opening_with_the_wrong_key_fails_closed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(SECRETS_STORE_FILE);
        IntegrationStore::open(path.clone(), test_key(1))
            .expect("open")
            .put("id".to_string(), sample_secret())
            .expect("put");

        assert!(matches!(
            IntegrationStore::open(path, test_key(2)),
            Err(SecretsError::Decrypt)
        ));
    }

    #[test]
    fn persisted_file_contains_no_plaintext_secret() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(SECRETS_STORE_FILE);
        let store = IntegrationStore::open(path.clone(), test_key(14)).expect("open");
        store.put("id".to_string(), sample_secret()).expect("put");

        let on_disk = std::fs::read(&path).expect("read file");
        for needle in [
            b"super-secret-refresh-token-value".as_slice(),
            b"client-secret-value".as_slice(),
            b"google".as_slice(),
        ] {
            assert!(
                !on_disk.windows(needle.len()).any(|window| window == needle),
                "plaintext leaked to disk: {:?}",
                String::from_utf8_lossy(needle)
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn persisted_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(SECRETS_STORE_FILE);
        IntegrationStore::open(path.clone(), test_key(15))
            .expect("open")
            .put("id".to_string(), sample_secret())
            .expect("put");

        let mode = std::fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd companion && cargo test -p server secrets::`
Expected: FAIL — compile error, `cannot find type IntegrationStore` / `IntegrationSecret`.

- [ ] **Step 3: Write the implementation**

Add to `secrets.rs`. Extend the imports at the top with `use std::collections::BTreeMap;`, `use std::sync::Mutex;`, and `use serde::{Deserialize, Serialize};`. Then:

```rust
/// The at-rest secrets file, kept next to `device-identities.json` in the config
/// directory (spec §2). The key that decrypts it lives elsewhere (Task 3).
pub const SECRETS_STORE_FILE: &str = "secrets.enc";

/// One integration's stored credentials. `deny_unknown_fields` so a format drift
/// is a loud decode error, not a silent dropped field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegrationSecret {
    /// The identity provider, e.g. `"google"`.
    pub provider: String,
    /// The long-lived OAuth refresh token.
    pub refresh_token: String,
    /// The OAuth client secret, when the deployment stores it here rather than in a
    /// separate keyfile. `None` when supplied out-of-band.
    pub client_secret: Option<String>,
    /// The scopes granted at authorization time.
    pub scopes: Vec<String>,
    /// Unix seconds when these credentials were obtained.
    pub obtained_at: i64,
}

/// Encrypted-at-rest map of `integration_id -> IntegrationSecret`. Cheap to share
/// behind an `Arc`; every mutation persists the whole sealed file atomically.
pub struct IntegrationStore {
    path: PathBuf,
    key: SecretsKey,
    secrets: Mutex<BTreeMap<String, IntegrationSecret>>,
}

impl IntegrationStore {
    /// Loads and decrypts an existing `secrets.enc`, or starts empty if none exists.
    /// A present-but-undecryptable file is a hard error (fail closed) rather than a
    /// silent reset that would discard the owner's integrations.
    pub fn open(path: PathBuf, key: SecretsKey) -> Result<Self, SecretsError> {
        let secrets = match secure_file::read_bounded(&path, MAX_SECRETS_FILE_BYTES) {
            Ok(Some(file_bytes)) => {
                let plaintext = open(&key, &file_bytes)?;
                serde_json::from_slice(&plaintext)
                    .map_err(|error| SecretsError::Serialize(error.to_string()))?
            }
            Ok(None) => BTreeMap::new(),
            Err(secure_file::BoundedReadError::TooLarge { maximum }) => {
                return Err(SecretsError::TooLarge { maximum });
            }
            Err(secure_file::BoundedReadError::Io(error)) => {
                let (operation, detail) = error.into_strings("secrets file");
                return Err(SecretsError::Io { operation, detail });
            }
        };
        Ok(Self {
            path,
            key,
            secrets: Mutex::new(secrets),
        })
    }

    pub fn get(&self, integration_id: &str) -> Option<IntegrationSecret> {
        self.lock().get(integration_id).cloned()
    }

    pub fn put(
        &self,
        integration_id: String,
        secret: IntegrationSecret,
    ) -> Result<(), SecretsError> {
        let mut secrets = self.lock();
        secrets.insert(integration_id, secret);
        self.persist(&secrets)
    }

    /// Removes an integration's secret. Returns whether one was present.
    pub fn remove(&self, integration_id: &str) -> Result<bool, SecretsError> {
        let mut secrets = self.lock();
        let existed = secrets.remove(integration_id).is_some();
        if existed {
            self.persist(&secrets)?;
        }
        Ok(existed)
    }

    /// The integration ids present, sorted. Carries no secret values (spec §6:
    /// presence, never token values).
    pub fn integration_ids(&self) -> Vec<String> {
        self.lock().keys().cloned().collect()
    }

    fn persist(&self, secrets: &BTreeMap<String, IntegrationSecret>) -> Result<(), SecretsError> {
        let plaintext =
            serde_json::to_vec(secrets).map_err(|error| SecretsError::Serialize(error.to_string()))?;
        let sealed = seal(&self.key, &plaintext)?;

        if let Some(parent) = secure_file::usable_parent(&self.path) {
            secure_file::create_directory(parent).map_err(|error| {
                let (operation, detail) = error.into_strings("secrets directory");
                SecretsError::Io { operation, detail }
            })?;
        }
        secure_file::write_and_replace(&self.path, &sealed).map_err(|error| {
            let (operation, detail) = error.into_strings("secrets file");
            SecretsError::Io { operation, detail }
        })?;
        if let Some(parent) = secure_file::usable_parent(&self.path) {
            secure_file::sync_parent(parent).map_err(|error| {
                let (operation, detail) = error.into_strings("secrets directory");
                SecretsError::Io { operation, detail }
            })?;
        }
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, IntegrationSecret>> {
        self.secrets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
```

Add the `secure_file` import at the top of the module: `use app_core::secure_file;`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd companion && cargo test -p server secrets::`
Expected: PASS — all store tests, including the no-plaintext-on-disk and `0o600` assertions.

- [ ] **Step 5: Commit**

```bash
git add companion/crates/server/src/secrets.rs
git commit -m "feat: add IntegrationStore for encrypted per-integration OAuth secrets"
```

---

### Task 5: Fail-closed startup helper

One helper that a future `main.rs` calls: acquire the key from the environment, then open the store — refusing to start when a `secrets.enc` exists but no key is configured. Env reading is isolated; the fail-closed decision is tested through an injected key result.

**Files:**
- Modify: `companion/crates/server/src/secrets.rs`
- Test: inline `#[cfg(test)] mod tests` in `secrets.rs`

**Interfaces:**
- Consumes: `acquire_key_from_env` (Task 3), `KeyError` (Task 3), `IntegrationStore::open` (Task 4), `SecretsError` (Task 4), `SECRETS_STORE_FILE` (Task 4), `SecretsKey` (Task 2).
- Produces:
  - `pub enum StartupError { KeyRequired, KeyRequiredForExistingSecrets, Key(KeyError), Store(SecretsError) }` (`Debug`, `thiserror::Error`)
  - `pub fn open_integration_store_with(config_dir: &Path, key: Result<SecretsKey, KeyError>) -> Result<IntegrationStore, StartupError>` — pure over the injected key result.
  - `pub fn open_integration_store(config_dir: &Path) -> Result<IntegrationStore, StartupError>` — env wrapper.

- [ ] **Step 1: Write the failing test**

Add these tests inside `mod tests` in `secrets.rs`:

```rust
    #[test]
    fn startup_fails_closed_when_key_missing_and_secrets_exist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(SECRETS_STORE_FILE);
        // Materialize a real sealed secrets file first.
        IntegrationStore::open(path, test_key(21))
            .expect("open")
            .put("id".to_string(), sample_secret())
            .expect("put");

        let result =
            open_integration_store_with(dir.path(), Err(KeyError::NotConfigured));
        assert!(matches!(
            result,
            Err(StartupError::KeyRequiredForExistingSecrets)
        ));
    }

    #[test]
    fn startup_requires_a_key_even_with_no_secrets_yet() {
        let dir = tempfile::tempdir().expect("tempdir");
        let result =
            open_integration_store_with(dir.path(), Err(KeyError::NotConfigured));
        assert!(matches!(result, Err(StartupError::KeyRequired)));
    }

    #[test]
    fn startup_propagates_a_concrete_key_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let result = open_integration_store_with(
            dir.path(),
            Err(KeyError::WrongLength { got: 10 }),
        );
        assert!(matches!(
            result,
            Err(StartupError::Key(KeyError::WrongLength { got: 10 }))
        ));
    }

    #[test]
    fn startup_opens_the_store_with_a_valid_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = open_integration_store_with(dir.path(), Ok(test_key(22)))
            .expect("store opens");
        assert!(store.integration_ids().is_empty());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd companion && cargo test -p server secrets::`
Expected: FAIL — compile error, `cannot find function open_integration_store_with` / `no variant StartupError`.

- [ ] **Step 3: Write the implementation**

Add to `secrets.rs` (above the test module):

```rust
#[derive(Debug, thiserror::Error)]
pub enum StartupError {
    #[error(
        "a secrets key is required: set {ENV_KEY_FILE} (preferred) or {ENV_KEY} before starting"
    )]
    KeyRequired,
    #[error(
        "{SECRETS_STORE_FILE} exists but no secrets key is configured; refusing to start. \
         Set {ENV_KEY_FILE} to the keyfile that decrypts it (spec fail-closed rule)"
    )]
    KeyRequiredForExistingSecrets,
    #[error("secrets key error: {0}")]
    Key(#[source] KeyError),
    #[error("secrets store error: {0}")]
    Store(#[source] SecretsError),
}

/// Opens the integration store from an injected key result, applying the
/// fail-closed startup rule (spec §1.1). A missing key when a `secrets.enc`
/// already exists is the emphatic refusal; a missing key with no file yet is
/// still an error, because integrations cannot function without one and a
/// "works until you connect an account" failure is worse.
pub fn open_integration_store_with(
    config_dir: &Path,
    key: Result<SecretsKey, KeyError>,
) -> Result<IntegrationStore, StartupError> {
    let secrets_path = config_dir.join(SECRETS_STORE_FILE);
    let key = match key {
        Ok(key) => key,
        Err(KeyError::NotConfigured) => {
            return Err(if secrets_path.exists() {
                StartupError::KeyRequiredForExistingSecrets
            } else {
                StartupError::KeyRequired
            });
        }
        Err(other) => return Err(StartupError::Key(other)),
    };
    IntegrationStore::open(secrets_path, key).map_err(StartupError::Store)
}

/// Production entry point: reads the key from the environment (Task 3), then
/// applies [`open_integration_store_with`].
pub fn open_integration_store(config_dir: &Path) -> Result<IntegrationStore, StartupError> {
    open_integration_store_with(config_dir, acquire_key_from_env(config_dir))
}
```

- [ ] **Step 4: Run the full workspace gates**

Run:
```bash
cd companion
cargo test -p server secrets::
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo test --workspace --doc
```
Expected: PASS across the board. (The `--workspace` runs are the controller's, per the project-memory note that sandboxed subagents cannot bind loopback for server integration tests.)

- [ ] **Step 5: Commit**

```bash
git add companion/crates/server/src/secrets.rs
git commit -m "feat: fail-closed startup helper for the integration secrets store"
```

---

## Self-Review

**1. Spec coverage.**

- §1.1 "JSON configs + one encrypted secrets blob, key from keyfile/env" → Tasks 2 (envelope), 3 (keyfile+env), 4 (the `secrets.enc` blob beside `device-identities.json`, configs untouched). SQLite explicitly not introduced. ✓
- §1.1 "vetted AEAD, not hand-rolled" → `chacha20poly1305` XChaCha20-Poly1305, Task 2, pinned `=0.10.1`. (Spec named `age` as first choice; this plan takes the spec's own named fallback because `age` has no raw-32-byte-key path without a scrypt/passphrase or bech32 X25519 detour — recorded here so the deviation is deliberate, not silent.) ✓
- §1.1 "key from `DESKMATE_SECRETS_KEY_FILE` (preferred) / `DESKMATE_SECRETS_KEY` (fallback)" → Task 3 `ENV_KEY_FILE`/`ENV_KEY`, keyfile precedence tested. ✓
- §1.1 "keyfile `0o600` … outside config dir" → Task 3 `from_keyfile` mode check + canonical containment check, both tested. ✓
- §1.1 "refuse to start if secrets file exists but no key" (fail closed) → Task 5 `StartupError::KeyRequiredForExistingSecrets`, tested. ✓
- §1.1 threat model "at-rest only; live-server compromise reads tokens" → stated in Global Constraints; no code overclaims. ✓
- §1.1 "key lives outside the config dir so a config-dir backup does not carry it" → Task 3 containment rejection. ✓
- §6 "encryption at rest; only in memory; access token never persisted" → this sub-project stores only long-lived secrets encrypted; the in-memory access-token cache is sub-project 2, correctly absent here. ✓
- §6 "no token readback — presence not values" → Task 4 `integration_ids()` returns keys only; `get` is server-internal, exposed to no route in this sub-project. ✓
- §9.1 automated coverage: round-trip put/get (Task 4), wrong-key fails closed (Tasks 2 & 4), missing-key fails closed at startup (Task 5), tamper/truncation rejected (Task 2), file mode `0o600` (Tasks 1 & 4), no-plaintext-on-disk (Task 4). ✓
- §8 sub-project 1 boundary "no OAuth, no HTTP, no provider" → none added; no route touched. ✓
- Out of scope confirmed absent: no `main.rs` wiring (the helper is provided but not yet called — that is sub-project 2/4's integration point), no admin route, no multi-tenant `owner_id` keying (§7 defers it; the flat `integration_id` map is single-owner by design). ✓

**2. Placeholder scan.** No "TBD"/"TODO"/"add error handling"/"similar to Task N". Every code step carries the real body; every "body unchanged" in Task 1 refers to lines already present in `secure_file.rs` (a visibility-only edit, not omitted content). Error handling is concrete (typed `KeyError`/`SecretsError`/`StartupError` with exhaustive `match`). ✓

**3. Type consistency.**

- `SecretsKey::from_bytes` (Task 2) / `from_base64`, `from_keyfile` (Task 3) — name and signatures consistent across `acquire_key`, tests, and `IntegrationStore::open`. ✓
- `seal(&SecretsKey, &[u8]) -> Result<Vec<u8>, SecretsError>` and `open(&SecretsKey, &[u8]) -> Result<Vec<u8>, SecretsError>` — used identically in Task 4's `persist`/`open`. ✓
- `SecretsError` variants (`Io{operation,detail}`, `TooLarge{maximum}`, `BadFormat`, `Decrypt`, `Encrypt`, `Serialize`) — every construction site (Task 2 seal/open, Task 4 persist/open) uses a defined variant; `TooLarge{maximum}` and `Io{operation,detail}` are produced from `secure_file::BoundedReadError` in Task 4 exactly as declared. ✓
- `app_core::secure_file::{write_and_replace, read_bounded, create_directory, usable_parent, sync_parent, BoundedReadError, FileIoError}` — every item Task 4 calls is promoted to `pub` in Task 1. `FileIoError::into_strings(self, subject)` returns `(String, String)`, consumed as `(operation, detail)` in Task 4. ✓
- `SECRETS_STORE_FILE` (Task 4) referenced by Task 5's helper and tests — same constant. ✓
- `ENV_KEY_FILE`/`ENV_KEY` (Task 3) referenced in Task 5's `StartupError` messages — same constants. ✓
- `IntegrationSecret` field set is identical between the struct definition (Task 4) and every `sample_secret()`/`deny_unknown_fields` round-trip. ✓

No gaps found. Plan is internally consistent and covers §1.1, §6, §8, and §9.1 for sub-project 1.
