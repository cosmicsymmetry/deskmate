//! Server-side asset transfer driver over the `RuntimeDevice` seam.
//!
//! Server-owned WebSocket devices support asset transfer through the same
//! `RuntimeDevice` methods as other runtime devices, so
//! [`AssetSync::reconcile`] can reconcile the complete
//! desired set without transport-specific branching.
//!
//! Content addressing is the whole inventory protocol: there is no separate
//! "what do you have?" query. Reconciliation sends `AssetBegin` for every
//! desired asset; when the device's `Ack` reports `already_present`, no
//! chunks are sent for that digest at all. Because `AssetBegin` always opens
//! a fresh reservation -- the device aborts any transfer already in flight
//! for that digest first (protocol v1 §4) -- there is no wire-level
//! way to resume a partially-uploaded asset from a remembered byte offset.
//! A dropped connection, or a retry after a local failure, restarts that
//! asset's chunks from zero; only whole committed assets are ever skipped.

use std::borrow::Cow;
use std::sync::Arc;

use crate::RuntimeDevice;
use device::DeviceError;
use protocol::{
    ASSET_DIGEST_LEN, ASSET_ENCODING_RAW, ASSET_ENCODING_RLE565, AssetBegin, AssetChunk,
    AssetCommit, AssetKind, AssetRelease, CAPABILITY_DURABLE_ASSET_ENCODING,
    CAPABILITY_VOLATILE_ASSETS, MAX_ASSET_CHUNK_BYTES, MAX_ASSET_DIGESTS, encode_rle565,
};

/// One asset resolved to bytes and ready to stream: the digest both sides
/// address it by, its wire kind, and the payload itself.
///
/// For a raster frame, `bytes` is always the decoded canonical LVGL blob and
/// `digest` hashes those decoded bytes. Transfer encoding is chosen
/// inside [`AssetSync`], never by the caller constructing this value.
///
/// Image-source hosts build these from bytes they already own; tests
/// build them directly.
#[derive(Debug, Clone)]
pub struct DesiredAsset {
    pub digest: [u8; ASSET_DIGEST_LEN],
    pub kind: AssetKind,
    pub bytes: Arc<[u8]>,
}

/// A failure partway through an asset reconciliation pass. Reconciliation
/// stops at the first failure and does **not** send the closing
/// `AssetRelease` -- an incomplete pass must not tell the device to mark
/// anything dead, since "desired but not yet reached" is indistinguishable
/// on the wire from "no longer desired". The caller is expected to retry the
/// whole pass later (on reconnect, or on the next config apply); a retried
/// asset restarts from its first chunk, per this module's doc comment.
#[derive(Debug, thiserror::Error)]
pub(crate) enum AssetSyncError {
    #[error("{desired} desired assets exceeds the device's {maximum}-digest AssetRelease limit")]
    TooManyDesiredAssets { desired: usize, maximum: usize },
    #[error("asset {digest:02x?} is {length} bytes, over the protocol's u32 length limit")]
    AssetTooLarge {
        digest: [u8; ASSET_DIGEST_LEN],
        length: usize,
    },
    #[error("required scene asset {digest:02x?} is not available from this host")]
    MissingRequiredAsset { digest: [u8; ASSET_DIGEST_LEN] },
    /// `docs/protocol/v2.md`: when `volatile` is true, `kind` must be `Image`.
    /// Refused here rather than on the wire so a caller's mistake never costs a
    /// device round trip and an `ErrorCode`.
    #[error("volatile asset {digest:02x?} must be an image, got {kind:?}")]
    VolatileKind {
        digest: [u8; ASSET_DIGEST_LEN],
        kind: AssetKind,
    },
    /// The device did not negotiate capability bit 9. Every deployed decoder
    /// refuses `AssetBegin { volatile: true }` without it, so this is caught
    /// here for the same reason.
    #[error("the device does not accept volatile assets")]
    VolatileUnsupported,
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

pub(crate) struct AssetSync;

struct SelectedAssetEncoding<'a> {
    wire: Cow<'a, [u8]>,
    encoding: u8,
    decoded_length: Option<u32>,
}

impl AssetSync {
    fn select_image_encoding(
        asset: &DesiredAsset,
        supports_rle: bool,
    ) -> Result<SelectedAssetEncoding<'_>, AssetSyncError> {
        let decoded_length =
            u32::try_from(asset.bytes.len()).map_err(|_| AssetSyncError::AssetTooLarge {
                digest: asset.digest,
                length: asset.bytes.len(),
            })?;
        let mut encoding = ASSET_ENCODING_RAW;
        let mut wire: Cow<'_, [u8]> = Cow::Borrowed(asset.bytes.as_ref());
        let mut encoded_decoded_length = None;
        if supports_rle
            && asset.bytes.len() >= 12
            && let Ok(encoded_pixels) = encode_rle565(&asset.bytes[12..])
        {
            let mut candidate = Vec::with_capacity(12 + encoded_pixels.len());
            candidate.extend_from_slice(&asset.bytes[..12]);
            candidate.extend_from_slice(&encoded_pixels);
            if candidate.len() < asset.bytes.len() {
                encoding = ASSET_ENCODING_RLE565;
                encoded_decoded_length = Some(decoded_length);
                wire = Cow::Owned(candidate);
            }
        }
        Ok(SelectedAssetEncoding {
            wire,
            encoding,
            decoded_length: encoded_decoded_length,
        })
    }

    /// Uploads one raster to the device's **volatile** PSRAM tier, leaving the
    /// durable inventory and the keep-set alone.
    ///
    /// This is the interactive path. A durable transfer costs an `AssetCommit`
    /// that writes flash (~1.5 s measured on `dev-0005`) and leaves a dead
    /// record whose reclamation compacts the blob region (~9.4 s measured),
    /// during which the panel shows its clock. A volatile transfer costs a
    /// `memcpy` into PSRAM and no partition endurance at all.
    ///
    /// The digest addresses **decoded** bytes, so it is identical in both
    /// tiers: the same frame can be pushed here now and written durably later
    /// without the scene that references it changing. `protocol_asset_resolver`
    /// checks PSRAM before flash, so the durable copy silently becomes the
    /// fallback once the volatile one is evicted.
    pub(crate) fn transfer_volatile(
        device: &mut dyn RuntimeDevice,
        asset: &DesiredAsset,
        capabilities: u64,
    ) -> Result<(), AssetSyncError> {
        if capabilities & CAPABILITY_VOLATILE_ASSETS == 0 {
            return Err(AssetSyncError::VolatileUnsupported);
        }
        if asset.kind != AssetKind::Image {
            return Err(AssetSyncError::VolatileKind {
                digest: asset.digest,
                kind: asset.kind,
            });
        }
        // The volatile tier decodes RLE565 itself and always has -- capability
        // bit 10 gates the encoding on the DURABLE tier only, so this one is
        // free to compress regardless of what the device negotiated.
        let selected = Self::select_image_encoding(asset, true)?;
        Self::transfer_one(
            device,
            asset,
            true,
            &selected.wire,
            selected.encoding,
            selected.decoded_length,
        )
    }

    fn transfer_one(
        device: &mut dyn RuntimeDevice,
        asset: &DesiredAsset,
        volatile: bool,
        wire_bytes: &[u8],
        encoding: u8,
        decoded_length: Option<u32>,
    ) -> Result<(), AssetSyncError> {
        let total_length =
            u32::try_from(wire_bytes.len()).map_err(|_| AssetSyncError::AssetTooLarge {
                digest: asset.digest,
                length: wire_bytes.len(),
            })?;

        let ack = device
            .send_asset_begin(AssetBegin {
                digest: asset.digest,
                kind: asset.kind,
                total_length,
                volatile,
                encoding,
                decoded_length,
            })
            .map_err(|source| AssetSyncError::Begin {
                digest: asset.digest,
                source,
            })?;
        if ack.already_present == Some(true) {
            return Ok(());
        }

        let mut offset: u32 = 0;
        for chunk in wire_bytes.chunks(MAX_ASSET_CHUNK_BYTES) {
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
        }

        device
            .send_asset_commit(AssetCommit {
                digest: asset.digest,
            })
            .map_err(|source| AssetSyncError::Commit {
                digest: asset.digest,
                source,
            })?;
        Ok(())
    }

    /// Reconciles desired assets and returns the exact keep-set sent to the device.
    pub(crate) fn reconcile(
        device: &mut dyn RuntimeDevice,
        desired: &[DesiredAsset],
        capabilities: u64,
    ) -> Result<Vec<[u8; ASSET_DIGEST_LEN]>, AssetSyncError> {
        Self::reconcile_releasing(device, desired, capabilities, true)
    }

    /// Reconciles, optionally withholding the closing `AssetRelease`.
    ///
    /// The release is what reclaims a replaced frame, and reclaiming compacts
    /// the flash blob region: `asset_store_plan_compaction` moves every record
    /// that sits after a dead one, which measured **9.36 s** on `dev-0005`
    /// (2026-09-26) with the panel showing its clock throughout. Sending it on
    /// every pass therefore pays that cost four times every fifteen minutes,
    /// forever, to reclaim space nothing is waiting for.
    ///
    /// Withholding it is safe because the message is a KEEP-set: a device that
    /// receives no release simply keeps what it has. The only cost is dead
    /// records, and the assets partition is **6 MB** against frames of roughly
    /// 35 KB -- about 170 of them -- while the four faces produce 16 an hour.
    /// Half an hour of deferral is some eight dead records. There is no version
    /// of this cadence that fills the partition before the next release.
    ///
    /// `false` never loses anything permanently: the next pass that does send
    /// one carries the complete keep-set and reclaims everything at once.
    pub(crate) fn reconcile_releasing(
        device: &mut dyn RuntimeDevice,
        desired: &[DesiredAsset],
        capabilities: u64,
        release: bool,
    ) -> Result<Vec<[u8; ASSET_DIGEST_LEN]>, AssetSyncError> {
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

        for asset in desired {
            let selected = if asset.kind == AssetKind::Image {
                Self::select_image_encoding(
                    asset,
                    capabilities & CAPABILITY_DURABLE_ASSET_ENCODING != 0,
                )?
            } else {
                SelectedAssetEncoding {
                    wire: Cow::Borrowed(asset.bytes.as_ref()),
                    encoding: ASSET_ENCODING_RAW,
                    decoded_length: None,
                }
            };
            Self::transfer_one(
                device,
                asset,
                false,
                &selected.wire,
                selected.encoding,
                selected.decoded_length,
            )?;
        }

        let mut keep_set = Vec::with_capacity(desired.len());
        for digest in desired.iter().map(|asset| asset.digest) {
            if !keep_set.contains(&digest) {
                keep_set.push(digest);
            }
        }
        if release {
            device
                .send_asset_release(AssetRelease {
                    digests: keep_set.clone(),
                })
                .map_err(|source| AssetSyncError::Release { source })?;
        }

        Ok(keep_set)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::{DeviceConnection, RuntimeDevice};
    use device::{ReceivedEvent, SessionDiagnostics};
    use protocol::{
        Ack, CAPABILITY_VOLATILE_ASSETS, CardConfig, PushScene, StatusResponse, TYPE_ASSET_BEGIN,
        TimeSync, TriggerInterrupt,
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

    fn raster_frame(digest: [u8; ASSET_DIGEST_LEN], high_entropy: bool) -> DesiredAsset {
        let mut bytes = Vec::with_capacity(protocol::VOLATILE_IMAGE_DECODED_LENGTH as usize);
        bytes.extend_from_slice(&[0x19, 0x12, 0, 0, 0xc0, 0x01, 0x70, 0x01, 0x80, 0x03, 0, 0]);
        if high_entropy {
            let mut state = 0x6d2b_79f5u32;
            for _ in 0..164_864 {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                bytes.extend_from_slice(&u16::try_from(state & 0xffff).unwrap().to_le_bytes());
            }
        } else {
            for _ in 0..164_864 {
                bytes.extend_from_slice(&0x1234u16.to_le_bytes());
            }
        }
        assert_eq!(
            bytes.len(),
            protocol::VOLATILE_IMAGE_DECODED_LENGTH as usize
        );
        DesiredAsset {
            digest,
            kind: AssetKind::Image,
            bytes: Arc::from(bytes),
        }
    }

    /// A `RuntimeDevice` double that only really implements the asset
    /// methods `AssetSync` calls; everything else is unreachable because
    /// reconciliation never calls it.
    #[derive(Default)]
    struct FakeDevice {
        already_present: std::collections::HashSet<[u8; ASSET_DIGEST_LEN]>,
        /// Chunks accepted since the last `recovered()` (or since creation).
        chunks_since_recovery: HashMap<[u8; ASSET_DIGEST_LEN], u32>,
        /// Chunks accepted for the transfer opened by the most recent
        /// `AssetBegin`, reset on every `AssetBegin` -- this is what a real
        /// device's own uncommitted-transfer progress would track.
        current_transfer_chunks: u32,
        fail_after_chunks: Option<u32>,
        committed: std::collections::HashSet<[u8; ASSET_DIGEST_LEN]>,
        last_release: Option<Vec<[u8; ASSET_DIGEST_LEN]>>,
        begins: Vec<AssetBegin>,
        wire_bytes: HashMap<[u8; ASSET_DIGEST_LEN], Vec<u8>>,
    }

    impl FakeDevice {
        fn new() -> Self {
            Self::default()
        }

        fn with_already_present(mut self, digest: [u8; ASSET_DIGEST_LEN]) -> Self {
            self.already_present.insert(digest);
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

        fn begins(&self) -> &[AssetBegin] {
            &self.begins
        }

        fn wire_bytes(&self, digest: &[u8; ASSET_DIGEST_LEN]) -> &[u8] {
            self.wire_bytes.get(digest).map_or(&[], Vec::as_slice)
        }
    }

    impl RuntimeDevice for FakeDevice {
        fn connect(&mut self) -> Result<DeviceConnection, DeviceError> {
            unreachable!("FakeDevice only exercises asset transfer in these tests")
        }
        fn status(&mut self) -> Result<StatusResponse, DeviceError> {
            unreachable!("FakeDevice only exercises asset transfer in these tests")
        }
        fn time_sync(&mut self, _sync: TimeSync) -> Result<(), DeviceError> {
            unreachable!("FakeDevice only exercises asset transfer in these tests")
        }
        fn apply_layout(
            &mut self,
            _rotation: u16,
            _cards: Vec<CardConfig>,
        ) -> Result<(), DeviceError> {
            unreachable!("FakeDevice only exercises asset transfer in these tests")
        }
        fn push_timer(
            &mut self,
            _card_id: String,
            _total_ms: u32,
            _remaining_ms: u32,
            _running: bool,
        ) -> Result<(), DeviceError> {
            unreachable!("FakeDevice only exercises asset transfer in these tests")
        }
        fn activate_card(&mut self, _card_id: String) -> Result<(), DeviceError> {
            unreachable!("FakeDevice only exercises asset transfer in these tests")
        }
        fn push_scene(&mut self, _push: PushScene) -> Result<(), DeviceError> {
            unreachable!("FakeDevice only exercises asset transfer in these tests")
        }
        fn trigger_interrupt(&mut self, _interrupt: TriggerInterrupt) -> Result<(), DeviceError> {
            unreachable!("FakeDevice only exercises asset transfer in these tests")
        }

        fn send_asset_begin(&mut self, begin: AssetBegin) -> Result<Ack, DeviceError> {
            let already_present = self.already_present.contains(&begin.digest);
            // A fresh `AssetBegin` always opens a new reservation and aborts
            // whatever was in flight for this digest before -- there is no
            // wire-level resume, so a real device's per-transfer progress
            // resets here too.
            self.current_transfer_chunks = 0;
            self.wire_bytes.remove(&begin.digest);
            self.begins.push(begin);
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
            let wire = self.wire_bytes.entry(chunk.digest).or_default();
            assert_eq!(usize::try_from(chunk.offset).unwrap(), wire.len());
            wire.extend_from_slice(&chunk.data);
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
    fn a_withheld_release_still_installs_the_frame_and_reports_the_keep_set() {
        // Withholding is housekeeping deferred, not work skipped: the bytes still
        // land, and the caller still learns the complete keep-set so its resident
        // bookkeeping stays accurate for the pass that does release.
        let mut device = FakeDevice::new();
        let digest = [0x6a; ASSET_DIGEST_LEN];
        let desired = vec![asset_blob(digest, 16)];

        let keep_set = AssetSync::reconcile_releasing(&mut device, &desired, 0, false)
            .expect("a deferred reconcile");

        assert_eq!(keep_set, vec![digest], "the keep-set is still computed");
        assert!(device.chunks_sent_for(&digest) > 0, "the bytes still land");
        assert_eq!(
            device.last_release(),
            None,
            "no release means no compaction, which is the entire point"
        );
    }

    #[test]
    fn a_volatile_transfer_spends_no_flash_and_no_inventory() {
        // The whole point of the tier. A durable pass writes the record and then
        // sends the AssetRelease whose compaction was measured at 9.4 s on
        // dev-0005, with the panel showing its clock throughout. The interactive
        // path must do neither: no release means nothing is reclaimed and nothing
        // is moved.
        let mut device = FakeDevice::new();
        let digest = [0x5a; ASSET_DIGEST_LEN];

        AssetSync::transfer_volatile(
            &mut device,
            &raster_frame(digest, false),
            CAPABILITY_VOLATILE_ASSETS,
        )
        .expect("a volatile transfer");

        assert_eq!(
            device.last_release(),
            None,
            "a volatile transfer must not send AssetRelease: that is what compacts the blob region"
        );
        assert!(device.chunks_sent_for(&digest) > 0, "the frame was sent");
        let begin = device.begins().last().expect("one AssetBegin");
        assert!(
            begin.volatile,
            "the frame must be addressed to the PSRAM tier, not written to flash"
        );
    }

    #[test]
    fn a_volatile_transfer_compresses_whatever_the_durable_tier_negotiated() {
        // Capability bit 10 gates RLE565 on the DURABLE tier only; the volatile
        // tier has always decoded it. Sending raw here would triple the chunk
        // count, and the chunk phase is round-trip-bound -- 17 acks at ~76 ms,
        // so every avoided chunk is real interactive latency.
        let mut device = FakeDevice::new();
        let digest = [0x5b; ASSET_DIGEST_LEN];

        AssetSync::transfer_volatile(
            &mut device,
            &raster_frame(digest, false),
            CAPABILITY_VOLATILE_ASSETS,
        )
        .expect("a volatile transfer");

        let begin = device.begins().last().expect("one AssetBegin");
        assert_eq!(begin.encoding, ASSET_ENCODING_RLE565);
        assert_eq!(
            begin.decoded_length,
            Some(protocol::VOLATILE_IMAGE_DECODED_LENGTH)
        );
    }

    #[test]
    fn a_volatile_transfer_is_refused_without_the_capability_or_the_kind() {
        // Both are wire contract rules (`docs/protocol/v2.md`). Refusing here
        // keeps a caller's mistake off the wire, where it would cost a round trip
        // and come back as an ErrorCode.
        let mut device = FakeDevice::new();
        let digest = [0x5c; ASSET_DIGEST_LEN];

        assert!(matches!(
            AssetSync::transfer_volatile(&mut device, &raster_frame(digest, false), 0),
            Err(AssetSyncError::VolatileUnsupported)
        ));
        assert!(matches!(
            AssetSync::transfer_volatile(
                &mut device,
                &asset_blob(digest, 16),
                CAPABILITY_VOLATILE_ASSETS
            ),
            Err(AssetSyncError::VolatileKind { .. })
        ));
        assert!(
            device.begins().is_empty(),
            "neither refusal may reach the device"
        );
    }

    #[test]
    fn reconcile_deduplicates_the_keep_set_in_first_seen_order() {
        let mut device = FakeDevice::new();
        let first = [0xbb; ASSET_DIGEST_LEN];
        let second = [0xaa; ASSET_DIGEST_LEN];
        let desired = vec![
            asset_blob(first, 16),
            asset_blob(second, 16),
            asset_blob(first, 16),
        ];

        let keep_set = AssetSync::reconcile(&mut device, &desired, 0).expect("reconcile");

        assert_eq!(keep_set, vec![first, second]);
        assert_eq!(device.last_release(), Some(keep_set));
    }

    #[test]
    fn reconcile_sends_an_empty_keep_set() {
        let mut device = FakeDevice::new();

        let keep_set = AssetSync::reconcile(&mut device, &[], 0).expect("reconcile");

        assert!(keep_set.is_empty());
        assert_eq!(device.last_release(), Some(Vec::new()));
    }

    #[test]
    fn reconcile_skips_assets_the_device_already_holds() {
        let mut device = FakeDevice::new().with_already_present([0xaa; ASSET_DIGEST_LEN]);
        let desired = vec![
            asset_blob([0xaa; ASSET_DIGEST_LEN], 4096),
            asset_blob([0xbb; ASSET_DIGEST_LEN], 2048),
        ];

        let keep_set = AssetSync::reconcile(&mut device, &desired, 0).expect("reconcile");

        // Content addressing is the inventory protocol: AssetBegin answers
        // already_present and we send no chunks at all for that digest.
        assert_eq!(device.chunks_sent_for(&[0xaa; ASSET_DIGEST_LEN]), 0);
        assert!(device.chunks_sent_for(&[0xbb; ASSET_DIGEST_LEN]) > 0);
        assert_eq!(
            keep_set,
            vec![[0xaa; ASSET_DIGEST_LEN], [0xbb; ASSET_DIGEST_LEN]]
        );
        assert_eq!(device.last_release(), Some(keep_set));
    }

    /// `AssetBegin` always opens a fresh reservation, aborting any transfer
    /// already in flight for that digest, and `Ack` carries no resumable
    /// offset. A retry therefore restarts the asset's chunks from zero. This
    /// test exercises that contract across a mid-transfer failure followed by
    /// a successful pass.
    #[test]
    fn reconcile_restarts_an_asset_after_a_mid_transfer_failure() {
        let mut device = FakeDevice::new().failing_after_chunks(2);
        let digest = [0xcc; ASSET_DIGEST_LEN];
        let desired = vec![asset_blob(digest, MAX_ASSET_CHUNK_BYTES * 5)];

        let first_attempt = AssetSync::reconcile(&mut device, &desired, 0);
        assert!(
            first_attempt.is_err(),
            "the injected failure must actually fire"
        );
        // Two chunks were accepted before the third failed.
        assert_eq!(device.chunks_sent_for(&digest), 2);

        device.recovered();
        let keep_set =
            AssetSync::reconcile(&mut device, &desired, 0).expect("the retry must succeed");

        // The retry sent all 5 chunks again, not just the remaining 3 --
        // proving it restarted rather than resuming from offset 2 * CHUNK.
        assert_eq!(device.chunks_sent_for(&digest), 5);
        assert_eq!(keep_set, vec![digest]);
        assert_eq!(device.last_release(), Some(keep_set));
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

        let error = AssetSync::reconcile(&mut device, &desired, 0).expect_err("must be rejected");

        assert!(matches!(error, AssetSyncError::TooManyDesiredAssets { .. }));
        assert!(device.last_release().is_none());
    }

    #[test]
    fn reconcile_durable_uses_rle_when_it_is_smaller() {
        let mut device = FakeDevice::new();
        let frame = raster_frame([0xd1; ASSET_DIGEST_LEN], false);

        AssetSync::reconcile(
            &mut device,
            std::slice::from_ref(&frame),
            CAPABILITY_DURABLE_ASSET_ENCODING,
        )
        .expect("durable reconcile");

        assert_eq!(device.begins().len(), 1);
        let begin = device.begins()[0];
        assert!(!begin.volatile);
        assert_eq!(begin.encoding, ASSET_ENCODING_RLE565);
        assert_eq!(
            begin.decoded_length,
            Some(protocol::VOLATILE_IMAGE_DECODED_LENGTH)
        );
        assert_eq!(begin.total_length, 24);
        let wire = device.wire_bytes(&frame.digest);
        assert_eq!(&wire[..12], &frame.bytes[..12]);
        assert_eq!(
            protocol::decode_rle565(&wire[12..], frame.bytes.len() - 12).unwrap(),
            &frame.bytes[12..]
        );
        assert!(device.committed.contains(&frame.digest));
        assert_eq!(device.last_release(), Some(vec![frame.digest]));
    }

    #[test]
    fn reconcile_durable_keeps_raw_encoding_when_rle_is_unavailable_or_larger() {
        for (name, fill, high_entropy, capabilities) in [
            ("rle-expands", 0xd2, true, CAPABILITY_DURABLE_ASSET_ENCODING),
            ("capability-absent", 0xd3, false, CAPABILITY_VOLATILE_ASSETS),
        ] {
            let mut device = FakeDevice::new();
            let frame = raster_frame([fill; ASSET_DIGEST_LEN], high_entropy);

            AssetSync::reconcile(&mut device, std::slice::from_ref(&frame), capabilities)
                .unwrap_or_else(|error| panic!("case {name} failed to reconcile: {error}"));

            let begin = *device.begins().first().expect(name);
            assert_eq!(begin.encoding, ASSET_ENCODING_RAW, "{name}");
            assert_eq!(begin.decoded_length, None, "{name}");
            assert_eq!(
                begin.total_length,
                protocol::VOLATILE_IMAGE_DECODED_LENGTH,
                "{name}"
            );
            assert_eq!(
                device.wire_bytes(&frame.digest),
                frame.bytes.as_ref(),
                "{name}"
            );
        }
    }

    #[test]
    fn reconcile_durable_rle_uses_fewer_chunks_than_raw_for_flat_colour_frame() {
        let frame = raster_frame([0xd4; ASSET_DIGEST_LEN], false);
        let mut rle_device = FakeDevice::new();
        let mut raw_device = FakeDevice::new();

        AssetSync::reconcile(
            &mut rle_device,
            std::slice::from_ref(&frame),
            CAPABILITY_DURABLE_ASSET_ENCODING,
        )
        .expect("RLE durable reconcile");
        AssetSync::reconcile(
            &mut raw_device,
            std::slice::from_ref(&frame),
            CAPABILITY_VOLATILE_ASSETS,
        )
        .expect("raw durable reconcile");

        let rle_chunks = rle_device.chunks_sent_for(&frame.digest);
        let raw_chunks = raw_device.chunks_sent_for(&frame.digest);
        assert_eq!(rle_chunks, 1);
        assert_eq!(raw_chunks, 172);
        assert!(rle_chunks < raw_chunks);
    }
}
