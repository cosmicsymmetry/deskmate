import { afterEach, beforeEach, expect, test } from "bun:test";

import * as account from "../src/lib/account";

import {
  resetBackendMocks,
  realMintImageSource,
  realListenToAppState,
} from "./support/backendMock";
import { installDomLifecycle } from "./support/dom";
import {
  installHttpLifecycle,
  httpCalls,
  allowImageMint,
  createHttpMock,
  httpHandlers,
  expectJsonRequest,
  jsonResponse,
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
    {
      method: "POST",
      path: "/v1/images",
      body: { name: "Picture", face_kind: null },
    },
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

test("account calls use the committed routes and unwrap their response envelopes", async () => {
  const row = {
    id: "account-1",
    email: "owner@example.com",
    email_verified: true,
    is_instance_owner: true,
  };
  httpHandlers.set("GET /v1/app/instance", () =>
    jsonResponse({
      setup_required: false,
      google_enabled: true,
      email_delivery: "email",
      signups_open: true,
      edition: "hosted",
    }),
  );
  httpHandlers.set("POST /v1/app/setup", (body, init) => {
    expectJsonRequest(init);
    expect(body).toEqual({ code: "ABCD-EFGH", email: row.email });
    return jsonResponse({ account: row });
  });
  httpHandlers.set("POST /v1/app/auth/email", (body, init) => {
    expectJsonRequest(init);
    expect(body).toEqual({ email: row.email });
    return new Response("{}", { status: 202 });
  });
  httpHandlers.set("POST /v1/app/auth/link", (body, init) => {
    expectJsonRequest(init);
    expect(body).toEqual({ token: "link-token" });
    return jsonResponse({ account: row });
  });
  httpHandlers.set("GET /v1/app/account", () => jsonResponse(row));
  httpHandlers.set("DELETE /v1/app/session", () => new Response(null, { status: 204 }));
  httpHandlers.set("POST /v1/app/sessions/revoke-all", () => new Response(null, { status: 204 }));
  httpHandlers.set("DELETE /v1/app/account", () => new Response(null, { status: 204 }));
  httpHandlers.set("PUT /v1/app/instance/signups", (body, init) => {
    expectJsonRequest(init);
    expect(body).toEqual({ open: false });
    return jsonResponse({ signups_open: false });
  });
  httpHandlers.set("GET /v1/app/devices", () =>
    jsonResponse([
      {
        id: "desk-1",
        connected: true,
        has_saved_config: true,
        configured_at: 123,
        state: "active",
      },
    ]),
  );
  httpHandlers.set(
    "POST /v1/app/devices/claim",
    () =>
      new Response(
        JSON.stringify({
          device_id: "desk-2",
          token: "once",
          link_url: "wss://desk/link",
        }),
        { status: 201 },
      ),
  );
  httpHandlers.set("DELETE /v1/app/devices/desk%2F2", () => new Response(null, { status: 204 }));

  expect(await account.getInstance()).toMatchObject({
    edition: "hosted",
    google_enabled: true,
  });
  expect(await account.completeSetup("ABCD-EFGH", row.email)).toEqual(row);
  await account.requestSignInLink(row.email);
  expect(await account.consumeSignInLink("link-token")).toEqual(row);
  expect(account.googleSignInUrl()).toBe("/v1/app/auth/google/start");
  expect(await account.getAccount()).toEqual(row);
  await account.signOut();
  await account.signOutEverywhere();
  await account.deleteAccount();
  expect(await account.setSignupsOpen(false)).toBe(false);
  expect(await account.listPanels()).toEqual([
    {
      id: "desk-1",
      connected: true,
      has_saved_config: true,
      configured_at: 123,
      state: "active",
    },
  ]);
  expect(await account.claimPanel()).toEqual({
    device_id: "desk-2",
    token: "once",
    link_url: "wss://desk/link",
  });
  await account.removePanel("desk/2");
});

test("account route errors keep the server's recovery sentence", async () => {
  httpHandlers.set(
    "POST /v1/app/auth/email",
    () =>
      new Response(JSON.stringify({ error: "Try again in 12 minutes." }), {
        status: 429,
        headers: { "content-type": "application/json" },
      }),
  );
  await expect(account.requestSignInLink("owner@example.com")).rejects.toMatchObject({
    message: "Try again in 12 minutes.",
  });
});

test("a linked display outranks a newer refused-config row", async () => {
  const remembered = window.localStorage.getItem("deskmate.device_id");
  const devices = [
    {
      id: "refused",
      connected: false,
      has_saved_config: true,
      configured_at: 200,
    },
    {
      id: "healthy",
      connected: true,
      has_saved_config: true,
      configured_at: 100,
    },
  ];
  httpHandlers.set("GET /v1/app/devices", () => new Response(JSON.stringify(devices)));

  try {
    window.localStorage.removeItem("deskmate.device_id");
    const client = await import("../src/lib/backendClient.ts?production-client");
    expect(await client.getNetworkSettings()).toMatchObject({
      device_id: "healthy",
    });
  } finally {
    if (remembered === null) window.localStorage.removeItem("deskmate.device_id");
    else window.localStorage.setItem("deskmate.device_id", remembered);
  }
});

test("real event streams preserve credentials, parsing and stale-source cleanup", async () => {
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
  httpHandlers.set(
    "GET /v1/app/devices",
    () =>
      new Response(
        JSON.stringify([
          {
            id: "desk A/α",
            connected: true,
            has_saved_config: true,
            configured_at: 100,
          },
        ]),
      ),
  );
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
    window.localStorage.setItem("deskmate.device_id", "desk A/α");
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

    stopA();
    subscribe();
    await settle();
    expect(sources[0].closes).toBe(1);
    expect(sources[1].url).toBe("/v1/app/desk%20A%2F%CE%B1/events");
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

test("session loss forgets the previous account's selected panel", async () => {
  const client = await import("../src/lib/backendClient.ts?production-client");
  client.resetAccountState();
  httpHandlers.set("GET /v1/app/devices", () =>
    jsonResponse([{ id: "first-panel", connected: true }]),
  );
  expect(await client.getNetworkSettings()).toMatchObject({
    device_id: "first-panel",
  });
  httpHandlers.set("GET /v1/app/account", () => new Response(null, { status: 401 }));
  await expect(client.request("GET", "/v1/app/account")).rejects.toThrow(
    client.SESSION_REQUIRED_MESSAGE,
  );
  httpHandlers.set("GET /v1/app/devices", () =>
    jsonResponse([{ id: "second-panel", connected: true }]),
  );
  expect(await client.getNetworkSettings()).toMatchObject({
    device_id: "second-panel",
  });
  client.resetAccountState();
});

test("a picture-source refusal shows its own sentence, never [object Object]", async () => {
  httpHandlers.set("POST /v1/images", () =>
    jsonResponse(
      {
        kind: "capacity",
        message: "the image-source capacity has been reached",
      },
      409,
    ),
  );
  const failure = await realMintImageSource("Terminal Command").catch((error: unknown) => error);
  expect(failure).toBeInstanceOf(Error);
  expect((failure as Error).message).toBe(
    "This account already has 8 picture sources, the most a panel can hold. Remove a picture card to add another.",
  );

  httpHandlers.set("POST /v1/images", () =>
    jsonResponse(
      {
        kind: "invalid-face-fields",
        message: "platform must be one of both, linux, macos",
      },
      422,
    ),
  );
  const invalid = await realMintImageSource("Terminal Command").catch((error: unknown) => error);
  expect((invalid as Error).message).toBe("platform must be one of both, linux, macos");
});
