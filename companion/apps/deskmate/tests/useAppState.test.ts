import { describe, expect, test } from "bun:test";

import {
  startAppStateSubscription,
  type AppStateSubscriptionOptions,
} from "../src/lib/useAppState";
import type { AppSnapshot } from "../src/lib/types";
import { ipcContractFixtures } from "../src/lib/types.contract";

class FakeEventTarget {
  visibilityState = "visible";
  private readonly listeners = new Map<string, Set<EventListener>>();

  addEventListener(type: string, listener: EventListener) {
    const listeners = this.listeners.get(type) ?? new Set<EventListener>();
    listeners.add(listener);
    this.listeners.set(type, listeners);
  }

  removeEventListener(type: string, listener: EventListener) {
    this.listeners.get(type)?.delete(listener);
  }

  dispatch(type: string) {
    for (const listener of this.listeners.get(type) ?? []) {
      listener(new Event(type));
    }
  }

  count(type: string) {
    return this.listeners.get(type)?.size ?? 0;
  }
}

const snapshot: AppSnapshot = ipcContractFixtures.snapshot;

async function flushPromises() {
  await Promise.resolve();
  await Promise.resolve();
}

describe("startAppStateSubscription", () => {
  test("the subscription fixture carries schema-v4 playlists", () => {
    expect(snapshot.config.schema_version).toBe(4);
    expect(snapshot.config.playlists.map((playlist) => playlist.id)).toEqual(["workday", "manual"]);
    expect(snapshot.config.active_playlist_id).toBe("workday");
  });

  test("cleans up when the Tauri listener resolves after unmount", async () => {
    const focusTarget = new FakeEventTarget();
    const visibilityTarget = new FakeEventTarget();
    let resolveListen!: (unlisten: () => void) => void;
    let unlistenCalls = 0;
    let fetchCalls = 0;

    const cleanup = startAppStateSubscription({
      fetchSnapshot: async () => {
        fetchCalls += 1;
        return snapshot;
      },
      listen: () =>
        new Promise((resolve) => {
          resolveListen = resolve;
        }),
      onSnapshot: () => {},
      onError: () => {},
      focusTarget,
      visibilityTarget,
    });

    expect(focusTarget.count("focus")).toBe(1);
    expect(visibilityTarget.count("visibilitychange")).toBe(1);
    cleanup();
    resolveListen(() => {
      unlistenCalls += 1;
    });
    await flushPromises();

    expect(unlistenCalls).toBe(1);
    expect(fetchCalls).toBe(0);
    expect(focusTarget.count("focus")).toBe(0);
    expect(visibilityTarget.count("visibilitychange")).toBe(0);
  });

  test("refreshes on focus and ignores events after cleanup", async () => {
    const focusTarget = new FakeEventTarget();
    const visibilityTarget = new FakeEventTarget();
    let eventHandler: ((next: AppSnapshot) => void) | undefined;
    let unlistenCalls = 0;
    let fetchCalls = 0;
    const accepted: AppSnapshot[] = [];
    const errors: unknown[] = [];

    const options: AppStateSubscriptionOptions = {
      fetchSnapshot: async () => {
        fetchCalls += 1;
        return snapshot;
      },
      listen: async (handler) => {
        eventHandler = handler;
        return () => {
          unlistenCalls += 1;
        };
      },
      onSnapshot: (next) => accepted.push(next),
      onError: (error) => errors.push(error),
      focusTarget,
      visibilityTarget,
    };
    const cleanup = startAppStateSubscription(options);
    await flushPromises();

    expect(fetchCalls).toBe(1);
    expect(accepted).toEqual([snapshot]);
    eventHandler?.(snapshot);
    focusTarget.dispatch("focus");
    await flushPromises();
    expect(fetchCalls).toBe(2);
    expect(accepted).toHaveLength(3);

    cleanup();
    eventHandler?.(snapshot);
    focusTarget.dispatch("focus");
    await flushPromises();
    expect(accepted).toHaveLength(3);
    expect(fetchCalls).toBe(2);
    expect(unlistenCalls).toBe(1);
    expect(errors).toEqual([]);
  });
});
