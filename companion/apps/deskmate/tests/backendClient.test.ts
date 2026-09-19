import { afterEach, beforeEach, expect, test } from "bun:test";

import { resetBackendMocks, realMintImageSource } from "./support/backendMock";
import { installDomLifecycle } from "./support/dom";
import { installHttpLifecycle, httpCalls, allowImageMint, createHttpMock } from "./support/http";

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
