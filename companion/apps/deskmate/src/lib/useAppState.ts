import { useCallback, useEffect, useRef, useState } from "react";

import { getAppSnapshot, listenToAppState, toIpcError } from "./tauri";
import type { AppSnapshot, IpcError } from "./types";

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
}

export function useAppState(): AppStateValue {
  const [snapshot, setSnapshot] = useState<AppSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<IpcError | null>(null);
  const [dataGeneration, setDataGeneration] = useState(0);
  const lastCardDataRef = useRef<string | null>(null);

  const acceptSnapshot = useCallback((next: AppSnapshot) => {
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

  return { snapshot, loading, error, refresh, dataGeneration };
}
