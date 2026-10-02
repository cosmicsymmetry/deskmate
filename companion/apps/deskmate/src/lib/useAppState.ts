import { useCallback, useEffect, useRef, useState } from "react";

import {
  DeskmateApiError,
  getAppSnapshot,
  getNetworkSettings,
  listenToAppState,
  isSessionMissing,
  saveConfig as saveConfigRequest,
  toApiError,
} from "./backend";
import type {
  AppConfig,
  AppSnapshot,
  ConfigApplyResult,
  DeviceTier,
  ApiError,
  NetworkSettings,
} from "./types";

interface EventTargetLike {
  addEventListener(type: string, listener: EventListener): void;
  removeEventListener(type: string, listener: EventListener): void;
}

interface VisibilityTargetLike extends EventTargetLike {
  readonly visibilityState: string;
}

export interface AppStateSubscriptionOptions {
  fetchSnapshot: () => Promise<AppSnapshot>;
  listen: (onSnapshot: (snapshot: AppSnapshot) => void) => Promise<() => void>;
  onSnapshot: (snapshot: AppSnapshot) => void;
  onError: (error: ApiError) => void;
  focusTarget?: EventTargetLike;
  visibilityTarget?: VisibilityTargetLike;
}

/**
 * Connects the event stream before fetching state, then refreshes on every focus or
 * visible transition. Cleanup also handles the listener promise resolving after an
 * unmount, which is the common reload/StrictMode leak edge case.
 */
export function startAppStateSubscription(options: AppStateSubscriptionOptions): () => void {
  let active = true;
  let unlisten: (() => void) | undefined;

  const publishSnapshot = (snapshot: AppSnapshot) => {
    if (active) {
      options.onSnapshot(snapshot);
    }
  };
  const publishError = (error: unknown) => {
    if (active) {
      options.onError(toApiError(error));
    }
  };
  const refresh = () => {
    void options.fetchSnapshot().then(publishSnapshot).catch(publishError);
  };
  const onFocus: EventListener = () => refresh();
  const onVisibilityChange: EventListener = () => {
    if (options.visibilityTarget?.visibilityState === "visible") {
      refresh();
    }
  };

  options.focusTarget?.addEventListener("focus", onFocus);
  options.visibilityTarget?.addEventListener("visibilitychange", onVisibilityChange);

  void (async () => {
    try {
      const stop = await options.listen(publishSnapshot);
      if (!active) {
        stop();
        return;
      }
      unlisten = stop;
    } catch (error) {
      publishError(error);
    }
    if (active) {
      refresh();
    }
  })();

  return () => {
    active = false;
    options.focusTarget?.removeEventListener("focus", onFocus);
    options.visibilityTarget?.removeEventListener("visibilitychange", onVisibilityChange);
    unlisten?.();
    unlisten = undefined;
  };
}

export interface AppStateValue {
  snapshot: AppSnapshot | null;
  loading: boolean;
  error: ApiError | null;
  refresh: () => Promise<void>;
  /**
   * Increments only when serialized `card_data` changes, so fresh snapshot identity
   * and telemetry-only deliveries cannot trigger identical preview requests.
   */
  dataGeneration: number;
  networkSettings: NetworkSettings;
  ownershipTier: DeviceTier | null;
  saveConfig: (config: AppConfig) => Promise<ConfigApplyResult>;
}

export function useAppState(): AppStateValue {
  const [snapshot, setSnapshot] = useState<AppSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<ApiError | null>(null);
  const [dataGeneration, setDataGeneration] = useState(0);
  const [networkSettings, setNetworkSettings] = useState<NetworkSettings>({
    server_url: "",
    device_id: "",
    tier: null,
  });
  const lastCardDataRef = useRef<string | null>(null);
  const networkSettingsRef = useRef(networkSettings);
  const sessionLost = useRef(false);
  const signedOut = error !== null && isSessionMissing(error);

  const acceptNetworkSettings = useCallback((next: NetworkSettings) => {
    if (sessionLost.current) return;
    networkSettingsRef.current = next;
    setNetworkSettings(next);
  }, []);
  const acceptError = useCallback((next: ApiError) => {
    if (sessionLost.current) return;
    if (isSessionMissing(next)) {
      sessionLost.current = true;
      setSnapshot(null);
      const empty = { server_url: "", device_id: "", tier: null };
      networkSettingsRef.current = empty;
      setNetworkSettings(empty);
    }
    setLoading(false);
    setError(next);
  }, []);

  const acceptSnapshot = useCallback((next: AppSnapshot) => {
    if (sessionLost.current) return;
    setSnapshot(next);
    setLoading(false);
    setError(null);
    const serializedCardData = JSON.stringify(next.card_data);
    if (serializedCardData !== lastCardDataRef.current) {
      lastCardDataRef.current = serializedCardData;
      setDataGeneration((current) => current + 1);
    }
  }, []);
  const refresh = useCallback(async () => {
    try {
      acceptSnapshot(await getAppSnapshot());
      // Network settings are refetched with the snapshot, not only on mount.
      // A panel can be removed or claimed in another tab, and saving must use
      // the account's current panel rather than stale mount-time state.
      acceptNetworkSettings(await getNetworkSettings());
    } catch (next) {
      acceptError(toApiError(next));
    }
  }, [acceptError, acceptNetworkSettings, acceptSnapshot]);

  const saveConfig = useCallback(async (config: AppConfig) => {
    const settings = networkSettingsRef.current;
    if (!settings.device_id) {
      throw new DeskmateApiError({
        category: "invalid-payload",
        message: "Set up a panel before saving.",
      });
    }
    return saveConfigRequest(config);
  }, []);

  useEffect(() => {
    if (signedOut) return;
    return startAppStateSubscription({
      fetchSnapshot: getAppSnapshot,
      listen: listenToAppState,
      onSnapshot: acceptSnapshot,
      onError: acceptError,
      focusTarget: window,
      visibilityTarget: document,
    });
  }, [acceptError, acceptSnapshot, signedOut]);

  useEffect(() => {
    let active = true;
    void getNetworkSettings()
      .then((settings) => {
        if (active) {
          acceptNetworkSettings(settings);
        }
      })
      .catch((next) => {
        if (active) {
          acceptError(toApiError(next));
        }
      });
    return () => {
      active = false;
    };
  }, [acceptError, acceptNetworkSettings]);

  const liveTier = snapshot?.device.tier ?? null;
  const ownershipTier = liveTier ?? networkSettings.tier;

  return {
    snapshot,
    loading,
    error,
    refresh,
    dataGeneration,
    networkSettings,
    ownershipTier,
    saveConfig,
  };
}
