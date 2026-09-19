import { afterEach, beforeEach, expect, test } from "bun:test";

import {
  resetBackendMocks,
  realMintImageSource,
  realListenToAppState,
  realSignInAndSelectDevice,
  realSignIn,
} from "./support/backendMock";
import { installDomLifecycle } from "./support/dom";
import {
  installHttpLifecycle,
  httpCalls,
  allowImageMint,
  createHttpMock,
  httpHandlers,
  expectJsonRequest,
} from "./support/http";

beforeEach(resetBackendMocks);
const { cleanupMountedRoots } = installDomLifecycle();
installHttpLifecycle(cleanupMountedRoots);
afterEach(resetBackendMocks);

test("minting a picture source makes a server round trip with the source name", async () => {
  allowImageMint("Picture");

  // The translation is the point: the route says `id`, the UI contract needs
  // `source_id`, and `push_url` is derived from this page's own origin.
  expect(await realMintImageSource("Picture")).toEqual({
    source_id: "picture-source",
    token: "plaintext-once",
    push_url: `${window.location.origin}/v1/images/plaintext-once`,
  });
  // faceKind rides along on every mint: null for an external producer, a kind
  // when the owner picked a server-drawn face from the menu.
  expect(httpCalls).toEqual([
    { method: "POST", path: "/v1/images", body: { name: "Picture", face_kind: null } },
  ]);
});

test("HTTP teardown detects an unexpected request even when its rejection is caught", async () => {
  const http = createHttpMock();
  await http.fetch("/unexpected").catch(() => {});
  expect(http.failures).toHaveLength(1);
  expect(() => http.assertNoFailures()).toThrow();
});

test("HTTP teardown records handler assertions and JSON parsing failures", async () => {
  const http = createHttpMock();
  http.handlers.set("POST /expected", () => {
    expect("wrong body").toBe("expected body");
    return new Response(null, { status: 204 });
  });
  await http.fetch("/expected", { method: "POST", body: "{}" }).catch(() => {});
  await http.fetch("/expected", { method: "POST", body: "{" }).catch(() => {});
  expect(http.failures).toHaveLength(2);
  expect(() => http.assertNoFailures()).toThrow();
});

test("real event streams preserve selection, credentials, parsing and stale-source cleanup", async () => {
  const { startAppStateSubscription } = await import("../src/lib/useAppState");
  const { snapshot } = await import("./support/fixtures");
  const sources: FakeEventSource[] = [];
  class FakeEventSource extends EventTarget {
    closes = 0;
    constructor(
      readonly url: string,
      readonly options: EventSourceInit,
    ) {
      super();
      sources.push(this);
    }
    close() {
      this.closes += 1;
    }
    frame(data: string, type = "app-state") {
      this.dispatchEvent(new MessageEvent(type, { data }));
    }
  }
  const original = globalThis.EventSource;
  const remembered = window.localStorage.getItem("deskmate.device_id");
  globalThis.EventSource = FakeEventSource as unknown as typeof EventSource;
  httpHandlers.set("POST /v1/app/session", (body, init) => {
    expectJsonRequest(init);
    expect(body).toEqual({ token: "session-token" });
    return new Response(null, { status: 204 });
  });
  const accepted: string[] = [];
  const stops: (() => void)[] = [];
  const settle = async () => {
    for (let step = 0; step < 8; step += 1) await Promise.resolve();
  };
  const subscribe = () => {
    const stop = startAppStateSubscription({
      listen: realListenToAppState,
      fetchSnapshot: async () => {
        expect(sources.length).toBeGreaterThan(0);
        return snapshot;
      },
      onSnapshot: (next) => accepted.push(next.config.preferences.timezone),
      onError: (error) => {
        throw new Error(error.message);
      },
    });
    stops.push(stop);
    return stop;
  };
  try {
    await realSignInAndSelectDevice("desk A/α", "session-token");
    await realSignIn("session-token");
    const stopA = subscribe();
    await settle();
    expect(sources[0].url).toBe("/v1/app/desk%20A%2F%CE%B1/events");
    expect(sources[0].options).toEqual({ withCredentials: true });
    accepted.length = 0;
    sources[0].frame("malformed");
    sources[0].frame(JSON.stringify(snapshot), "message");
    expect(accepted).toEqual([]);
    sources[0].frame(JSON.stringify(snapshot));
    expect(accepted).toEqual([snapshot.config.preferences.timezone]);

    await realSignInAndSelectDevice("desk-B", "session-token");
    stopA();
    subscribe();
    await settle();
    expect(sources[0].closes).toBe(1);
    expect(sources[1].url).toBe("/v1/app/desk-B/events");
    expect(sources[1].options).toEqual({ withCredentials: true });
    accepted.length = 0;
    sources[0].frame(JSON.stringify(snapshot));
    expect(accepted).toEqual([]);
    sources[1].frame(JSON.stringify(snapshot));
    expect(accepted).toHaveLength(1);

    const stopPending = subscribe();
    stopPending();
    await settle();
    expect(sources[2].closes).toBe(1);
    accepted.length = 0;
    sources[2].frame(JSON.stringify(snapshot));
    expect(accepted).toEqual([]);
  } finally {
    for (const stop of stops) stop();
    globalThis.EventSource = original;
    if (remembered === null) window.localStorage.removeItem("deskmate.device_id");
    else window.localStorage.setItem("deskmate.device_id", remembered);
  }
});
