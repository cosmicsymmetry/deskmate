//! Server-side asset transfer driver over the `RuntimeDevice` seam.
//!
//! Server-owned WebSocket devices support asset transfer through the same
//! `RuntimeDevice` methods as other runtime devices, so
//! [`AssetSync::reconcile_with_active_volatile`] can reconcile the complete
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
    AssetCommit, AssetKind, AssetRelease, CAPABILITY_DURABLE_ASSET_ENCODING, MAX_ASSET_CHUNK_BYTES,
    MAX_ASSET_DIGESTS, encode_rle565,
};

/// One asset resolved to bytes and ready to stream: the digest both sides
/// address it by, its wire kind, and the payload itself.
///
/// For a raster frame, `bytes` is always the decoded canonical LVGL blob and
/// `digest` hashes those decoded bytes. Volatile transfer encoding is chosen
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

/// Compose the one device-wide `AssetRelease` keep-set without sending it.
/// Every desired durable digest is followed by the active volatile
/// raster digest, deduplicated and bounded by the wire ceiling.
fn compose_asset_keep_set(
    durable: &[[u8; ASSET_DIGEST_LEN]],
    active_volatile: Option<[u8; ASSET_DIGEST_LEN]>,
) -> Result<Vec<[u8; ASSET_DIGEST_LEN]>, AssetSyncError> {
    let mut keep = Vec::with_capacity(durable.len() + usize::from(active_volatile.is_some()));
    for digest in durable.iter().copied().chain(active_volatile) {
        if !keep.contains(&digest) {
            keep.push(digest);
        }
    }
    if keep.len() > MAX_ASSET_DIGESTS {
        return Err(AssetSyncError::TooManyDesiredAssets {
            desired: keep.len(),
            maximum: MAX_ASSET_DIGESTS,
        });
    }
    Ok(keep)
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

    fn transfer_one(
        device: &mut dyn RuntimeDevice,
        asset: &DesiredAsset,
        volatile: bool,
        wire_bytes: &[u8],
        encoding: u8,
        decoded_length: Option<u32>,
        on_chunk_sent: &mut dyn FnMut(),
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
        Ok(())
    }

    /// Reconciles durable assets while retaining the volatile digest read by
    /// the currently displayed scene, and returns the exact keep-set sent to
    /// the device. This is the only safe full-sync shape before its
    /// replacement `PushScene` succeeds.
    pub(crate) fn reconcile_with_active_volatile(
        device: &mut dyn RuntimeDevice,
        desired: &[DesiredAsset],
        active_volatile: Option<[u8; ASSET_DIGEST_LEN]>,
        capabilities: u64,
    ) -> Result<Vec<[u8; ASSET_DIGEST_LEN]>, AssetSyncError> {
        Self::reconcile_with_active_volatile_yielding(
            device,
            desired,
            active_volatile,
            capabilities,
            &mut || {},
        )
    }

    /// Same as [`Self::reconcile_with_active_volatile`], but calls
    /// `on_chunk_sent` after every chunk the device accepts.
    ///
    /// Protocol v1 allows exactly one outstanding request per connection
    /// (spec §4's "one outstanding plus preemption", chosen deliberately
    /// over a windowed transfer), and one asset can be on the order of 250
    /// sequential chunk round trips. `RuntimeDevice`'s methods are
    /// synchronous by design (see `runtime_device.rs`'s own doc comment on
    /// why: it avoids entering a Tokio runtime from a synchronous trait), so
    /// there is no `.await` point here for interactive traffic to preempt
    /// implicitly. `on_chunk_sent` is the explicit substitute: a caller that
    /// embeds this operation inside a command loop (the runtime
    /// worker's, or the WebSocket actor's) is expected to use this hook to
    /// service one pending non-asset command -- a tap, a heartbeat, a
    /// pomodoro tick -- before the next chunk claims the connection's one
    /// outstanding-request slot again. The non-yielding wrapper passes a
    /// no-op hook, which is correct for a caller that owns the device
    /// exclusively for the duration of the pass.
    fn reconcile_with_active_volatile_yielding(
        device: &mut dyn RuntimeDevice,
        desired: &[DesiredAsset],
        active_volatile: Option<[u8; ASSET_DIGEST_LEN]>,
        capabilities: u64,
        on_chunk_sent: &mut dyn FnMut(),
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
                on_chunk_sent,
            )?;
        }

        let durable: Vec<[u8; ASSET_DIGEST_LEN]> =
            desired.iter().map(|asset| asset.digest).collect();
        let keep_set = compose_asset_keep_set(&durable, active_volatile)?;
        device
            .send_asset_release(AssetRelease {
                digests: keep_set.clone(),
            })
            .map_err(|source| AssetSyncError::Release { source })?;

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
    fn reconcile_skips_assets_the_device_already_holds() {
        let mut device = FakeDevice::new().with_already_present([0xaa; ASSET_DIGEST_LEN]);
        let desired = vec![
            asset_blob([0xaa; ASSET_DIGEST_LEN], 4096),
            asset_blob([0xbb; ASSET_DIGEST_LEN], 2048),
        ];

        let keep_set = AssetSync::reconcile_with_active_volatile(&mut device, &desired, None, 0)
            .expect("reconcile");

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

        let first_attempt =
            AssetSync::reconcile_with_active_volatile(&mut device, &desired, None, 0);
        assert!(
            first_attempt.is_err(),
            "the injected failure must actually fire"
        );
        // Two chunks were accepted before the third failed.
        assert_eq!(device.chunks_sent_for(&digest), 2);

        device.recovered();
        let keep_set = AssetSync::reconcile_with_active_volatile(&mut device, &desired, None, 0)
            .expect("the retry must succeed");

        // The retry sent all 5 chunks again, not just the remaining 3 --
        // proving it restarted rather than resuming from offset 2 * CHUNK.
        assert_eq!(device.chunks_sent_for(&digest), 5);
        assert_eq!(keep_set, vec![digest]);
        assert_eq!(device.last_release(), Some(keep_set));
    }

    #[test]
    fn reconcile_with_active_volatile_yielding_calls_the_hook_once_per_chunk() {
        let mut device = FakeDevice::new();
        let desired = vec![asset_blob(
            [0xee; ASSET_DIGEST_LEN],
            MAX_ASSET_CHUNK_BYTES * 3,
        )];
        let mut yields = 0usize;

        AssetSync::reconcile_with_active_volatile_yielding(
            &mut device,
            &desired,
            None,
            0,
            &mut || yields += 1,
        )
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

        let error = AssetSync::reconcile_with_active_volatile(&mut device, &desired, None, 0)
            .expect_err("must be rejected");

        assert!(matches!(error, AssetSyncError::TooManyDesiredAssets { .. }));
        assert!(device.last_release().is_none());
    }

    #[test]
    fn reconcile_durable_uses_rle_when_it_is_smaller() {
        let mut device = FakeDevice::new();
        let frame = raster_frame([0xd1; ASSET_DIGEST_LEN], false);

        AssetSync::reconcile_with_active_volatile(
            &mut device,
            std::slice::from_ref(&frame),
            None,
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
    fn reconcile_durable_uses_raw_when_rle_expands() {
        let mut device = FakeDevice::new();
        let frame = raster_frame([0xd2; ASSET_DIGEST_LEN], true);

        AssetSync::reconcile_with_active_volatile(
            &mut device,
            std::slice::from_ref(&frame),
            None,
            CAPABILITY_DURABLE_ASSET_ENCODING,
        )
        .expect("durable reconcile");

        let begin = device.begins()[0];
        assert_eq!(begin.encoding, ASSET_ENCODING_RAW);
        assert_eq!(begin.decoded_length, None);
        assert_eq!(begin.total_length, protocol::VOLATILE_IMAGE_DECODED_LENGTH);
        assert_eq!(device.wire_bytes(&frame.digest), frame.bytes.as_ref());
    }

    #[test]
    fn reconcile_durable_without_bit_10_keeps_raw_encoding() {
        let mut device = FakeDevice::new();
        let frame = raster_frame([0xd3; ASSET_DIGEST_LEN], false);

        AssetSync::reconcile_with_active_volatile(
            &mut device,
            std::slice::from_ref(&frame),
            None,
            CAPABILITY_VOLATILE_ASSETS,
        )
        .expect("durable reconcile");

        let begin = device.begins()[0];
        assert_eq!(begin.encoding, ASSET_ENCODING_RAW);
        assert_eq!(begin.decoded_length, None);
        assert_eq!(device.wire_bytes(&frame.digest), frame.bytes.as_ref());
    }

    #[test]
    fn reconcile_durable_rle_uses_fewer_chunks_than_raw_for_flat_colour_frame() {
        let frame = raster_frame([0xd4; ASSET_DIGEST_LEN], false);
        let mut rle_device = FakeDevice::new();
        let mut raw_device = FakeDevice::new();

        AssetSync::reconcile_with_active_volatile(
            &mut rle_device,
            std::slice::from_ref(&frame),
            None,
            CAPABILITY_DURABLE_ASSET_ENCODING,
        )
        .expect("RLE durable reconcile");
        AssetSync::reconcile_with_active_volatile(
            &mut raw_device,
            std::slice::from_ref(&frame),
            None,
            CAPABILITY_VOLATILE_ASSETS,
        )
        .expect("raw durable reconcile");

        let rle_chunks = rle_device.chunks_sent_for(&frame.digest);
        let raw_chunks = raw_device.chunks_sent_for(&frame.digest);
        assert_eq!(rle_chunks, 1);
        assert_eq!(raw_chunks, 172);
        assert!(rle_chunks < raw_chunks);
    }

    #[test]
    fn compose_asset_keep_set_preserves_an_empty_keep_set() {
        assert_eq!(
            compose_asset_keep_set(&[], None).unwrap(),
            Vec::<[u8; ASSET_DIGEST_LEN]>::new()
        );
    }

    #[test]
    fn compose_asset_keep_set_preserves_durable_order() {
        let durable_a = [0xa1; ASSET_DIGEST_LEN];
        let durable_b = [0xa2; ASSET_DIGEST_LEN];
        assert_eq!(
            compose_asset_keep_set(&[durable_a, durable_b], None).unwrap(),
            vec![durable_a, durable_b]
        );
    }

    #[test]
    fn compose_asset_keep_set_keeps_an_active_volatile_digest() {
        let raster = [0xb1; ASSET_DIGEST_LEN];
        assert_eq!(
            compose_asset_keep_set(&[], Some(raster)).unwrap(),
            vec![raster]
        );
    }

    #[test]
    fn compose_asset_keep_set_appends_active_volatile_after_durable_assets() {
        let durable_a = [0xa1; ASSET_DIGEST_LEN];
        let durable_b = [0xa2; ASSET_DIGEST_LEN];
        let raster = [0xb1; ASSET_DIGEST_LEN];
        assert_eq!(
            compose_asset_keep_set(&[durable_a, durable_b], Some(raster)).unwrap(),
            vec![durable_a, durable_b, raster]
        );
    }

    #[test]
    fn compose_asset_keep_set_deduplicates_the_active_volatile_digest() {
        let same = [0xc1; ASSET_DIGEST_LEN];
        assert_eq!(
            compose_asset_keep_set(&[same], Some(same)).unwrap(),
            vec![same]
        );
    }

    #[test]
    fn compose_asset_keep_set_enforces_the_wire_ceiling() {
        let durable: Vec<[u8; ASSET_DIGEST_LEN]> = (0..MAX_ASSET_DIGESTS)
            .map(|index| {
                let mut digest = [0u8; ASSET_DIGEST_LEN];
                digest[0] = u8::try_from(index).unwrap();
                digest
            })
            .collect();
        assert!(matches!(
            compose_asset_keep_set(&durable, Some([0xff; ASSET_DIGEST_LEN])),
            Err(AssetSyncError::TooManyDesiredAssets { .. })
        ));
    }

    #[test]
    fn reconcile_releases_the_union_of_durable_and_active_volatile_digests() {
        let durable = asset_blob([0xd5; ASSET_DIGEST_LEN], 16);
        let active_volatile = [0xe5; ASSET_DIGEST_LEN];
        let mut device = FakeDevice::new();

        let keep_set = AssetSync::reconcile_with_active_volatile(
            &mut device,
            std::slice::from_ref(&durable),
            Some(active_volatile),
            0,
        )
        .expect("reconcile");

        assert_eq!(keep_set, vec![durable.digest, active_volatile]);
        assert_eq!(device.last_release(), Some(keep_set));
    }
}
