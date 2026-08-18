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
  saveConfig: (config: AppConfig) => Promise<ConfigApplyResult>;
  saveServerAccess: (serverUrl: string, adminToken: string) => Promise<void>;
  pairDevice: (input: PairDeviceInput) => Promise<void>;
  unpairDevice: () => Promise<void>;
  factoryReset: () => Promise<void>;
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
  return tier === "networked" ? destinations.server() : destinations.local();
}

export function useAppState(): AppStateValue {
  const [snapshot, setSnapshot] = useState<AppSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<IpcError | null>(null);
  const [dataGeneration, setDataGeneration] = useState(0);
  const [networkSettings, setNetworkSettings] = useState<NetworkSettings>({
    server_url: "",
    device_id: "",
  });
  const lastCardDataRef = useRef<string | null>(null);
  const snapshotRef = useRef<AppSnapshot | null>(null);
  const networkSettingsRef = useRef(networkSettings);
  const adminTokenRef = useRef<string | null>(null);

  const acceptSnapshot = useCallback((next: AppSnapshot) => {
    snapshotRef.current = next;
    setSnapshot(next);
    setLoading(false);
    setError(null);
    const serializedCardData = JSON.stringify(next.card_data);
    if (serializedCardData !== lastCardDataRef.current) {
      lastCardDataRef.current = serializedCardData;
      setDataGeneration((current) => current + 1);
    }
  }, []);
  const acceptError = useCallback((next: IpcError) => {
    setLoading(false);
    setError(next);
  }, []);
  const refresh = useCallback(async () => {
    try {
      acceptSnapshot(await getAppSnapshot());
    } catch (next) {
      acceptError(toIpcError(next));
    }
  }, [acceptError, acceptSnapshot]);

  const acceptNetworkSettings = useCallback((next: NetworkSettings) => {
    networkSettingsRef.current = next;
    setNetworkSettings(next);
  }, []);

  const saveServerAccess = useCallback(
    async (serverUrl: string, adminToken: string) => {
      const settings = await setServerEndpoint(serverUrl, adminToken);
      adminTokenRef.current = adminToken;
      acceptNetworkSettings(settings);
    },
    [acceptNetworkSettings],
  );

  const pairDevice = useCallback(
    async (input: PairDeviceInput) => {
      await saveServerAccess(input.server_url, input.admin_token);
      const settings = await provisionDevice({
        ssid: input.ssid,
        passphrase: input.passphrase,
        server_url: input.server_url,
        device_id: input.device_id,
        device_token: input.device_token,
        tier: input.tier,
      });
      acceptNetworkSettings(settings);
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
  }, []);

  const saveConfig = useCallback(async (config: AppConfig) => {
    const tier = snapshotRef.current?.device.tier ?? null;
    return saveConfigForTier(tier, {
      local: () => saveApplyConfig(config),
      server: () => {
        const settings = networkSettingsRef.current;
        const adminToken = adminTokenRef.current;
        if (!settings.server_url || !settings.device_id || !adminToken) {
          throw new DeskmateCommandError({
            category: "invalid-payload",
            message:
              "Enter the server URL, device ID, and admin token in Network setup before saving.",
          });
        }
        return saveServerConfig(config, settings, adminToken);
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
          acceptNetworkSettings(settings);
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

  return {
    snapshot,
    loading,
    error,
    refresh,
    dataGeneration,
    networkSettings,
    saveConfig,
    saveServerAccess,
    pairDevice,
    unpairDevice,
    factoryReset,
  };
}
