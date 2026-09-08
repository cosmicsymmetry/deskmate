import { describe, expect, test } from "bun:test";

import {
  startAppStateSubscription,
  startServerCardStatePoll,
  type AppStateSubscriptionOptions,
} from "../src/lib/useAppState";
import type { AppSnapshot, IpcError, ServerCardState } from "../src/lib/types";
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
  test("the subscription fixture carries schema-v6 playlists", () => {
    expect(snapshot.config.schema_version).toBe(6);
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

describe("startServerCardStatePoll", () => {
  /** A scheduler whose `clear` really stops the handler, so "after stop" means it. */
  function fakeScheduler() {
    const handlers = new Map<number, () => void>();
    let nextHandle = 0;
    let cleared = 0;
    return {
      scheduler: {
        set: (handler: () => void) => {
          nextHandle += 1;
          handlers.set(nextHandle, handler);
          return nextHandle;
        },
        clear: (handle: number) => {
          cleared += 1;
          handlers.delete(handle);
        },
      },
      tick: () => {
        for (const handler of [...handlers.values()]) {
          handler();
        }
      },
      cleared: () => cleared,
    };
  }

  test("polls on start, on focus and on each tick, but never while hidden", async () => {
    const focusTarget = new FakeEventTarget();
    const visibilityTarget = new FakeEventTarget();
    const { scheduler, tick, cleared } = fakeScheduler();
    const accepted: ServerCardState[][] = [];
    let fetches = 0;

    const stop = startServerCardStatePoll({
      fetchCardState: async () => {
        fetches += 1;
        return [];
      },
      onCardState: (state) => accepted.push(state),
      onError: () => {},
      focusTarget,
      visibilityTarget,
      scheduler,
    });
    await flushPromises();
    expect(fetches).toBe(1);
    expect(accepted).toHaveLength(1);

    focusTarget.dispatch("focus");
    tick();
    await flushPromises();
    expect(fetches).toBe(3);

    visibilityTarget.visibilityState = "hidden";
    tick();
    focusTarget.dispatch("focus");
    await flushPromises();
    expect(fetches).toBe(3);

    visibilityTarget.visibilityState = "visible";
    visibilityTarget.dispatch("visibilitychange");
    await flushPromises();
    expect(fetches).toBe(4);

    stop();
    tick();
    focusTarget.dispatch("focus");
    await flushPromises();
    expect(fetches).toBe(4);
    expect(cleared()).toBe(1);
    expect(focusTarget.count("focus")).toBe(0);
    expect(visibilityTarget.count("visibilitychange")).toBe(0);
  });

  test("a failed poll is reported as a typed error and never as a card state", async () => {
    const { scheduler } = fakeScheduler();
    const errors: IpcError[] = [];
    const stop = startServerCardStatePoll({
      fetchCardState: async () => {
        throw { category: "runtime-unavailable", message: "no runtime for this device" };
      },
      onCardState: () => {
        throw new Error("a failed poll must not publish a card state");
      },
      onError: (error) => errors.push(error),
      scheduler,
    });
    await flushPromises();
    expect(errors).toEqual([
      { category: "runtime-unavailable", message: "no runtime for this device" },
    ]);
    stop();
  });
});
