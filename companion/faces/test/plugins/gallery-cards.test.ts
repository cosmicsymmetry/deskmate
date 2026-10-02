import { beforeAll, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { renderRequest, tapRequest, viewsRequest } from "../../src/main";
import type { HttpReply, RequestFn } from "../../src/kit/http";
import { textWidth } from "../../src/kit/raster";
import { discoverPlugins } from "../../src/plugins/discovery";
import { runPlugin } from "../../src/plugins/run";
import { runInSandbox, warmSandbox } from "../../src/plugins/sandbox";
import { shippedPlugin } from "./test_support";

interface Item {
  id: number | string;
  collection: string;
  title: string;
  artist?: string;
  date?: string;
  credit?: string;
  image: string;
  source: string;
}
interface GalleryState {
  day: string;
  index: number;
  collection: string;
  mode: string;
}

const root = join(import.meta.dir, "../../plugins");
const now = new Date("2026-10-02T12:00:00Z");
const load = (id: string, file: string) => readFileSync(join(root, id, file), "utf8");
const fixture = JSON.parse(load("ghibli-scenes", "check.json"));
// Original synthetic image, explicitly labelled in its pixels. No Ghibli media.
const jpeg = Buffer.from(fixture.cases[0].responses[0].base64, "base64");
beforeAll(warmSandbox);

for (const id of ["art-of-the-day", "ghibli-scenes"]) {
  const { source, manifest } = shippedPlugin(id);
  const catalog = JSON.parse(load(id, "catalog.json")) as Item[];
  const isArt = id === "art-of-the-day";
  const firstCollection = isArt ? "prints" : "totoro";
  const count = catalog.filter((item) => item.collection === firstCollection).length;
  const initial: GalleryState = {
    day: "2026-10-2",
    index: 0,
    collection: firstCollection,
    mode: "daily",
  };
  const defaultSettings = { collection: firstCollection };

  function host(
    calls: string[] = [],
    replacement?: (url: string) => HttpReply | undefined,
  ): RequestFn {
    return async ({ url }) => {
      calls.push(url);
      const override = replacement?.(url);
      if (override)
        return {
          ...override,
          ...(url.includes("collectionapi") &&
          typeof override.body === "string" &&
          override.status === 200
            ? { json: JSON.parse(override.body) }
            : {}),
        };
      if (url.startsWith("https://collectionapi.metmuseum.org/")) {
        const item = catalog.find((item) => String(item.id) === url.split("/").pop());
        if (!item) throw new Error(`Unexpected object request: ${url}`);
        return {
          status: 200,
          json: { objectID: item.id, isPublicDomain: true, primaryImageSmall: item.image },
          body: JSON.stringify({
            objectID: item.id,
            isPublicDomain: true,
            primaryImageSmall: item.image,
          }),
        };
      }
      if (!catalog.some((item) => item.image === url)) {
        throw new Error(`Unexpected image request: ${url}`);
      }
      return { status: 200, body: jpeg };
    };
  }

  function draw(
    options: {
      state?: unknown;
      settings?: Record<string, string>;
      date?: Date;
      timezone?: string;
      taps?: number;
      request?: RequestFn;
    } = {},
  ) {
    return runPlugin({
      manifest,
      source,
      settings: options.settings ?? defaultSettings,
      now: options.date ?? now,
      timezone: options.timezone ?? "UTC",
      secrets: {},
      state: options.state ?? initial,
      ...(options.taps === undefined ? {} : { event: { taps: options.taps, point: null } }),
      request: options.request ?? host(),
    });
  }

  describe(id, () => {
    test("discovery is clock-safe and exposes the actual render-tap contract", async () => {
      expect(runInSandbox<unknown[]>(source, "plan", {})).toEqual([]);
      expect(manifest.secrets).toEqual([]);
      const { faces, skipped } = await discoverPlugins(root, { request: host() }, [id]);
      expect(skipped).toEqual([]);
      const face = faces[0];
      if (!face) throw new Error("Gallery was not discovered");
      expect(face.kind).toBe(id);
      expect(face.tap).toContain("Next");
      expect(viewsRequest({ kind: id, settings: {} }, face)).toEqual({ views: [""] });
      expect(() => tapRequest({ kind: id, settings: {}, event: { taps: 1 } }, face)).toThrow(
        "handles taps through render",
      );
      const result = await renderRequest(
        {
          kind: id,
          settings: defaultSettings,
          state: initial,
          event: { taps: 2 },
          timezone: "UTC",
        },
        face,
        now,
      );
      const png = Buffer.from(result.png, "base64");
      expect(png.readUInt32BE(16)).toBe(448);
      expect(png.readUInt32BE(20)).toBe(368);
      expect((result.state as GalleryState).index).toBe(2);
    });

    test("requests only exact declared URLs, finishes within two rounds and embeds bytes", async () => {
      const calls: string[] = [];
      const result = await draw({ request: host(calls) });
      expect(calls).toHaveLength(isArt ? 2 : 1);
      expect(result.log).toEqual([]);
      expect(result.svg).toContain('href="data:image/jpeg;base64,');
      expect(result.svg).not.toMatch(/href="https?:/);
      expect(Buffer.byteLength(result.svg)).toBeLessThan(512 * 1024);
      expect(JSON.stringify(result.state).length).toBeLessThan(200);
      expect(result.svg).toContain(isArt ? "The Met · CC0" : "© 1988 Hayao Miyazaki/Studio Ghibli");
    });

    test("the catalog bundle agrees with reviewed source metadata and all labels fit", async () => {
      for (const item of catalog) {
        const items = catalog.filter((entry) => entry.collection === item.collection);
        const calls: string[] = [];
        const result = await draw({
          settings: { collection: item.collection },
          state: { ...initial, collection: item.collection, index: items.indexOf(item) },
          request: host(calls),
        });
        expect(calls.at(-1)).toBe(item.image);
        expect(result.svg).toContain(item.source);
        expect(result.svg).toContain(item.title);
        if (item.artist) expect(result.svg).toContain(item.artist);
        if (item.credit) expect(result.svg).toContain(item.credit);
        expect(textWidth(item.title, 17, 600)).toBeLessThan(isArt ? 412 : 340);
        if (isArt) expect(textWidth(`${item.artist} · ${item.date}`, 12, 400)).toBeLessThan(330);
        else {
          if (!item.credit) throw new Error("Gallery credit is missing");
          expect(textWidth(item.credit, 10, 400)).toBeLessThan(412);
        }
      }
    });

    test("daily image persists on refresh, coalesced taps advance and wrap", async () => {
      const start = await draw();
      const refresh = await draw({ state: start.state });
      expect(refresh.svg).toBe(start.svg);
      expect((await draw({ taps: 2 })).state).toEqual({ ...initial, index: 2 });
      expect((await draw({ taps: count })).state).toEqual(initial);
      expect((await draw({ taps: 100 })).state).toEqual({ ...initial, index: 32 % count });
      expect((await draw({ taps: -1 })).state).toEqual(initial);
    });

    test("local midnight resets daily selection, including zones ahead of UTC and DST", async () => {
      const tokyo = new Date("2026-10-02T15:01:00Z");
      const result = await draw({ date: tokyo, timezone: "Asia/Tokyo" });
      expect((result.state as GalleryState).day).toBe("2026-10-3");
      const fresh = await draw({ date: tokyo, timezone: "Asia/Tokyo", state: {} });
      expect(result.state).toEqual(fresh.state);
      const before = await draw({
        date: new Date("2026-11-01T05:30:00Z"),
        timezone: "America/New_York",
        state: {},
      });
      const after = await draw({
        date: new Date("2026-11-01T06:30:00Z"),
        timezone: "America/New_York",
        state: before.state,
      });
      expect(before.state).toEqual(after.state);
    });

    test("consecutive local days always select a different work, across month and year boundaries", async () => {
      for (const midnight of [
        "2026-11-01T00:00:00Z",
        "2027-01-01T00:00:00Z",
        "2028-03-01T00:00:00Z",
      ]) {
        const after = new Date(midnight);
        const before = new Date(after.getTime() - 60000);
        const previous = await draw({ date: before, state: {} });
        const next = await draw({ date: after, state: previous.state });
        expect((next.state as GalleryState).index).toBe(
          ((previous.state as GalleryState).index + 1) % count,
        );
      }
    });

    test("shuffle changes each successful refresh without an immediate repeat", async () => {
      const settings = { collection: firstCollection, change: "shuffle" };
      const first = await draw({ settings, state: {} });
      const next = await draw({ settings, state: first.state });
      expect((next.state as GalleryState).index).not.toBe((first.state as GalleryState).index);
      expect(await draw({ settings, state: first.state })).toEqual(next);
    });

    test("malformed state and unknown settings fall back predictably", async () => {
      const fresh = await draw({ state: {} });
      for (const index of [-1, 1e100, 0.5, "2", null]) {
        expect((await draw({ state: { ...initial, index } })).state).toEqual(fresh.state);
      }
      const unknown = await draw({
        settings: { collection: "__proto__", change: "bad", framing: "bad" },
      });
      expect((unknown.state as GalleryState).collection).toBe("all");
      expect((unknown.state as GalleryState).mode).toBe("daily");
      expect(unknown.svg).toContain('preserveAspectRatio="xMidYMid meet"');
      const filled = await draw({ settings: { ...defaultSettings, framing: "fill" } });
      expect(filled.svg).toContain('preserveAspectRatio="xMidYMid slice"');
    });

    test("missing, invalid, and impossible host dates fail without inventing a day", () => {
      for (const local of [
        undefined,
        {},
        { year: 2026, month: 2, day: 29 },
        { year: 1e100, month: 1, day: 1 },
      ]) {
        expect(() => runInSandbox(source, "render", { now: { local } })).toThrow("valid date");
      }
    });

    test.each([302, 403, 404, 429, 503])(
      "HTTP %s keeps the previous frame via a transient failure",
      async (status) => {
        await expect(
          draw({ request: async () => ({ status, body: "unavailable" }) }),
        ).rejects.toThrow("previous frame is kept");
      },
    );

    test("HTML, truncated JPEG, huge data, and oversized dimensions are rejected", async () => {
      const hugeDimensions = Buffer.from(jpeg);
      const frame = hugeDimensions.indexOf(Buffer.from([255, 192]));
      expect(frame).toBeGreaterThan(0);
      hugeDimensions[frame + 5] = 127;
      hugeDimensions[frame + 6] = 255;
      const huge = Buffer.alloc(380001, 0);
      const bodies = [
        Buffer.from("<html>upstream error page</html>"),
        jpeg.subarray(0, -4),
        huge,
        hugeDimensions,
      ];
      for (const body of bodies) {
        const request = host([], (url) =>
          url.includes("collectionapi") ? undefined : { status: 200, body },
        );
        await expect(draw({ request })).rejects.toThrow("previous frame is kept");
      }
      await expect(
        draw({
          request: host([], (url) =>
            url.includes("collectionapi")
              ? undefined
              : { status: 200, body: Buffer.alloc(1024 * 1024 + 1) },
          ),
        }),
      ).rejects.toThrow("previous frame is kept");
    });

    if (isArt) {
      test("public-domain status, object identity, and image host/path are checked before a second request", async () => {
        const first = catalog.find((item) => item.collection === firstCollection);
        if (!first) throw new Error("Gallery collection is missing");
        const good = { objectID: first.id, isPublicDomain: true, primaryImageSmall: first.image };
        for (const data of [
          null,
          { ...good, objectID: 123 },
          { ...good, isPublicDomain: false },
          { ...good, primaryImageSmall: "https://images.metmuseum.org.evil.example/image.jpg" },
          {
            ...good,
            primaryImageSmall: "http://images.metmuseum.org/CRDImages/as/web-large/a.jpg",
          },
          {
            ...good,
            primaryImageSmall: "https://images.metmuseum.org/CRDImages/as/web-large/../../a.jpg",
          },
          {
            ...good,
            primaryImageSmall:
              "https://images.metmuseum.org/CRDImages/as/web-large/a.jpg?redirect=x",
          },
          { ...good, primaryImageSmall: "file:///etc/passwd" },
        ]) {
          const calls: string[] = [];
          await expect(
            draw({ request: host(calls, () => ({ status: 200, body: JSON.stringify(data) })) }),
          ).rejects.toThrow("approved public-domain image");
          expect(calls).toHaveLength(1);
        }
      });
    }
  });
}
