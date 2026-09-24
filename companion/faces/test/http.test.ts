import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { ConfigurationError, TransientError } from "../src/face";
import {
  createFetchText,
  createRequest,
  dial,
  isPrivateAddress,
  pinnedAddress,
  replyFrom,
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
