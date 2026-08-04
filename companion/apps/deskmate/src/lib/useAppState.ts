import { useCallback, useEffect, useState } from "react";

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
}

export function useAppState(): AppStateValue {
  const [snapshot, setSnapshot] = useState<AppSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<IpcError | null>(null);

  const acceptSnapshot = useCallback((next: AppSnapshot) => {
    setSnapshot(next);
    setLoading(false);
    setError(null);
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

  return { snapshot, loading, error, refresh };
}
