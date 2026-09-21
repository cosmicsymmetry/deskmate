import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { ConfigurationError, TransientError } from "../src/face";
import { createFetchText, dial, isPrivateAddress, pinnedAddress } from "../src/kit/http";

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
