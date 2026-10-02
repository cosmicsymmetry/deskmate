//! Replacement occupancy matters as well as the size of the desired set.
#[allow(clippy::wildcard_imports)]
use super::*;

/// Reclaim before any upload when the pool can run out of replacement slots.
/// Keep the last accepted scene alive, even if its digest is no longer desired.
pub(super) fn prepare_asset_transfer(
    state: &mut WorkerState,
    device: &mut dyn RuntimeDevice,
    desired: &[DesiredAsset],
) -> Result<(), AssetSyncError> {
    let capacity = state
        .device
        .volatile_assets
        .map_or(16, |pool| pool.slot_capacity as usize);
    let pressed = volatile_pool_is_pressed(state)
        || state.confirmed_resident_assets.len() + 1 >= capacity.saturating_sub(2)
        || desired.len() >= capacity.saturating_sub(1);
    if state.device.capability_bits() & protocol::CAPABILITY_VOLATILE_ASSETS != 0 && pressed {
        release_obsolete(state, device, desired)?;
    }
    Ok(())
}

fn release_obsolete(
    state: &mut WorkerState,
    device: &mut dyn RuntimeDevice,
    desired: &[DesiredAsset],
) -> Result<(), AssetSyncError> {
    let mut keep: BTreeSet<_> = desired.iter().map(|asset| asset.digest).collect();
    keep.extend(&state.live_scene_assets);
    if keep.len() > protocol::MAX_ASSET_DIGESTS {
        return Err(AssetSyncError::TooManyDesiredAssets {
            desired: keep.len(),
            maximum: protocol::MAX_ASSET_DIGESTS,
        });
    }
    device
        .send_asset_release(AssetRelease {
            digests: keep.iter().copied().collect(),
        })
        .map_err(|source| AssetSyncError::Release { source })?;
    // A keep-set is not evidence that every requested digest is already present.
    state
        .confirmed_resident_assets
        .retain(|digest| keep.contains(digest));
    state.last_asset_release = Some(Instant::now());
    Ok(())
}

pub(super) fn reconcile_assets(
    state: &mut WorkerState,
    device: &mut dyn RuntimeDevice,
    desired: &[DesiredAsset],
    release: bool,
) -> Result<Vec<[u8; protocol::ASSET_DIGEST_LEN]>, AssetSyncError> {
    prepare_asset_transfer(state, device, desired)?;
    let installed =
        AssetSync::reconcile_releasing(device, desired, state.device.capability_bits(), false)?;
    state.confirmed_resident_assets.extend(installed);
    if release {
        release_obsolete(state, device, desired)?;
    }
    Ok(state.confirmed_resident_assets.iter().copied().collect())
}
