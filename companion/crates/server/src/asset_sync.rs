//! Server-side asset transfer driver over the `RuntimeDevice` seam.
//!
//! Unlike `RuntimeDevice::provision`/`factory_reset` -- which return a typed
//! unsupported-on-this-transport error because provisioning is a cable
//! operation by design -- asset transfer must work on *both* transports. The
//! server owning the device over the tunnel is the entire point of networked
//! tier, so [`AssetSync::reconcile`] drives the same `send_asset_*` methods
//! regardless of whether `device` is backed by USB or the WebSocket link.
//!
//! Content addressing is the whole inventory protocol: there is no separate
//! "what do you have?" query. `reconcile` sends `AssetBegin` for every
//! desired asset; when the device's `Ack` reports `already_present`, no
//! chunks are sent for that digest at all. Because `AssetBegin` always opens
//! a fresh reservation -- the device aborts any transfer already in flight
//! for that digest first (protocol v1 §4/Task 9) -- there is no wire-level
//! way to resume a partially-uploaded asset from a remembered byte offset.
//! A dropped connection, or a retry after a local failure, restarts that
//! asset's chunks from zero; only whole committed assets are ever skipped.

use std::fs;
use std::sync::Arc;

use app_core::{AssetKind as ConfigAssetKind, AssetSettings, AssetSource, RuntimeDevice};
use device::DeviceError;
use protocol::{
    ASSET_DIGEST_LEN, AssetBegin, AssetChunk, AssetCommit, AssetKind, AssetRelease,
    MAX_ASSET_CHUNK_BYTES, MAX_ASSET_DIGESTS,
};
use sha2::{Digest, Sha256};

/// One asset resolved to bytes and ready to stream: the digest both sides
/// address it by, its wire kind, and the payload itself.
///
/// [`resolve_assets`] builds these from a compiled config's `AssetSettings`
/// (reading the referenced file and hashing it); tests build them directly.
#[derive(Debug, Clone)]
pub struct DesiredAsset {
    pub digest: [u8; ASSET_DIGEST_LEN],
    pub kind: AssetKind,
    pub bytes: Arc<[u8]>,
}

/// A failure to turn one config-declared asset into transferable bytes.
#[derive(Debug, thiserror::Error)]
pub enum AssetResolveError {
    #[error("cannot read asset {id:?} from {path:?}: {message}")]
    Io {
        id: String,
        path: String,
        message: String,
    },
    #[error("asset {id:?} is empty")]
    Empty { id: String },
    #[error("asset {id:?} is {actual} bytes, over its {maximum}-byte budget")]
    TooLarge {
        id: String,
        actual: usize,
        maximum: u32,
    },
}

/// Read every asset's source file, hash it, and pair the digest with the
/// bytes and wire kind `AssetSync::reconcile` needs. Enforces the same
/// per-asset budget the config already validated at save time -- a file that
/// changed on disk after validation must not be allowed to exceed it.
pub fn resolve_assets(settings: &[AssetSettings]) -> Result<Vec<DesiredAsset>, AssetResolveError> {
    settings.iter().map(resolve_one_asset).collect()
}

fn resolve_one_asset(setting: &AssetSettings) -> Result<DesiredAsset, AssetResolveError> {
    let AssetSource::File(path) = &setting.source;
    let bytes = fs::read(path).map_err(|error| AssetResolveError::Io {
        id: setting.id.clone(),
        path: path.clone(),
        message: error.to_string(),
    })?;
    if bytes.is_empty() {
        return Err(AssetResolveError::Empty {
            id: setting.id.clone(),
        });
    }
    if bytes.len() > setting.maximum_bytes as usize {
        return Err(AssetResolveError::TooLarge {
            id: setting.id.clone(),
            actual: bytes.len(),
            maximum: setting.maximum_bytes,
        });
    }
    let digest: [u8; ASSET_DIGEST_LEN] = Sha256::digest(&bytes).into();
    let kind = match &setting.kind {
        ConfigAssetKind::Font => AssetKind::Font,
        ConfigAssetKind::IconFont { .. } => AssetKind::IconFont,
        ConfigAssetKind::Image => AssetKind::Image,
    };
    Ok(DesiredAsset {
        digest,
        kind,
        bytes: Arc::from(bytes),
    })
}

/// What one [`AssetSync::reconcile`] pass did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AssetSyncReport {
    /// Assets the device already held (by digest), so no chunks were sent.
    pub skipped: usize,
    /// Assets actually streamed and committed this pass.
    pub uploaded: usize,
    /// The full desired digest set, exactly as sent in the closing
    /// `AssetRelease`.
    pub released: Vec<[u8; ASSET_DIGEST_LEN]>,
}

/// A failure partway through a [`AssetSync::reconcile`] pass. `reconcile`
/// stops at the first failure and does **not** send the closing
/// `AssetRelease` -- an incomplete pass must not tell the device to mark
/// anything dead, since "desired but not yet reached" is indistinguishable
/// on the wire from "no longer desired". The caller is expected to retry the
/// whole pass later (on reconnect, or on the next config apply); a retried
/// asset restarts from its first chunk, per this module's doc comment.
#[derive(Debug, thiserror::Error)]
pub enum AssetSyncError {
    #[error("{desired} desired assets exceeds the device's {maximum}-digest AssetRelease limit")]
    TooManyDesiredAssets { desired: usize, maximum: usize },
    #[error("asset {digest:02x?} is {length} bytes, over the protocol's u32 length limit")]
    AssetTooLarge {
        digest: [u8; ASSET_DIGEST_LEN],
        length: usize,
    },
    #[error("AssetBegin for {digest:02x?} failed: {source}")]
    Begin {
        digest: [u8; ASSET_DIGEST_LEN],
        #[source]
        source: DeviceError,
    },
    #[error("AssetChunk for {digest:02x?} at offset {offset} failed: {source}")]
    Chunk {
        digest: [u8; ASSET_DIGEST_LEN],
        offset: u32,
        #[source]
        source: DeviceError,
    },
    #[error("AssetCommit for {digest:02x?} failed: {source}")]
    Commit {
        digest: [u8; ASSET_DIGEST_LEN],
        #[source]
        source: DeviceError,
    },
    #[error("AssetRelease failed: {source}")]
    Release {
        #[source]
        source: DeviceError,
    },
}

pub struct AssetSync;

impl AssetSync {
    /// Reconcile `device`'s asset store with `desired`: skip anything the
    /// device already has (by digest), upload and commit everything else,
    /// then tell the device the full desired set so it can drop anything
    /// else it is holding.
    pub fn reconcile(
        device: &mut dyn RuntimeDevice,
        desired: &[DesiredAsset],
    ) -> Result<AssetSyncReport, AssetSyncError> {
        Self::reconcile_yielding(device, desired, &mut || {})
    }

    /// Same as [`Self::reconcile`], but calls `on_chunk_sent` after every
    /// chunk the device accepts.
    ///
    /// Protocol v1 allows exactly one outstanding request per connection
    /// (spec §4's "one outstanding plus preemption", chosen deliberately
    /// over a windowed transfer), and one asset can be on the order of 250
    /// sequential chunk round trips. `RuntimeDevice`'s methods are
    /// synchronous by design (see `runtime_device.rs`'s own doc comment on
    /// why: it avoids entering a Tokio runtime from a synchronous trait), so
    /// there is no `.await` point here for interactive traffic to preempt
    /// implicitly. `on_chunk_sent` is the explicit substitute: a caller that
    /// embeds `reconcile_yielding` inside a command loop (the runtime
    /// worker's, or the WebSocket actor's) is expected to use this hook to
    /// service one pending non-asset command -- a tap, a heartbeat, a
    /// pomodoro tick -- before the next chunk claims the connection's one
    /// outstanding-request slot again. `reconcile` passes a no-op hook,
    /// which is correct for a caller that owns the device exclusively for
    /// the duration of the pass and has nothing else to interleave.
    pub fn reconcile_yielding(
        device: &mut dyn RuntimeDevice,
        desired: &[DesiredAsset],
        on_chunk_sent: &mut dyn FnMut(),
    ) -> Result<AssetSyncReport, AssetSyncError> {
        // `MAX_ASSET_DIGESTS` (32) comfortably covers the config's own
        // `MAX_ASSETS` (16) today, but that headroom is a property of two
        // constants that could drift independently. Assert it here rather
        // than assume it: a release that silently dropped digests past a
        // truncation point would tell the device to delete assets that are
        // still desired.
        if desired.len() > MAX_ASSET_DIGESTS {
            return Err(AssetSyncError::TooManyDesiredAssets {
                desired: desired.len(),
                maximum: MAX_ASSET_DIGESTS,
            });
        }

        let mut skipped = 0usize;
        let mut uploaded = 0usize;

        for asset in desired {
            let total_length =
                u32::try_from(asset.bytes.len()).map_err(|_| AssetSyncError::AssetTooLarge {
                    digest: asset.digest,
                    length: asset.bytes.len(),
                })?;

            let ack = device
                .send_asset_begin(AssetBegin {
                    digest: asset.digest,
                    kind: asset.kind,
                    total_length,
                    // Stage 1 has no volatile (PSRAM) tier -- the device
                    // refuses `volatile: true` outright. That tier exists
                    // for rasterized frames in a later stage.
                    volatile: false,
                })
                .map_err(|source| AssetSyncError::Begin {
                    digest: asset.digest,
                    source,
                })?;

            // Content addressing IS the inventory protocol: `already_present`
            // is the only signal we act on, and it means "send no chunks",
            // full stop -- not "send fewer chunks".
            if ack.already_present == Some(true) {
                skipped += 1;
                continue;
            }

            let mut offset: u32 = 0;
            for chunk in asset.bytes.chunks(MAX_ASSET_CHUNK_BYTES) {
                let chunk_len = u32::try_from(chunk.len())
                    .expect("Vec::chunks yields pieces bounded by MAX_ASSET_CHUNK_BYTES");
                device
                    .send_asset_chunk(AssetChunk {
                        digest: asset.digest,
                        offset,
                        data: chunk.to_vec(),
                    })
                    .map_err(|source| AssetSyncError::Chunk {
                        digest: asset.digest,
                        offset,
                        source,
                    })?;
                offset += chunk_len;
                on_chunk_sent();
            }

            device
                .send_asset_commit(AssetCommit {
                    digest: asset.digest,
                })
                .map_err(|source| AssetSyncError::Commit {
                    digest: asset.digest,
                    source,
                })?;
            uploaded += 1;
        }

        let released: Vec<[u8; ASSET_DIGEST_LEN]> =
            desired.iter().map(|asset| asset.digest).collect();
        device
            .send_asset_release(AssetRelease {
                digests: released.clone(),
            })
            .map_err(|source| AssetSyncError::Release { source })?;

        Ok(AssetSyncReport {
            skipped,
            uploaded,
            released,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use app_core::{
        AssetKind as ConfigAssetKind, AssetSettings, AssetSource, DeviceConnection, RuntimeDevice,
    };
    use device::{ReceivedEvent, SessionDiagnostics};
    use protocol::{
        Ack, Field, NetworkConfig, PushScene, ScreenConfig, StatusResponse, TYPE_ASSET_BEGIN,
        TimeSync, TriggerInterrupt, WidgetConfig,
    };

    use super::*;

    fn asset_blob(digest: [u8; ASSET_DIGEST_LEN], len: usize) -> DesiredAsset {
        // Deterministic, non-zero filler -- a real font/image is never all
        // zero bytes, and a chunker that silently coalesced runs of zeros
        // would still pass a naive all-zero fixture.
        let bytes: Vec<u8> = (0..len)
            .map(|index| u8::try_from(index % 251).expect("index % 251 fits in a u8"))
            .collect();
        DesiredAsset {
            digest,
            kind: AssetKind::Font,
            bytes: Arc::from(bytes),
        }
    }

    /// A `RuntimeDevice` double that only really implements the asset
    /// methods `AssetSync` calls; everything else is unreachable because
    /// `reconcile` never calls it.
    #[derive(Default)]
    struct FakeDevice {
        already_present: std::collections::HashSet<[u8; ASSET_DIGEST_LEN]>,
        holding: std::collections::HashSet<[u8; ASSET_DIGEST_LEN]>,
        /// Chunks accepted since the last `recovered()` (or since creation).
        chunks_since_recovery: HashMap<[u8; ASSET_DIGEST_LEN], u32>,
        /// Chunks accepted for the transfer opened by the most recent
        /// `AssetBegin`, reset on every `AssetBegin` -- this is what a real
        /// device's own uncommitted-transfer progress would track.
        current_transfer_chunks: u32,
        fail_after_chunks: Option<u32>,
        committed: std::collections::HashSet<[u8; ASSET_DIGEST_LEN]>,
        last_release: Option<Vec<[u8; ASSET_DIGEST_LEN]>>,
    }

    impl FakeDevice {
        fn new() -> Self {
            Self::default()
        }

        fn with_already_present(mut self, digest: [u8; ASSET_DIGEST_LEN]) -> Self {
            self.already_present.insert(digest);
            self
        }

        fn holding(mut self, digests: impl IntoIterator<Item = [u8; ASSET_DIGEST_LEN]>) -> Self {
            self.holding.extend(digests);
            self
        }

        /// Fail every `AssetChunk` call once the currently-open transfer has
        /// already accepted `count` chunks -- i.e. the `(count + 1)`th chunk
        /// of the asset fails.
        fn failing_after_chunks(mut self, count: u32) -> Self {
            self.fail_after_chunks = Some(count);
            self
        }

        /// Clear the injected failure and reset the per-pass chunk tally, as
        /// if a fresh connection replaced the one that just failed.
        /// `chunks_since_recovery` therefore counts only what this next
        /// `reconcile` call actually sends, which is what proves a retry
        /// restarts an asset rather than resuming a remembered offset.
        fn recovered(&mut self) -> &mut Self {
            self.fail_after_chunks = None;
            self.chunks_since_recovery.clear();
            self
        }

        fn chunks_sent_for(&self, digest: &[u8; ASSET_DIGEST_LEN]) -> u32 {
            self.chunks_since_recovery.get(digest).copied().unwrap_or(0)
        }

        fn last_release(&self) -> Option<Vec<[u8; ASSET_DIGEST_LEN]>> {
            self.last_release.clone()
        }
    }

    impl RuntimeDevice for FakeDevice {
        fn connect(&mut self) -> Result<DeviceConnection, DeviceError> {
            unreachable!("FakeDevice only exercises asset transfer in these tests")
        }
        fn status(&mut self) -> Result<StatusResponse, DeviceError> {
            unreachable!("FakeDevice only exercises asset transfer in these tests")
        }
        fn provision(&mut self, _config: &NetworkConfig) -> Result<(), DeviceError> {
            unreachable!("FakeDevice only exercises asset transfer in these tests")
        }
        fn factory_reset(&mut self) -> Result<(), DeviceError> {
            unreachable!("FakeDevice only exercises asset transfer in these tests")
        }
        fn time_sync(&mut self, _sync: TimeSync) -> Result<(), DeviceError> {
            unreachable!("FakeDevice only exercises asset transfer in these tests")
        }
        fn apply_layout(
            &mut self,
            _rotation: u16,
            _widgets: Vec<WidgetConfig>,
            _screens: Vec<ScreenConfig>,
        ) -> Result<(), DeviceError> {
            unreachable!("FakeDevice only exercises asset transfer in these tests")
        }
        fn push_fields(
            &mut self,
            _widget_id: String,
            _fields: Vec<Field>,
        ) -> Result<(), DeviceError> {
            unreachable!("FakeDevice only exercises asset transfer in these tests")
        }
        fn activate_screen(&mut self, _screen_id: String) -> Result<(), DeviceError> {
            unreachable!("FakeDevice only exercises asset transfer in these tests")
        }
        fn push_scene(&mut self, _push: PushScene) -> Result<(), DeviceError> {
            unreachable!("FakeDevice only exercises asset transfer in these tests")
        }
        fn trigger_interrupt(&mut self, _interrupt: TriggerInterrupt) -> Result<(), DeviceError> {
            unreachable!("FakeDevice only exercises asset transfer in these tests")
        }

        fn send_asset_begin(&mut self, begin: AssetBegin) -> Result<Ack, DeviceError> {
            let already_present = self.already_present.contains(&begin.digest)
                || self.holding.contains(&begin.digest);
            // A fresh `AssetBegin` always opens a new reservation and aborts
            // whatever was in flight for this digest before -- there is no
            // wire-level resume, so a real device's per-transfer progress
            // resets here too.
            self.current_transfer_chunks = 0;
            Ok(Ack {
                acknowledged_type: TYPE_ASSET_BEGIN,
                revision: None,
                already_present: Some(already_present),
            })
        }

        fn send_asset_chunk(&mut self, chunk: AssetChunk) -> Result<(), DeviceError> {
            self.current_transfer_chunks += 1;
            if let Some(limit) = self.fail_after_chunks
                && self.current_transfer_chunks > limit
            {
                return Err(DeviceError::Timeout);
            }
            *self.chunks_since_recovery.entry(chunk.digest).or_insert(0) += 1;
            Ok(())
        }

        fn send_asset_commit(&mut self, commit: AssetCommit) -> Result<(), DeviceError> {
            self.committed.insert(commit.digest);
            Ok(())
        }

        fn send_asset_release(&mut self, release: AssetRelease) -> Result<(), DeviceError> {
            self.last_release = Some(release.digests);
            Ok(())
        }

        fn try_recv_event(&mut self) -> Option<ReceivedEvent> {
            None
        }
        fn diagnostics(&self) -> SessionDiagnostics {
            SessionDiagnostics::default()
        }
    }

    #[test]
    fn reconcile_skips_assets_the_device_already_holds() {
        let mut device = FakeDevice::new().with_already_present([0xaa; ASSET_DIGEST_LEN]);
        let desired = vec![
            asset_blob([0xaa; ASSET_DIGEST_LEN], 4096),
            asset_blob([0xbb; ASSET_DIGEST_LEN], 2048),
        ];

        let report = AssetSync::reconcile(&mut device, &desired).expect("reconcile");

        // Content addressing is the inventory protocol: AssetBegin answers
        // already_present and we send no chunks at all for that digest.
        assert_eq!(report.skipped, 1);
        assert_eq!(report.uploaded, 1);
        assert_eq!(device.chunks_sent_for(&[0xaa; ASSET_DIGEST_LEN]), 0);
        assert!(device.chunks_sent_for(&[0xbb; ASSET_DIGEST_LEN]) > 0);
    }

    /// The brief this module was built from sketched a test asserting that a
    /// retried reconcile resumes from a remembered byte offset. That is not
    /// how the wire protocol works: `AssetBegin` always opens a fresh
    /// reservation and the device aborts whatever was in flight for that
    /// digest first (Task 9's protocol wiring), and `Ack` carries no offset
    /// for `reconcile` to learn one from. This test instead pins the actual,
    /// intentional behavior -- a retry restarts the asset's chunks from
    /// zero -- and genuinely exercises a mid-transfer failure followed by a
    /// successful pass, which is what the brief actually asked this test to
    /// prove.
    #[test]
    fn reconcile_restarts_an_asset_after_a_mid_transfer_failure() {
        let mut device = FakeDevice::new().failing_after_chunks(2);
        let digest = [0xcc; ASSET_DIGEST_LEN];
        let desired = vec![asset_blob(digest, MAX_ASSET_CHUNK_BYTES * 5)];

        let first_attempt = AssetSync::reconcile(&mut device, &desired);
        assert!(
            first_attempt.is_err(),
            "the injected failure must actually fire"
        );
        // Two chunks were accepted before the third failed.
        assert_eq!(device.chunks_sent_for(&digest), 2);

        device.recovered();
        let report = AssetSync::reconcile(&mut device, &desired).expect("the retry must succeed");

        assert_eq!(report.uploaded, 1);
        assert_eq!(report.skipped, 0);
        // The retry sent all 5 chunks again, not just the remaining 3 --
        // proving it restarted rather than resuming from offset 2 * CHUNK.
        assert_eq!(device.chunks_sent_for(&digest), 5);
        assert_eq!(report.released, vec![digest]);
    }

    #[test]
    fn reconcile_releases_digests_no_longer_desired() {
        let mut device =
            FakeDevice::new().holding([[0xaa; ASSET_DIGEST_LEN], [0xdd; ASSET_DIGEST_LEN]]);
        let desired = vec![asset_blob([0xaa; ASSET_DIGEST_LEN], 128)];

        AssetSync::reconcile(&mut device, &desired).expect("reconcile");

        assert_eq!(device.last_release(), Some(vec![[0xaa; ASSET_DIGEST_LEN]]));
    }

    #[test]
    fn reconcile_yielding_calls_the_hook_once_per_chunk() {
        let mut device = FakeDevice::new();
        let desired = vec![asset_blob(
            [0xee; ASSET_DIGEST_LEN],
            MAX_ASSET_CHUNK_BYTES * 3,
        )];
        let mut yields = 0usize;

        AssetSync::reconcile_yielding(&mut device, &desired, &mut || yields += 1)
            .expect("reconcile");

        assert_eq!(yields, 3);
    }

    #[test]
    fn reconcile_rejects_more_desired_assets_than_a_release_can_carry() {
        let mut device = FakeDevice::new();
        let max_digests_as_u8 =
            u8::try_from(MAX_ASSET_DIGESTS).expect("MAX_ASSET_DIGESTS fits in a u8");
        let desired: Vec<DesiredAsset> = (0u8..=max_digests_as_u8)
            .map(|index| asset_blob([index; ASSET_DIGEST_LEN], 16))
            .collect();
        assert!(desired.len() > MAX_ASSET_DIGESTS);

        let error = AssetSync::reconcile(&mut device, &desired).expect_err("must be rejected");

        assert!(matches!(error, AssetSyncError::TooManyDesiredAssets { .. }));
        assert!(device.last_release().is_none());
    }

    #[test]
    fn resolve_assets_reads_hashes_and_kinds_the_source_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("inter.ttf");
        std::fs::write(&path, b"not really a font, just bytes").expect("write fixture");

        let settings = vec![AssetSettings {
            id: "inter".into(),
            source: AssetSource::File(path.to_string_lossy().into_owned()),
            kind: ConfigAssetKind::Font,
            maximum_bytes: 1024,
        }];

        let resolved = resolve_assets(&settings).expect("resolve");

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].kind, AssetKind::Font);
        assert_eq!(
            &*resolved[0].bytes,
            b"not really a font, just bytes".as_slice()
        );
        let expected_digest: [u8; ASSET_DIGEST_LEN] =
            Sha256::digest(b"not really a font, just bytes").into();
        assert_eq!(resolved[0].digest, expected_digest);
    }

    #[test]
    fn resolve_assets_rejects_a_file_over_its_own_budget() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("big.ttf");
        std::fs::write(&path, vec![0u8; 64]).expect("write fixture");

        let settings = vec![AssetSettings {
            id: "big".into(),
            source: AssetSource::File(path.to_string_lossy().into_owned()),
            kind: ConfigAssetKind::Image,
            maximum_bytes: 32,
        }];

        let error = resolve_assets(&settings).expect_err("must be rejected");

        assert!(matches!(error, AssetResolveError::TooLarge { .. }));
    }
}
