import { useCallback, useEffect, useRef, useState } from "react";

import {
  DeskmateCommandError,
  factoryResetDevice,
  getAppSnapshot,
  getNetworkSettings,
  listenToAppState,
  provisionDevice,
  saveApplyConfig,
  saveServerConfig,
  setServerEndpoint,
  toIpcError,
  chooseLocalOwnership,
} from "./tauri";
import type {
  AppConfig,
  AppSnapshot,
  ConfigApplyResult,
  DeviceTier,
  IpcError,
  NetworkSettings,
  ProvisionDeviceInput,
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
  onError: (error: IpcError) => void;
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
      options.onError(toIpcError(error));
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
  error: IpcError | null;
  refresh: () => Promise<void>;
  /**
   * Bumped whenever a new snapshot's `card_data` differs from the previous one (a
   * provider refresh, a pomodoro tick that changed a published field, and so on).
   * `DevicePreview` depends on this rather than on `card_data` itself, because the
   * runtime pushes a freshly-deserialized `AppSnapshot` on every tick even when
   * nothing in it actually changed — comparing object identity would re-request a
   * preview render every tick, and comparing the whole snapshot would miss nothing
   * changing except, say, `diagnostics`. Counting real `card_data` changes gives the
   * preview a signal that fires exactly when the pixels it would render could differ.
   */
  dataGeneration: number;
  networkSettings: NetworkSettings;
  ownershipTier: DeviceTier | null;
  saveConfig: (config: AppConfig) => Promise<ConfigApplyResult>;
  saveServerAccess: (serverUrl: string, adminToken: string) => Promise<void>;
  pairDevice: (input: PairDeviceInput) => Promise<void>;
  unpairDevice: () => Promise<void>;
  factoryReset: () => Promise<void>;
  chooseLocalMode: () => Promise<void>;
}

export interface PairDeviceInput extends ProvisionDeviceInput {
  admin_token: string;
}

interface ConfigSaveDestinations {
  local: () => Promise<ConfigApplyResult>;
  server: () => Promise<ConfigApplyResult>;
}

/** The ownership guard: one draft, but exactly one write destination. */
export function saveConfigForTier(
  tier: DeviceTier | null,
  destinations: ConfigSaveDestinations,
): Promise<ConfigApplyResult> {
  if (tier === "networked") {
    return destinations.server();
  }
  if (tier === "local") {
    return destinations.local();
  }
  throw new DeskmateCommandError({
    category: "invalid-payload",
    message: "Display ownership is unavailable. Connect over USB before saving.",
  });
}

/** Live ownership wins; otherwise use persisted ownership and conservatively treat
 * any legacy server identity as networked so an unplugged save never hits USB. */
export function resolveDeviceTier(
  liveTier: DeviceTier | null,
  settings: NetworkSettings,
): DeviceTier | null {
  if (liveTier) {
    return liveTier;
  }
  if (settings.tier) {
    return settings.tier;
  }
  return settings.server_url || settings.device_id ? "networked" : "local";
}

export function useAppState(): AppStateValue {
  const [snapshot, setSnapshot] = useState<AppSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<IpcError | null>(null);
  const [dataGeneration, setDataGeneration] = useState(0);
  const [networkSettings, setNetworkSettings] = useState<NetworkSettings>({
    server_url: "",
    device_id: "",
    tier: null,
  });
  const [networkSettingsLoaded, setNetworkSettingsLoaded] = useState(false);
  const lastCardDataRef = useRef<string | null>(null);
  const snapshotRef = useRef<AppSnapshot | null>(null);
  const networkSettingsRef = useRef(networkSettings);
  const networkSettingsLoadedRef = useRef(false);

  const acceptNetworkSettings = useCallback((next: NetworkSettings) => {
    networkSettingsRef.current = next;
    networkSettingsLoadedRef.current = true;
    setNetworkSettings(next);
    setNetworkSettingsLoaded(true);
  }, []);
  const acceptError = useCallback((next: IpcError) => {
    setLoading(false);
    setError(next);
  }, []);

  const acceptSnapshot = useCallback(
    (next: AppSnapshot) => {
      snapshotRef.current = next;
      setSnapshot(next);
      if (next.device.tier && networkSettingsRef.current.tier !== next.device.tier) {
        // `project_snapshot` persists a newly observed tier before emitting it. Read
        // that store back instead of copying the live value: immediately after a
        // provision/reset the cable can still report its pre-reboot tier briefly,
        // while the persisted requested ownership is already the routing truth.
        void getNetworkSettings()
          .then(acceptNetworkSettings)
          .catch((error) => acceptError(toIpcError(error)));
      }
      setLoading(false);
      setError(null);
      const serializedCardData = JSON.stringify(next.card_data);
      if (serializedCardData !== lastCardDataRef.current) {
        lastCardDataRef.current = serializedCardData;
        setDataGeneration((current) => current + 1);
      }
    },
    [acceptError, acceptNetworkSettings],
  );
  const refresh = useCallback(async () => {
    try {
      acceptSnapshot(await getAppSnapshot());
    } catch (next) {
      acceptError(toIpcError(next));
    }
  }, [acceptError, acceptSnapshot]);

  const saveServerAccess = useCallback(
    async (serverUrl: string, adminToken: string) => {
      const settings = await setServerEndpoint(serverUrl, adminToken);
      acceptNetworkSettings(settings);
    },
    [acceptNetworkSettings],
  );

  const pairDevice = useCallback(
    async (input: PairDeviceInput) => {
      const settings = await provisionDevice({
        ssid: input.ssid,
        passphrase: input.passphrase,
        server_url: input.server_url,
        device_id: input.device_id,
        device_token: input.device_token,
        tier: input.tier,
      });
      acceptNetworkSettings(settings);
      await saveServerAccess(input.server_url, input.admin_token);
    },
    [acceptNetworkSettings, saveServerAccess],
  );

  const unpairDevice = useCallback(async () => {
    const settings = await provisionDevice({
      ssid: "",
      passphrase: "",
      server_url: networkSettingsRef.current.server_url,
      device_id: networkSettingsRef.current.device_id,
      device_token: "",
      tier: "local",
    });
    acceptNetworkSettings(settings);
  }, [acceptNetworkSettings]);

  const factoryReset = useCallback(async () => {
    await factoryResetDevice();
    acceptNetworkSettings(await getNetworkSettings());
  }, [acceptNetworkSettings]);

  const chooseLocalMode = useCallback(async () => {
    acceptNetworkSettings(await chooseLocalOwnership());
  }, [acceptNetworkSettings]);

  const saveConfig = useCallback(async (config: AppConfig) => {
    const liveTier = snapshotRef.current?.device.tier ?? null;
    const tier =
      liveTier ??
      networkSettingsRef.current.tier ??
      (networkSettingsLoadedRef.current
        ? resolveDeviceTier(null, networkSettingsRef.current)
        : null);
    return saveConfigForTier(tier, {
      local: () => saveApplyConfig(config),
      server: () => {
        const settings = networkSettingsRef.current;
        if (!settings.server_url || !settings.device_id) {
          throw new DeskmateCommandError({
            category: "invalid-payload",
            message: "Enter the server URL and device ID in Network setup before saving.",
          });
        }
        return saveServerConfig(config);
      },
    });
  }, []);

  useEffect(
    () =>
      startAppStateSubscription({
        fetchSnapshot: getAppSnapshot,
        listen: listenToAppState,
        onSnapshot: acceptSnapshot,
        onError: acceptError,
        focusTarget: window,
        visibilityTarget: document,
      }),
    [acceptError, acceptSnapshot],
  );

  useEffect(() => {
    let active = true;
    void getNetworkSettings()
      .then((settings) => {
        if (active) {
          const liveTier = snapshotRef.current?.device.tier ?? null;
          acceptNetworkSettings(liveTier ? { ...settings, tier: liveTier } : settings);
        }
      })
      .catch((next) => {
        if (active) {
          acceptError(toIpcError(next));
        }
      });
    return () => {
      active = false;
    };
  }, [acceptError, acceptNetworkSettings]);

  const liveTier = snapshot?.device.tier ?? null;
  const ownershipTier =
    liveTier ??
    networkSettings.tier ??
    (networkSettingsLoaded ? resolveDeviceTier(null, networkSettings) : null);

  return {
    snapshot,
    loading,
    error,
    refresh,
    dataGeneration,
    networkSettings,
    ownershipTier,
    saveConfig,
    saveServerAccess,
    pairDevice,
    unpairDevice,
    factoryReset,
    chooseLocalMode,
  };
}
