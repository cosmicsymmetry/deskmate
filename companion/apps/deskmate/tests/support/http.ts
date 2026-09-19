import { afterEach, beforeEach, expect } from "bun:test";

type HttpCall = { method: string; path: string; body: unknown };
type Handler = (body: unknown, init: RequestInit | undefined) => Response | Promise<Response>;

export function jsonResponse(value: unknown) {
  return new Response(JSON.stringify(value), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

export function createHttpMock() {
  const calls: HttpCall[] = [];
  const failures: unknown[] = [];
  const handlers = new Map<string, Handler>();
  return {
    calls,
    failures,
    handlers,
    async fetch(input: RequestInfo | URL, init?: RequestInit) {
      try {
        const path = typeof input === "string" ? input : input.toString();
        const method = init?.method ?? "GET";
        const body: unknown = typeof init?.body === "string" ? JSON.parse(init.body) : null;
        calls.push({ method, path, body });
        const handler = handlers.get(`${method} ${path}`);
        if (!handler) throw new Error(`Unexpected request: ${method} ${path}`);
        return handler(body, init);
      } catch (error) {
        // App catches optional-request failures and the client translates fetch
        // errors. Keep evidence outside the promise so neither can hide a bad call.
        failures.push(error);
        throw error;
      }
    },
    assertNoFailures() {
      expect(failures).toEqual([]);
    },
  };
}

const http = createHttpMock();
export const httpCalls = http.calls;
export const httpHandlers = http.handlers;
export const httpState: { creatableFacesResponse: unknown[] } = { creatableFacesResponse: [] };

export function expectJsonRequest(init: RequestInit | undefined) {
  expect(init?.credentials).toBe("same-origin");
  expect(new Headers(init?.headers).get("content-type")).toBe("application/json");
}

export function allowImageMint(name: string, faceKind: string | null = null) {
  httpHandlers.set("POST /v1/images", (body, init) => {
    expectJsonRequest(init);
    expect(body).toEqual({ name, face_kind: faceKind });
    // The route returns only id/token; source_id and push_url must be translated
    // by the real client, not supplied by the fixture.
    return jsonResponse({ id: "picture-source", token: "plaintext-once" });
  });
}

export function installHttpLifecycle(cleanup: () => Promise<void>) {
  let originalFetch: typeof fetch;
  beforeEach(() => {
    originalFetch = globalThis.fetch;
    http.calls.length = 0;
    http.failures.length = 0;
    http.handlers.clear();
    httpState.creatableFacesResponse = [];
    http.handlers.set("GET /v1/faces", () => jsonResponse(httpState.creatableFacesResponse));
    globalThis.fetch = Object.assign(http.fetch, { preconnect: originalFetch.preconnect });
  });
  afterEach(async () => {
    try {
      await cleanup();
      http.assertNoFailures();
    } finally {
      globalThis.fetch = originalFetch;
    }
  });
}
