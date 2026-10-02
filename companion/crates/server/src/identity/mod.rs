mod email;

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use app_core::secure_file;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction};
use thiserror::Error;

use crate::credential::{random_token, token_digest};

pub(crate) use email::normalize_email;

pub(crate) const SESSION_TTL_DAYS: i64 = 30;
pub(crate) const LOGIN_TOKEN_TTL_MINUTES: i64 = 15;

const SECONDS_PER_DAY: i64 = 86_400;
const SECONDS_PER_MINUTE: i64 = 60;
const SIGNUPS_OPEN_KEY: &str = "signups_open";

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS accounts (
  id TEXT PRIMARY KEY, email TEXT NOT NULL UNIQUE, email_verified INTEGER NOT NULL,
  created_at INTEGER NOT NULL, is_instance_owner INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS sessions (
  id_sha256 BLOB PRIMARY KEY, account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL, last_seen_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS login_tokens (
  token_sha256 BLOB PRIMARY KEY, email TEXT NOT NULL, expires_at INTEGER NOT NULL, used_at INTEGER);
CREATE TABLE IF NOT EXISTS google_identities (
  google_sub TEXT PRIMARY KEY, account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE);
CREATE TABLE IF NOT EXISTS device_owners (
  device_id TEXT PRIMARY KEY, account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  state TEXT NOT NULL CHECK (state IN ('pending','active')), claimed_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS instance (key TEXT PRIMARY KEY, value TEXT NOT NULL);
";

#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AccountId(pub String);

impl fmt::Display for AccountId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[doc(hidden)]
pub struct Account {
    pub id: AccountId,
    pub email: String,
    pub email_verified: bool,
    pub created_at: DateTime<Utc>,
    pub is_instance_owner: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[doc(hidden)]
pub enum DeviceState {
    Pending,
    Active,
}

impl DeviceState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
        }
    }

    fn from_str(value: &str) -> Result<Self, IdentityError> {
        match value {
            "pending" => Ok(Self::Pending),
            "active" => Ok(Self::Active),
            _ => Err(IdentityError::Sqlite(format!(
                "invalid device ownership state {value:?}"
            ))),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[doc(hidden)]
pub struct DeviceOwner {
    pub device_id: String,
    pub account_id: AccountId,
    pub state: DeviceState,
    pub claimed_at: DateTime<Utc>,
}

#[derive(Debug, Error, PartialEq, Eq)]
#[doc(hidden)]
pub enum IdentityError {
    #[error("identity database error: {0}")]
    Sqlite(String),
    #[error("an account already uses that email")]
    EmailTaken,
    #[error("invalid email address")]
    InvalidEmail,
    #[error("identity record not found")]
    NotFound,
}

#[doc(hidden)]
pub struct IdentityStore {
    connection: Mutex<Connection>,
}

impl IdentityStore {
    pub fn open(path: &Path) -> Result<Self, IdentityError> {
        let parent = secure_file::usable_parent(path).ok_or_else(|| {
            IdentityError::Sqlite(format!(
                "identity database path has no parent: {}",
                path.display()
            ))
        })?;
        secure_file::create_directory(parent).map_err(|error| {
            let (operation, source) = error.into_strings("identity database directory");
            IdentityError::Sqlite(format!("{operation}: {source}"))
        })?;

        let mut connection = Connection::open(path).map_err(sqlite_error)?;
        secure_database_files(path)?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(sqlite_error)?;
        connection
            .pragma_update(None, "foreign_keys", true)
            .map_err(sqlite_error)?;
        {
            let transaction = connection.transaction().map_err(sqlite_error)?;
            transaction.execute_batch(SCHEMA).map_err(sqlite_error)?;
            transaction.commit().map_err(sqlite_error)?;
        }
        secure_database_files(path)?;

        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn account_count(&self) -> Result<u64, IdentityError> {
        let connection = self.lock()?;
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get(0))
            .map_err(sqlite_error)?;
        u64::try_from(count)
            .map_err(|_| IdentityError::Sqlite(format!("invalid account count {count}")))
    }

    pub fn create_account(
        &self,
        email: &str,
        verified: bool,
        instance_owner: bool,
        now: DateTime<Utc>,
    ) -> Result<Account, IdentityError> {
        let email = normalize_email(email).ok_or(IdentityError::InvalidEmail)?;
        let account = Account {
            id: AccountId(format!("acc_{}", &random_token()[..32])),
            email,
            email_verified: verified,
            created_at: now,
            is_instance_owner: instance_owner,
        };
        let connection = self.lock()?;
        match connection.execute(
            "INSERT INTO accounts (id, email, email_verified, created_at, is_instance_owner) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            (
                &account.id.0,
                &account.email,
                account.email_verified,
                account.created_at.timestamp(),
                account.is_instance_owner,
            ),
        ) {
            Ok(_) => Ok(account),
            Err(error) if is_unique_violation(&error) => Err(IdentityError::EmailTaken),
            Err(error) => Err(sqlite_error(error)),
        }
    }

    pub fn account(&self, id: &AccountId) -> Result<Option<Account>, IdentityError> {
        let connection = self.lock()?;
        select_account(&connection, "accounts.id = ?1", &id.0)
    }

    pub fn account_by_email(&self, email: &str) -> Result<Option<Account>, IdentityError> {
        let email = normalize_email(email).ok_or(IdentityError::InvalidEmail)?;
        let connection = self.lock()?;
        select_account(&connection, "accounts.email = ?1", &email)
    }

    pub fn accounts(&self) -> Result<Vec<Account>, IdentityError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT id, email, email_verified, created_at, is_instance_owner \
                 FROM accounts ORDER BY created_at, id",
            )
            .map_err(sqlite_error)?;
        let rows = statement
            .query_map([], account_from_row)
            .map_err(sqlite_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(sqlite_error)
    }

    pub fn mark_email_verified(&self, id: &AccountId) -> Result<(), IdentityError> {
        let connection = self.lock()?;
        require_changed(
            connection
                .execute(
                    "UPDATE accounts SET email_verified = 1 WHERE id = ?1",
                    [&id.0],
                )
                .map_err(sqlite_error)?,
        )
    }

    pub fn delete_account(&self, id: &AccountId) -> Result<Vec<String>, IdentityError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(sqlite_error)?;
        let device_ids = {
            let mut statement = transaction
                .prepare(
                    "SELECT device_id FROM device_owners \
                     WHERE account_id = ?1 ORDER BY device_id",
                )
                .map_err(sqlite_error)?;
            let rows = statement
                .query_map([&id.0], |row| row.get(0))
                .map_err(sqlite_error)?;
            rows.collect::<Result<Vec<String>, _>>()
                .map_err(sqlite_error)?
        };
        let changed = transaction
            .execute("DELETE FROM accounts WHERE id = ?1", [&id.0])
            .map_err(sqlite_error)?;
        require_changed(changed)?;
        transaction.commit().map_err(sqlite_error)?;
        Ok(device_ids)
    }

    pub fn create_session(
        &self,
        account: &AccountId,
        now: DateTime<Utc>,
    ) -> Result<String, IdentityError> {
        let plaintext = random_token();
        let digest = token_digest(&plaintext);
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO sessions \
                 (id_sha256, account_id, created_at, expires_at, last_seen_at) \
                 VALUES (?1, ?2, ?3, ?4, ?3)",
                (
                    digest.as_slice(),
                    &account.0,
                    now.timestamp(),
                    now.timestamp() + SESSION_TTL_DAYS * SECONDS_PER_DAY,
                ),
            )
            .map_err(reference_error)?;
        Ok(plaintext)
    }

    pub fn session_account(
        &self,
        plaintext: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<Account>, IdentityError> {
        let digest = token_digest(plaintext);
        let now_timestamp = now.timestamp();
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(sqlite_error)?;
        remove_expired_credentials(&transaction, now_timestamp)?;

        let found = transaction
            .query_row(
                "SELECT accounts.id, accounts.email, accounts.email_verified, \
                        accounts.created_at, accounts.is_instance_owner, sessions.last_seen_at \
                 FROM sessions JOIN accounts ON accounts.id = sessions.account_id \
                 WHERE sessions.id_sha256 = ?1 AND sessions.expires_at > ?2",
                (digest.as_slice(), now_timestamp),
                |row| Ok((account_from_row(row)?, row.get::<_, i64>(5)?)),
            )
            .optional()
            .map_err(sqlite_error)?;
        let Some((account, last_seen_at)) = found else {
            transaction.commit().map_err(sqlite_error)?;
            return Ok(None);
        };

        if now_timestamp - last_seen_at >= SECONDS_PER_DAY {
            transaction
                .execute(
                    "UPDATE sessions SET last_seen_at = ?1, expires_at = ?2 \
                     WHERE id_sha256 = ?3",
                    (
                        now_timestamp,
                        now_timestamp + SESSION_TTL_DAYS * SECONDS_PER_DAY,
                        digest.as_slice(),
                    ),
                )
                .map_err(sqlite_error)?;
        }
        transaction.commit().map_err(sqlite_error)?;
        Ok(Some(account))
    }

    pub fn delete_session(&self, plaintext: &str) -> Result<(), IdentityError> {
        let digest = token_digest(plaintext);
        let connection = self.lock()?;
        connection
            .execute(
                "DELETE FROM sessions WHERE id_sha256 = ?1",
                [digest.as_slice()],
            )
            .map_err(sqlite_error)?;
        Ok(())
    }

    pub fn delete_sessions_for(&self, account: &AccountId) -> Result<(), IdentityError> {
        let connection = self.lock()?;
        connection
            .execute("DELETE FROM sessions WHERE account_id = ?1", [&account.0])
            .map_err(sqlite_error)?;
        Ok(())
    }

    pub fn create_login_token(
        &self,
        email: &str,
        now: DateTime<Utc>,
    ) -> Result<String, IdentityError> {
        let email = normalize_email(email).ok_or(IdentityError::InvalidEmail)?;
        let plaintext = random_token();
        let digest = token_digest(&plaintext);
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO login_tokens (token_sha256, email, expires_at, used_at) \
                 VALUES (?1, ?2, ?3, NULL)",
                (
                    digest.as_slice(),
                    email,
                    now.timestamp() + LOGIN_TOKEN_TTL_MINUTES * SECONDS_PER_MINUTE,
                ),
            )
            .map_err(sqlite_error)?;
        Ok(plaintext)
    }

    pub fn consume_login_token(
        &self,
        plaintext: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<String>, IdentityError> {
        let digest = token_digest(plaintext);
        let now_timestamp = now.timestamp();
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(sqlite_error)?;
        remove_expired_credentials(&transaction, now_timestamp)?;
        let email = transaction
            .query_row(
                "UPDATE login_tokens SET used_at = ?1 \
                 WHERE token_sha256 = ?2 AND used_at IS NULL AND expires_at > ?1 \
                 RETURNING email",
                (now_timestamp, digest.as_slice()),
                |row| row.get(0),
            )
            .optional()
            .map_err(sqlite_error)?;
        transaction.commit().map_err(sqlite_error)?;
        Ok(email)
    }

    pub fn account_for_google(&self, sub: &str) -> Result<Option<Account>, IdentityError> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT accounts.id, accounts.email, accounts.email_verified, \
                        accounts.created_at, accounts.is_instance_owner \
                 FROM google_identities \
                 JOIN accounts ON accounts.id = google_identities.account_id \
                 WHERE google_identities.google_sub = ?1",
                [sub],
                account_from_row,
            )
            .optional()
            .map_err(sqlite_error)
    }

    pub fn link_google(&self, sub: &str, account: &AccountId) -> Result<(), IdentityError> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO google_identities (google_sub, account_id) VALUES (?1, ?2)",
                (sub, &account.0),
            )
            .map_err(reference_error)?;
        Ok(())
    }

    pub fn assign_device(
        &self,
        device_id: &str,
        account: &AccountId,
        state: DeviceState,
        now: DateTime<Utc>,
    ) -> Result<(), IdentityError> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO device_owners (device_id, account_id, state, claimed_at) \
                 VALUES (?1, ?2, ?3, ?4)",
                (device_id, &account.0, state.as_str(), now.timestamp()),
            )
            .map_err(reference_error)?;
        Ok(())
    }

    pub fn device_owner(&self, device_id: &str) -> Result<Option<DeviceOwner>, IdentityError> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT device_id, account_id, state, claimed_at \
                 FROM device_owners WHERE device_id = ?1",
                [device_id],
                device_owner_row,
            )
            .optional()
            .map_err(sqlite_error)?
            .map(device_owner_from_values)
            .transpose()
    }

    pub fn devices_for(&self, account: &AccountId) -> Result<Vec<DeviceOwner>, IdentityError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT device_id, account_id, state, claimed_at \
                 FROM device_owners WHERE account_id = ?1 ORDER BY device_id",
            )
            .map_err(sqlite_error)?;
        let rows = statement
            .query_map([&account.0], device_owner_row)
            .map_err(sqlite_error)?;
        rows.map(|row| row.map_err(sqlite_error).and_then(device_owner_from_values))
            .collect()
    }

    pub fn activate_device(&self, device_id: &str) -> Result<(), IdentityError> {
        let connection = self.lock()?;
        require_changed(
            connection
                .execute(
                    "UPDATE device_owners SET state = 'active' WHERE device_id = ?1",
                    [device_id],
                )
                .map_err(sqlite_error)?,
        )
    }

    pub fn release_device(&self, device_id: &str) -> Result<(), IdentityError> {
        let connection = self.lock()?;
        connection
            .execute(
                "DELETE FROM device_owners WHERE device_id = ?1",
                [device_id],
            )
            .map_err(sqlite_error)?;
        Ok(())
    }

    pub fn stale_pending(
        &self,
        claimed_before: DateTime<Utc>,
    ) -> Result<Vec<String>, IdentityError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT device_id FROM device_owners \
                 WHERE state = 'pending' AND claimed_at < ?1 ORDER BY device_id",
            )
            .map_err(sqlite_error)?;
        let rows = statement
            .query_map([claimed_before.timestamp()], |row| row.get(0))
            .map_err(sqlite_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(sqlite_error)
    }

    pub fn signups_open(&self) -> Result<Option<bool>, IdentityError> {
        let connection = self.lock()?;
        let value: Option<String> = connection
            .query_row(
                "SELECT value FROM instance WHERE key = ?1",
                [SIGNUPS_OPEN_KEY],
                |row| row.get(0),
            )
            .optional()
            .map_err(sqlite_error)?;
        value
            .map(|value| match value.as_str() {
                "true" => Ok(true),
                "false" => Ok(false),
                _ => Err(IdentityError::Sqlite(format!(
                    "invalid signups_open value {value:?}"
                ))),
            })
            .transpose()
    }

    pub fn set_signups_open(&self, open: bool) -> Result<(), IdentityError> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO instance (key, value) VALUES (?1, ?2) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                (SIGNUPS_OPEN_KEY, if open { "true" } else { "false" }),
            )
            .map_err(sqlite_error)?;
        Ok(())
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, IdentityError> {
        self.connection
            .lock()
            .map_err(|_| IdentityError::Sqlite("identity database lock poisoned".to_string()))
    }
}

fn select_account(
    connection: &Connection,
    condition: &str,
    value: &str,
) -> Result<Option<Account>, IdentityError> {
    let sql = format!(
        "SELECT id, email, email_verified, created_at, is_instance_owner \
         FROM accounts WHERE {condition}"
    );
    connection
        .query_row(&sql, [value], account_from_row)
        .optional()
        .map_err(sqlite_error)
}

fn account_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    let created_at = row.get::<_, i64>(3)?;
    Ok(Account {
        id: AccountId(row.get(0)?),
        email: row.get(1)?,
        email_verified: row.get(2)?,
        created_at: timestamp(created_at, 3)?,
        is_instance_owner: row.get(4)?,
    })
}

fn device_owner_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(String, String, String, i64)> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
}

fn device_owner_from_values(
    (device_id, account_id, state, claimed_at): (String, String, String, i64),
) -> Result<DeviceOwner, IdentityError> {
    Ok(DeviceOwner {
        device_id,
        account_id: AccountId(account_id),
        state: DeviceState::from_str(&state)?,
        claimed_at: DateTime::from_timestamp(claimed_at, 0).ok_or_else(|| {
            IdentityError::Sqlite(format!("invalid device claimed_at timestamp {claimed_at}"))
        })?,
    })
}

fn timestamp(value: i64, column: usize) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::from_timestamp(value, 0)
        .ok_or(rusqlite::Error::IntegralValueOutOfRange(column, value))
}

fn remove_expired_credentials(
    transaction: &Transaction<'_>,
    now: i64,
) -> Result<(), IdentityError> {
    transaction
        .execute("DELETE FROM sessions WHERE expires_at <= ?1", [now])
        .map_err(sqlite_error)?;
    transaction
        .execute("DELETE FROM login_tokens WHERE expires_at <= ?1", [now])
        .map_err(sqlite_error)?;
    Ok(())
}

fn require_changed(changed: usize) -> Result<(), IdentityError> {
    if changed == 0 {
        Err(IdentityError::NotFound)
    } else {
        Ok(())
    }
}

fn reference_error(error: rusqlite::Error) -> IdentityError {
    if is_foreign_key_violation(&error) {
        IdentityError::NotFound
    } else {
        sqlite_error(error)
    }
}

fn is_unique_violation(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(code, _)
            if code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
    )
}

fn is_foreign_key_violation(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(code, _)
            if code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY
    )
}

// `map_err` passes its error by value; keeping this adapter avoids a closure at
// every SQLite call site even though formatting only borrows the value.
#[allow(clippy::needless_pass_by_value)]
fn sqlite_error(error: rusqlite::Error) -> IdentityError {
    IdentityError::Sqlite(error.to_string())
}

#[cfg(unix)]
fn secure_database_files(path: &Path) -> Result<(), IdentityError> {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    for candidate in [
        path.to_path_buf(),
        sidecar_path(path, "-wal"),
        sidecar_path(path, "-shm"),
    ] {
        match fs::metadata(&candidate) {
            Ok(_) => {
                fs::set_permissions(&candidate, fs::Permissions::from_mode(0o600)).map_err(
                    |error| {
                        IdentityError::Sqlite(format!(
                            "set permissions on {}: {error}",
                            candidate.display()
                        ))
                    },
                )?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(IdentityError::Sqlite(format!(
                    "inspect permissions on {}: {error}",
                    candidate.display()
                )));
            }
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn secure_database_files(_path: &Path) -> Result<(), IdentityError> {
    Ok(())
}

fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};

    use super::{DeviceState, IdentityError, IdentityStore};

    fn store() -> (tempfile::TempDir, IdentityStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = IdentityStore::open(&dir.path().join("identity.db")).unwrap();
        (dir, store)
    }

    fn t(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000 + secs, 0).unwrap()
    }

    #[test]
    fn login_token_is_single_use_and_expires() {
        let (_d, s) = store();
        let token = s.create_login_token("a@example.com", t(0)).unwrap();
        assert_eq!(
            s.consume_login_token(&token, t(60)).unwrap().as_deref(),
            Some("a@example.com")
        );
        assert_eq!(
            s.consume_login_token(&token, t(61)).unwrap(),
            None,
            "second use"
        );
        let late = s.create_login_token("a@example.com", t(0)).unwrap();
        assert_eq!(
            s.consume_login_token(&late, t(15 * 60)).unwrap(),
            None,
            "expired at 15 min"
        );
    }

    #[test]
    fn an_older_unused_link_still_works_after_a_newer_one() {
        let (_d, s) = store();
        let first = s.create_login_token("a@example.com", t(0)).unwrap();
        let _second = s.create_login_token("a@example.com", t(10)).unwrap();
        assert!(s.consume_login_token(&first, t(20)).unwrap().is_some());
    }

    #[test]
    fn session_slides_at_most_once_a_day_and_expires() {
        let (_d, s) = store();
        let a = s.create_account("a@example.com", true, true, t(0)).unwrap();
        let sid = s.create_session(&a.id, t(0)).unwrap();
        let day = 86_400;
        assert!(s.session_account(&sid, t(29 * day)).unwrap().is_some());
        assert!(s.session_account(&sid, t(58 * day)).unwrap().is_some());
        let idle = s.create_session(&a.id, t(0)).unwrap();
        assert!(
            s.session_account(&idle, t(30 * day)).unwrap().is_none(),
            "unused session expires"
        );
    }

    #[test]
    fn delete_sessions_for_signs_out_everywhere() {
        let (_d, s) = store();
        let a = s.create_account("a@example.com", true, true, t(0)).unwrap();
        let one = s.create_session(&a.id, t(0)).unwrap();
        let two = s.create_session(&a.id, t(0)).unwrap();
        s.delete_sessions_for(&a.id).unwrap();
        assert!(s.session_account(&one, t(1)).unwrap().is_none());
        assert!(s.session_account(&two, t(1)).unwrap().is_none());
    }

    #[test]
    fn database_file_never_contains_a_plaintext_secret() {
        let (dir, s) = store();
        let a = s.create_account("a@example.com", true, true, t(0)).unwrap();
        let sid = s.create_session(&a.id, t(0)).unwrap();
        let login = s.create_login_token("a@example.com", t(0)).unwrap();
        drop(s);
        let mut bytes = Vec::new();
        for entry in std::fs::read_dir(dir.path()).unwrap() {
            bytes.extend(std::fs::read(entry.unwrap().path()).unwrap());
        }
        let hay = String::from_utf8_lossy(&bytes);
        assert!(!hay.contains(&sid), "session id in plaintext");
        assert!(!hay.contains(&login), "login token in plaintext");
    }

    #[test]
    fn email_is_unique_after_normalisation() {
        let (_d, s) = store();
        s.create_account("a@example.com", true, true, t(0)).unwrap();
        assert!(matches!(
            s.create_account("  A@Example.COM ", false, false, t(1)),
            Err(IdentityError::EmailTaken)
        ));
        assert!(s.account_by_email("A@EXAMPLE.com").unwrap().is_some());
    }

    #[test]
    fn deleting_an_account_releases_its_devices_and_sessions() {
        let (_d, s) = store();
        let a = s.create_account("a@example.com", true, true, t(0)).unwrap();
        s.assign_device("dev-0001", &a.id, DeviceState::Active, t(0))
            .unwrap();
        let sid = s.create_session(&a.id, t(0)).unwrap();
        assert_eq!(
            s.delete_account(&a.id).unwrap(),
            vec!["dev-0001".to_string()]
        );
        assert!(s.device_owner("dev-0001").unwrap().is_none());
        assert!(s.session_account(&sid, t(1)).unwrap().is_none());
    }

    #[test]
    fn stale_pending_lists_only_pending_claims_older_than_the_cutoff() {
        let (_d, s) = store();
        let a = s.create_account("a@example.com", true, true, t(0)).unwrap();
        s.assign_device("dev-0001", &a.id, DeviceState::Pending, t(0))
            .unwrap();
        s.assign_device("dev-0002", &a.id, DeviceState::Pending, t(100))
            .unwrap();
        s.assign_device("dev-0003", &a.id, DeviceState::Active, t(0))
            .unwrap();
        assert_eq!(
            s.stale_pending(t(50)).unwrap(),
            vec!["dev-0001".to_string()]
        );
    }

    #[test]
    fn account_google_device_and_instance_crud_round_trip() {
        let (_d, s) = store();
        assert_eq!(s.account_count().unwrap(), 0);
        assert_eq!(s.signups_open().unwrap(), None);

        let a = s
            .create_account("a@example.com", false, true, t(0))
            .unwrap();
        assert_eq!(s.account_count().unwrap(), 1);
        assert_eq!(s.account(&a.id).unwrap(), Some(a.clone()));
        assert_eq!(s.accounts().unwrap(), vec![a.clone()]);

        s.mark_email_verified(&a.id).unwrap();
        assert!(s.account(&a.id).unwrap().unwrap().email_verified);
        s.link_google("google-sub", &a.id).unwrap();
        assert_eq!(
            s.account_for_google("google-sub").unwrap().unwrap().id,
            a.id
        );

        s.assign_device("dev-0001", &a.id, DeviceState::Pending, t(1))
            .unwrap();
        assert_eq!(s.devices_for(&a.id).unwrap().len(), 1);
        s.activate_device("dev-0001").unwrap();
        assert_eq!(
            s.device_owner("dev-0001").unwrap().unwrap().state,
            DeviceState::Active
        );
        s.release_device("dev-0001").unwrap();
        assert!(s.devices_for(&a.id).unwrap().is_empty());

        let session = s.create_session(&a.id, t(2)).unwrap();
        s.delete_session(&session).unwrap();
        assert!(s.session_account(&session, t(3)).unwrap().is_none());

        s.set_signups_open(true).unwrap();
        assert_eq!(s.signups_open().unwrap(), Some(true));
        s.set_signups_open(false).unwrap();
        assert_eq!(s.signups_open().unwrap(), Some(false));
    }

    #[cfg(unix)]
    #[test]
    fn database_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let (dir, _s) = store();
        for entry in std::fs::read_dir(dir.path()).unwrap() {
            let entry = entry.unwrap();
            assert_eq!(
                entry.metadata().unwrap().permissions().mode() & 0o777,
                0o600,
                "{}",
                entry.path().display()
            );
        }
    }
}
