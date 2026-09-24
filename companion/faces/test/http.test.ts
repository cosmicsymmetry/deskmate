import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { ConfigurationError, TransientError } from "../src/face";
import {
  createFetchText,
  createRequest,
  dial,
  type DialFn,
  isPrivateAddress,
  pinnedAddress,
  replyFrom,
  USER_AGENT,
} from "../src/kit/http";

const PUBLIC = "93.184.215.14";

describe("which addresses are refused", () => {
  test("private, loopback, link-local, CGNAT and metadata are; public ones are not", () => {
    for (const address of [
      "127.0.0.1",
      "10.1.2.3",
      "172.16.0.1",
      "192.168.1.1",
      "169.254.169.254",
      "100.93.166.123",
      "0.0.0.0",
      "::1",
      "fd00::1",
      "fe80::1",
      "::ffff:10.0.0.1",
    ]) {
      expect(isPrivateAddress(address)).toBe(true);
    }
    for (const address of ["1.1.1.1", "172.32.0.1", "100.128.0.1", "2606:4700::1111"]) {
      expect(isPrivateAddress(address)).toBe(false);
    }
  });

  test("a name with ONE private answer among public ones is refused outright", async () => {
    // The rebinding shape: trusting the client to pick the good address is the bug.
    const mixed = async () => [PUBLIC, "192.168.1.10"];
    expect(pinnedAddress(new URL("https://feed.example/rss"), mixed)).rejects.toBeInstanceOf(
      ConfigurationError,
    );
  });

  test("an IP literal is judged as itself and never resolved", async () => {
    const never = async (): Promise<string[]> => {
      throw new Error("an IP literal must not reach the resolver");
    };
    expect(await pinnedAddress(new URL("http://1.1.1.1/"), never)).toBe("1.1.1.1");
    expect(pinnedAddress(new URL("http://127.0.0.1:8443/"), never)).rejects.toBeInstanceOf(
      ConfigurationError,
    );
    expect(pinnedAddress(new URL("http://[::1]/"), never)).rejects.toBeInstanceOf(
      ConfigurationError,
    );
  });

  test("IPv4 is preferred, a name that does not resolve is transient, and only http(s) is fetched", async () => {
    const dual = async () => ["2606:4700::1111", PUBLIC];
    expect(await pinnedAddress(new URL("https://feed.example/"), dual)).toBe(PUBLIC);
    const nothing = async () => [];
    expect(pinnedAddress(new URL("https://feed.example/"), nothing)).rejects.toBeInstanceOf(
      TransientError,
    );
    expect(pinnedAddress(new URL("file:///etc/passwd"), dual)).rejects.toBeInstanceOf(
      ConfigurationError,
    );
  });
});

describe("the pin", () => {
  let server: ReturnType<typeof Bun.serve>;
  beforeAll(() => {
    server = Bun.serve({
      hostname: "127.0.0.1",
      port: 0,
      fetch: (request) =>
        new Response(`host=${request.headers.get("host")} path=${new URL(request.url).pathname}`),
    });
  });
  afterAll(() => server.stop(true));

  test("the dialled address is the pinned one, and the name only rides in the Host header", async () => {
    // `feed.invalid` resolves nowhere, so this request can only arrive if the address
    // handed to `dial` is what gets connected to -- a second lookup would fail it.
    const url = new URL(`http://feed.invalid:${server.port}/rss.xml`);
    const response = await dial(url, "127.0.0.1", AbortSignal.timeout(5_000));
    expect(await response.text()).toBe(`host=feed.invalid:${server.port} path=/rss.xml`);
  });

  test("a redirect hop is validated like the first: a public feed cannot redirect inward", async () => {
    // Every hop resolves to a private address here, so the first one is refused --
    // and the message names the host the owner typed, never an address to retry.
    const inward = createFetchText(async () => ["10.0.0.5"]);
    const failure = await inward("https://feed.example/rss").catch((error: unknown) => error);
    expect(failure).toBeInstanceOf(ConfigurationError);
    expect((failure as Error).message).toContain("feed.example");
  });

  test("something that is not a URL is the owner's to fix", async () => {
    expect(createFetchText()("not a url")).rejects.toBeInstanceOf(ConfigurationError);
  });
});

describe("dial refuses to let a caller take back its Host header", () => {
  test("a caller-supplied Host, in any casing, does not reach the server; ordinary headers still do", async () => {
    const server = Bun.serve({
      hostname: "127.0.0.1",
      port: 0,
      fetch: (request) =>
        Response.json({
          host: request.headers.get("host"),
          auth: request.headers.get("authorization"),
          lang: request.headers.get("accept-language"),
        }),
    });
    try {
      const url = new URL(`http://spoof.invalid:${server.port}/`);
      const response = await dial(url, "127.0.0.1", AbortSignal.timeout(5_000), {
        headers: {
          Host: "evil.example",
          HOST: "also-evil.example",
          host: "still-evil.example",
          Authorization: "Bearer t",
          "Accept-Language": "en",
        },
      });
      const seen = (await response.json()) as {
        host: string | null;
        auth: string | null;
        lang: string | null;
      };
      // The guard's own Host wins over every casing of a caller-supplied one.
      expect(seen.host).toBe(`spoof.invalid:${server.port}`);
      // Ordinary headers are not collateral damage from the filter.
      expect(seen.auth).toBe("Bearer t");
      expect(seen.lang).toBe("en");
    } finally {
      server.stop(true);
    }
  });
});

describe("dial's User-Agent and Accept defaults replace, and never join, a caller's", () => {
  let server: ReturnType<typeof Bun.serve>;
  beforeAll(() => {
    server = Bun.serve({
      hostname: "127.0.0.1",
      port: 0,
      fetch: (request) =>
        Response.json({
          ua: request.headers.get("user-agent"),
          accept: request.headers.get("accept"),
          host: request.headers.get("host"),
          auth: request.headers.get("authorization"),
          lang: request.headers.get("accept-language"),
          custom: request.headers.get("x-plugin-token"),
        }),
    });
  });
  afterAll(() => server.stop(true));

  type Seen = {
    ua: string | null;
    accept: string | null;
    host: string | null;
    auth: string | null;
    lang: string | null;
    custom: string | null;
  };

  const seenFrom = async (headers: Record<string, string>): Promise<Seen> => {
    const url = new URL(`http://ua.invalid:${server.port}/`);
    const response = await dial(url, "127.0.0.1", AbortSignal.timeout(5_000), { headers });
    return (await response.json()) as Seen;
  };

  test("a caller's User-Agent, in any casing, replaces the default instead of joining it", async () => {
    for (const key of ["user-agent", "User-Agent", "USER-AGENT"]) {
      const seen = await seenFrom({ [key]: "plugin-ua/1" });
      expect(seen.ua).toBe("plugin-ua/1");
    }
  });

  test("a caller's Accept, in any casing, replaces the default instead of joining it", async () => {
    for (const key of ["accept", "Accept", "ACCEPT"]) {
      const seen = await seenFrom({ [key]: "application/vnd.example+json" });
      expect(seen.accept).toBe("application/vnd.example+json");
    }
  });

  test("a caller that sends neither still gets the package's defaults", async () => {
    const seen = await seenFrom({});
    expect(seen.ua).toBe(USER_AGENT);
    expect(seen.accept).toBe("*/*");
  });

  test("Host is still stripped in any casing, and ordinary headers still pass through verbatim", async () => {
    const seen = await seenFrom({
      Host: "evil.example",
      HOST: "also-evil.example",
      host: "still-evil.example",
      Authorization: "Bearer t",
      "Accept-Language": "en",
      "X-Plugin-Token": "abc123",
    });
    expect(seen.host).toBe(`ua.invalid:${server.port}`);
    expect(seen.auth).toBe("Bearer t");
    expect(seen.lang).toBe("en");
    expect(seen.custom).toBe("abc123");
  });
});

describe("createRequest", () => {
  let server: ReturnType<typeof Bun.serve>;
  const seen: { method: string; auth: string | null; body: string }[] = [];
  beforeAll(() => {
    server = Bun.serve({
      hostname: "127.0.0.1",
      port: 0,
      fetch: async (request) => {
        const url = new URL(request.url);
        seen.push({
          method: request.method,
          auth: request.headers.get("authorization"),
          body: await request.text(),
        });
        if (url.pathname === "/json") return Response.json({ n: 7 });
        if (url.pathname === "/png") return new Response(new Uint8Array([137, 80, 78, 71]));
        if (url.pathname === "/big") return new Response("x".repeat(1024 * 1024 + 1));
        if (url.pathname === "/missing") return new Response("gone", { status: 404 });
        return new Response("text");
      },
    });
  });
  afterAll(() => server.stop(true));

  const at = (path: string) => `http://local.invalid:${server.port}${path}`;

  // `createRequest` itself pins its address through `pinnedAddress`, which refuses
  // loopback -- the same reason this file's `dial`-level tests above hit the real local
  // server directly instead of through `createFetchText`. These four tests exercise the
  // exact same `dial` + `replyFrom` pair `createRequest` calls internally, dialling the
  // pinned loopback address by hand; the two tests below that only need to observe a
  // *refusal* go through `createRequest` itself, because no real connection is ever made.
  const wire = async (
    path: string,
    input: { method?: "GET" | "POST"; headers?: Record<string, string>; body?: string },
    as: "json" | "text" | "bytes",
  ) => {
    const response = await dial(new URL(at(path)), "127.0.0.1", AbortSignal.timeout(5_000), input);
    return replyFrom(response, as);
  };

  test("returns decoded JSON, text and bytes as asked", async () => {
    expect((await wire("/json", {}, "json")).json).toEqual({ n: 7 });
    expect((await wire("/text", {}, "text")).body).toBe("text");
    const bytes = (await wire("/png", {}, "bytes")).body;
    expect(bytes).toBeInstanceOf(Uint8Array);
    expect(Array.from(bytes as Uint8Array)).toEqual([137, 80, 78, 71]);
  });

  test("sends the method, body and headers it was given", async () => {
    seen.length = 0;
    await wire(
      "/text",
      { method: "POST", body: '{"q":1}', headers: { Authorization: "Bearer t" } },
      "text",
    );
    expect(seen[0]).toEqual({ method: "POST", auth: "Bearer t", body: '{"q":1}' });
  });

  test("returns a non-ok status instead of throwing, because a 404 is the plugin's business", async () => {
    expect((await wire("/missing", {}, "text")).status).toBe(404);
  });

  test("refuses a body over the 1 MB cap", async () => {
    expect(wire("/big", {}, "text")).rejects.toBeInstanceOf(TransientError);
  });

  test("refuses a private address, and names the host the owner typed", async () => {
    const inward = createRequest(async () => ["169.254.169.254"]);
    const failure = await inward({ url: "https://metadata.example/", as: "text" }).catch(
      (error: unknown) => error,
    );
    expect(failure).toBeInstanceOf(ConfigurationError);
    expect((failure as Error).message).toContain("metadata.example");
  });

  test("refuses a scheme that is not http(s)", async () => {
    const local = createRequest(async () => ["127.0.0.1"]);
    expect(local({ url: "file:///etc/passwd", as: "text" })).rejects.toBeInstanceOf(
      ConfigurationError,
    );
  });
});

describe("createRequest end to end, with an injected transport", () => {
  // The resolver answers with a PUBLIC address, so `pinnedAddress` genuinely runs and
  // passes -- these tests exercise `createRequest` itself, not a bypass of it. Only the
  // transport is faked, the same way `Resolve` is already faked above; the production
  // default for both arguments is the real thing.
  type RecordedCall = {
    url: string;
    address: string;
    method?: string;
    headers?: Record<string, string>;
    body?: string;
  };

  function recordingDial(response: Response): { dialFn: DialFn; calls: RecordedCall[] } {
    const calls: RecordedCall[] = [];
    const dialFn: DialFn = async (url, address, _signal, init) => {
      calls.push({
        url: url.toString(),
        address,
        method: init?.method,
        headers: init?.headers,
        body: init?.body,
      });
      return response;
    };
    return { dialFn, calls };
  }

  test("decodes json, text and bytes from the injected response", async () => {
    const json = createRequest(async () => [PUBLIC], recordingDial(Response.json({ n: 7 })).dialFn);
    expect((await json({ url: "https://api.example/data", as: "json" })).json).toEqual({ n: 7 });

    const text = createRequest(async () => [PUBLIC], recordingDial(new Response("text")).dialFn);
    expect((await text({ url: "https://api.example/data", as: "text" })).body).toBe("text");

    const bytes = createRequest(
      async () => [PUBLIC],
      recordingDial(new Response(new Uint8Array([137, 80, 78, 71]))).dialFn,
    );
    const body = (await bytes({ url: "https://api.example/data", as: "bytes" })).body;
    expect(body).toBeInstanceOf(Uint8Array);
    expect(Array.from(body as Uint8Array)).toEqual([137, 80, 78, 71]);
  });

  test("the method, headers and body reach the transport unchanged", async () => {
    const { dialFn, calls } = recordingDial(new Response("ok"));
    const withTransport = createRequest(async () => [PUBLIC], dialFn);
    await withTransport({
      url: "https://api.example/data",
      method: "POST",
      headers: { Authorization: "Bearer t" },
      body: '{"q":1}',
      as: "text",
    });
    expect(calls).toHaveLength(1);
    expect(calls[0]).toEqual({
      url: "https://api.example/data",
      address: PUBLIC,
      method: "POST",
      headers: { Authorization: "Bearer t" },
      body: '{"q":1}',
    });
  });

  test("a non-ok status comes back as {status: 404} and does not throw", async () => {
    const { dialFn } = recordingDial(new Response("gone", { status: 404 }));
    const withTransport = createRequest(async () => [PUBLIC], dialFn);
    expect((await withTransport({ url: "https://api.example/data", as: "text" })).status).toBe(404);
  });

  test("a body over the 1 MB cap is refused as TransientError", async () => {
    const { dialFn } = recordingDial(new Response("x".repeat(1024 * 1024 + 1)));
    const withTransport = createRequest(async () => [PUBLIC], dialFn);
    expect(withTransport({ url: "https://api.example/data", as: "text" })).rejects.toBeInstanceOf(
      TransientError,
    );
  });

  test("the guard refuses a private address before the transport is ever called", async () => {
    const { dialFn, calls } = recordingDial(new Response("should not be reached"));
    const inward = createRequest(async () => ["169.254.169.254"], dialFn);
    const failure = await inward({ url: "https://metadata.example/", as: "text" }).catch(
      (error: unknown) => error,
    );
    expect(failure).toBeInstanceOf(ConfigurationError);
    expect(calls).toHaveLength(0);
  });

  test("a deadline surfaces as TransientError, not a raw DOM exception", async () => {
    // The real deadline is `AbortSignal.timeout(TIMEOUT_MS)`, 15s -- too slow to wait
    // out in a test. The injected transport instead throws what a real `fetch` throws
    // when that signal fires, so this proves `createRequest`'s own catch, not the clock.
    const timesOut: DialFn = async () => {
      throw new DOMException("The operation timed out.", "TimeoutError");
    };
    const req = createRequest(async () => [PUBLIC], timesOut);
    const failure = await req({ url: "https://api.example/data", as: "text" }).catch(
      (error: unknown) => error,
    );
    expect(failure).toBeInstanceOf(TransientError);
    expect(failure).not.toBeInstanceOf(DOMException);
    expect((failure as Error).message).toContain("api.example");
  });
});
