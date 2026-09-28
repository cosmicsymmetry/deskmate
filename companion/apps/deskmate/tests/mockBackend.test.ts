import { afterEach, beforeAll, beforeEach, describe, expect, spyOn, test } from "bun:test";

import { mockConfig } from "../src/dev/fixture";
import type { AppSnapshot, FaceDescriptor } from "../src/lib/types";

let backend: typeof import("../src/dev/backendClient");
let now = 0;
let timers: { at: number; run: () => void }[] = [];
let restoreTimeout: () => void;
const cleanup: (() => void)[] = [];

beforeAll(async () => {
  const previousUrl = window.location.href;
  window.location.href = "http://localhost/?scenario=signedout";
  // A separate client instance pins its initial authentication state even when
  // another suite has already imported the ordinary mock client.
  backend = await import("../src/dev/backendClient.ts?characterization");
  window.location.href = previousUrl;
});

beforeEach(() => {
  now = 0;
  timers = [];
  const browserWindow: Window = window;
  const timeout = spyOn(browserWindow, "setTimeout").mockImplementation((handler, ms = 0) => {
    timers.push({ at: now + ms, run: () => (handler as () => void)() });
    return timers.length;
  });
  restoreTimeout = () => timeout.mockRestore();
});

afterEach(() => {
  for (const remove of cleanup.splice(0)) remove();
  restoreTimeout();
});

async function advance(ms: number) {
  now += ms;
  const due = timers.filter((timer) => timer.at <= now);
  timers = timers.filter((timer) => timer.at > now);
  for (const timer of due) timer.run();
  // Promise wrappers may adopt another promise before notifying callers.
  for (let turn = 0; turn < 8; turn += 1) await Promise.resolve();
}

async function finish<T>(pending: Promise<T>, ms = 90): Promise<T> {
  await advance(ms);
  return pending;
}

function twoPomodoroConfig() {
  const draft = mockConfig();
  const first = draft.cards.find((card) => card.kind === "pomodoro");
  if (first?.kind !== "pomodoro") throw new Error("missing timer fixture");
  const second = {
    ...first,
    id: "pomodoro-two",
    label: "Second timer",
    duration_seconds: 600,
  };
  draft.cards.push(second);
  return { draft, first, second };
}

describe("mock backend contract", () => {
  test("requires sign-in and accepts the same link route as the shipped app", async () => {
    await expect(backend.getAppSnapshot()).rejects.toMatchObject({
      details: { category: "runtime-unavailable", message: backend.SESSION_REQUIRED_MESSAGE },
    });
    const instance = backend.request<{ setup_required: boolean }>("GET", "/v1/app/instance");
    expect((await finish(instance)).setup_required).toBe(false);
    await expect(backend.request("GET", "/v1/app/devices")).rejects.toMatchObject({
      details: { message: backend.SESSION_REQUIRED_MESSAGE },
    });
    const signedIn = backend.request("POST", "/v1/app/auth/link", { token: "token" });
    expect(timers).toHaveLength(1);
    await finish(signedIn);
    expect((await finish(backend.getAppSnapshot())).config).toBeDefined();
  });

  test("exposes the complete application backend shape", () => {
    expect(Object.keys(backend).sort()).toEqual(
      [
        "DeskmateApiError",
        "NO_PANELS_MESSAGE",
        "SESSION_REQUIRED_MESSAGE",
        "isNoPanels",
        "isSessionMissing",
        "toApiError",
        "request",
        "getAppSnapshot",
        "validateConfigDraft",
        "saveConfig",
        "getNetworkSettings",
        "resumePushing",
        "controlPomodoro",
        "renderCardPreview",
        "mintImageSource",
        "listCreatableFaces",
        "listImageSources",
        "updateImageSourceFace",
        "listenToAppState",
      ].sort(),
    );
  });

  test("publishes a JSON-isolated save immediately, before its 350 ms receipt", async () => {
    const draft = mockConfig();
    draft.assets = [
      {
        id: "icons",
        source: { kind: "file", value: "icons.bin" },
        maximum_bytes: 10,
        kind: { kind: "icon-font", glyphs: [{ name: "sun", codepoint: 42 }] },
      },
    ];
    const expected = JSON.parse(JSON.stringify(draft));
    const published: AppSnapshot[] = [];
    cleanup.push(await backend.listenToAppState((next) => published.push(next)));
    let settled = false;
    const pending = backend.saveConfig(draft).then((result) => {
      settled = true;
      return result;
    });
    expect(published).toHaveLength(1);
    expect(published[0].config).toEqual(expected);
    expect(published[0].config).not.toBe(draft);
    draft.preferences.timezone = "UTC";
    draft.cards[0].tap_action.kind = "dismiss";
    if (draft.cards[0].kind === "clock") draft.cards[0].template.kind = "analog-clock";
    draft.image_sources[0].name = "Changed";
    draft.assets[0].source.value = "changed.bin";
    if (draft.assets[0].kind.kind === "icon-font") draft.assets[0].kind.glyphs[0].name = "moon";
    draft.advance = { kind: "manual" };
    draft.updater.channel = "beta";
    expect(published[0].config).toEqual(expected);
    await advance(349);
    expect(settled).toBe(false);
    await advance(1);
    expect(await pending).toEqual({ save: { generation: 1, warning: null } });
    expect((await finish(backend.getAppSnapshot())).config).toEqual(expected);
  });

  test("rejects publication failures but throws serialization failures synchronously", async () => {
    const failure = new Error("subscriber failed");
    const remove = await backend.listenToAppState(() => {
      throw failure;
    });
    cleanup.push(remove);
    await expect(backend.saveConfig(mockConfig())).rejects.toBe(failure);
    await expect(backend.resumePushing()).rejects.toBe(failure);
    await expect(backend.controlPomodoro("pomodoro", "pause")).rejects.toBe(failure);
    expect(timers).toHaveLength(0);
    remove();
    const draft = {
      ...mockConfig(),
      toJSON() {
        throw failure;
      },
    };
    expect(() => backend.saveConfig(draft)).toThrow(failure);
    expect(() => backend.validateConfigDraft(draft)).toThrow(failure);
  });

  test("validates the supplied draft at call time and waits 40 ms", async () => {
    const draft = mockConfig();
    draft.preferences.timezone = "Mars/Olympus";
    const timer = draft.cards.find((card) => card.kind === "pomodoro");
    if (timer?.kind !== "pomodoro") throw new Error("missing timer fixture");
    timer.duration_seconds = 1;
    timer.alert = { kind: "on-timer-finish", hold: { kind: "seconds", value: 1 } };
    draft.image_sources = [];
    draft.advance = { kind: "timed", default_dwell_seconds: 1 };
    let settled = false;
    const pending = backend.validateConfigDraft(draft).then((result) => {
      settled = true;
      return result;
    });
    draft.preferences.timezone = "UTC";
    await advance(39);
    expect(settled).toBe(false);
    await advance(1);
    expect(await pending).toEqual({
      valid: false,
      issues: [
        {
          path: "preferences.timezone",
          code: "invalid-timezone",
          message: "That is not a timezone name.",
        },
        {
          path: "cards[1].duration_seconds",
          code: "out-of-range",
          message: "Use between 1 and 1440 minutes.",
        },
        {
          path: "cards[1].alert.hold.value",
          code: "out-of-range",
          message: "Hold for between 5 and 600 seconds.",
        },
        {
          path: "cards[2].source_id",
          code: "missing-reference",
          message: "Choose an existing picture source.",
        },
        {
          path: "advance.default_dwell_seconds",
          code: "out-of-range",
          message: "Use 5 to 3600 seconds.",
        },
      ],
    });
    expect(await finish(backend.validateConfigDraft(mockConfig()), 40)).toEqual({
      valid: true,
      issues: [],
    });
  });

  test("uses the default 90 ms delay for reads", async () => {
    let settled = false;
    const pending = backend.getNetworkSettings().then((result) => {
      settled = true;
      return result;
    });
    await advance(89);
    expect(settled).toBe(false);
    await advance(1);
    expect((await pending).tier).toBe("networked");
  });

  test("looks previews up by card id and rejects missing cards", async () => {
    const context = spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
    try {
      expect(await backend.renderCardPreview("clock")).toEqual({
        png_base64: "",
        sample: true,
        state: null,
      });
      expect(context).toHaveBeenCalledWith("2d");
      await expect(backend.renderCardPreview("missing")).rejects.toEqual({
        category: "not-found",
        message: "No such card.",
      });
      expect(timers).toHaveLength(0);
    } finally {
      context.mockRestore();
    }
  });

  test("removes listeners and publishes resume before its delay", async () => {
    const draft = mockConfig();
    draft.preferences.paused = true;
    await finish(backend.saveConfig(draft), 350);
    const published: AppSnapshot[] = [];
    const remove = await backend.listenToAppState((next) => published.push(next));
    cleanup.push(remove);
    expect(published).toHaveLength(0);
    const pending = backend.resumePushing();
    expect(published).toHaveLength(1);
    expect(published[0].config.preferences.paused).toBe(false);
    expect(published[0].runtime).toEqual({ kind: "running" });
    expect(timers.map((timer) => timer.at - now)).toEqual([90]);
    await finish(pending);
    remove();
    await finish(backend.resumePushing());
    expect(published).toHaveLength(1);
  });

  test("discovers creatable faces and returns defensive clones", async () => {
    const expected: FaceDescriptor[] = [
      {
        kind: "weather",
        label: "Weather",
        fields: [
          {
            type: "text",
            key: "location",
            label: "Location",
            value: "",
            placeholder: "Dubai",
          },
          {
            type: "enum",
            key: "units",
            label: "Units",
            value: "metric",
            options: [
              { value: "metric", label: "Metric" },
              { value: "imperial", label: "Imperial" },
            ],
          },
        ],
      },
      {
        kind: "hackernews",
        label: "Hacker News",
        tap: "Tap the panel for the next stories.",
        fields: [
          {
            type: "enum",
            key: "list",
            label: "Stories",
            value: "top",
            options: [
              { value: "top", label: "Front page" },
              { value: "best", label: "Best" },
              { value: "new", label: "Newest" },
              { value: "ask", label: "Ask HN" },
              { value: "show", label: "Show HN" },
            ],
          },
        ],
      },
      {
        kind: "rss",
        label: "RSS feed",
        fields: [
          {
            type: "url",
            key: "url",
            label: "Feed URL",
            value: "",
            placeholder: "https://example.com/feed.xml",
          },
          {
            type: "text",
            key: "title",
            label: "Title",
            value: "",
            placeholder: "News",
          },
        ],
      },
      {
        kind: "token",
        label: "Token price",
        fields: [
          {
            type: "text",
            key: "coin_id",
            label: "Ticker",
            value: "",
            placeholder: "SOL",
          },
          {
            type: "text",
            key: "currency",
            label: "Currency",
            value: "usd",
            placeholder: "usd",
          },
          {
            type: "enum",
            key: "chart",
            label: "Chart",
            value: "line",
            options: [
              { value: "line", label: "Line" },
              { value: "candles", label: "Candles" },
              { value: "none", label: "None" },
            ],
          },
        ],
      },
    ];
    const discovered = await finish(backend.listCreatableFaces());
    expect(discovered).toEqual(expected);
    discovered[0].label = "mutated";
    discovered[0].fields[0].value = "mutated";
    expect(await finish(backend.listCreatableFaces())).toEqual(expected);
  });

  test("mints named sources immediately with distinct identities and independent faces", async () => {
    await finish(backend.saveConfig(mockConfig()), 350);
    const weather = await finish(backend.mintImageSource("My weather", "weather"));
    const feed = await finish(backend.mintImageSource("My feed", "rss"));
    expect(weather.source_id).not.toBe(feed.source_id);
    expect(weather.token).not.toBe(feed.token);
    expect(weather.push_url).toEndWith(`/v1/images/${weather.token}`);
    expect(feed.push_url).toEndWith(`/v1/images/${feed.token}`);

    const listed = await finish(backend.listImageSources());
    expect(listed.find((source) => source.id === weather.source_id)).toMatchObject({
      name: "My weather",
      face: { kind: "weather" },
    });
    expect(listed.find((source) => source.id === feed.source_id)).toMatchObject({
      name: "My feed",
      face: { kind: "rss" },
    });

    const updatedWeather = await finish(
      backend.updateImageSourceFace(weather.source_id, { location: "Tbilisi" }),
    );
    const updatedFeed = await finish(
      backend.updateImageSourceFace(feed.source_id, {
        url: "https://example.com/news.xml",
        title: "Desk news",
      }),
    );
    expect(updatedWeather.fields.map((field) => field.value)).toEqual(["Tbilisi", "metric"]);
    expect(updatedFeed.fields.map((field) => field.value)).toEqual([
      "https://example.com/news.xml",
      "Desk news",
    ]);

    updatedWeather.fields[0].value = "mutated";
    const relisted = await finish(backend.listImageSources());
    expect(
      relisted
        .find((source) => source.id === weather.source_id)
        ?.face?.fields.map((field) => field.value),
    ).toEqual(["Tbilisi", "metric"]);
    expect(
      relisted
        .find((source) => source.id === feed.source_id)
        ?.face?.fields.map((field) => field.value),
    ).toEqual(["https://example.com/news.xml", "Desk news"]);
  });

  test("rejects unknown face kinds transactionally and reconciles sources on save", async () => {
    await finish(backend.saveConfig(mockConfig()), 350);
    const before = await finish(backend.listImageSources());
    await expect(backend.mintImageSource("Unknown", "not-a-face")).rejects.toEqual({
      category: "invalid-payload",
      message: 'Unknown face kind "not-a-face".',
    });
    expect(await finish(backend.listImageSources())).toEqual(before);

    const kept = await finish(backend.mintImageSource("Kept", "token"));
    const removed = await finish(backend.mintImageSource("Removed", "weather"));
    const draft = mockConfig();
    draft.image_sources.push({ id: kept.source_id, name: "Kept" });
    await finish(backend.saveConfig(draft), 350);
    const after = await finish(backend.listImageSources());
    expect(after.some((source) => source.id === kept.source_id)).toBe(true);
    expect(after.some((source) => source.id === removed.source_id)).toBe(false);
    await expect(backend.updateImageSourceFace(removed.source_id, {})).rejects.toEqual({
      category: "not-found",
      message: "This picture source no longer exists.",
    });
  });

  test("rejects an explicitly empty face kind without minting a source", async () => {
    await finish(backend.saveConfig(mockConfig()), 350);
    const before = await finish(backend.listImageSources());
    let rejection: unknown;
    try {
      await finish(backend.mintImageSource("Empty kind probe", ""));
    } catch (error) {
      rejection = error;
    }
    expect(rejection).toEqual({
      category: "invalid-payload",
      message: 'Unknown face kind "".',
    });
    expect(await finish(backend.listImageSources())).toEqual(before);
  });

  test("rejects updates for external and missing sources", async () => {
    await finish(backend.saveConfig(mockConfig()), 350);
    await expect(backend.updateImageSourceFace("studio", {})).rejects.toEqual({
      category: "invalid-payload",
      message: "This picture source has no configurable face.",
    });
    await expect(backend.updateImageSourceFace("missing", {})).rejects.toEqual({
      category: "not-found",
      message: "This picture source no longer exists.",
    });
  });

  test("preserves the picture scenario's editable face when seeding source state", async () => {
    const previousUrl = window.location.href;
    window.location.href = "http://localhost/?scenario=picture";
    try {
      const modulePath = "../src/dev/mockBackend.ts?picture-face-regression";
      const pictureBackend: typeof import("../src/dev/mockBackend") = await import(modulePath);
      const sources = await finish(pictureBackend.listImageSources());
      expect(sources).toHaveLength(1);
      expect(sources[0]).toMatchObject({
        id: "claude-limits",
        name: "Claude usage",
        face: { kind: "weather" },
      });
      expect(sources[0].face?.fields.map((field) => field.value)).toEqual(["Dubai", "metric"]);
    } finally {
      window.location.href = previousUrl;
    }
  });

  test("targets pomodoro actions by card id with exhaustive state semantics", async () => {
    await finish(backend.request("POST", "/v1/app/auth/link", { token: "token" }));
    const { draft } = twoPomodoroConfig();
    await finish(backend.saveConfig(draft), 350);
    const firstState = (await finish(backend.getAppSnapshot())).pomodoros.find(
      (timer) => timer.card_id === "pomodoro",
    )?.state;

    const published: AppSnapshot[] = [];
    cleanup.push(await backend.listenToAppState((next) => published.push(next)));
    await finish(backend.controlPomodoro("pomodoro-two", "start"));
    expect(
      published.at(-1)?.pomodoros.find((timer) => timer.card_id === "pomodoro-two")?.state,
    ).toBe("running");
    expect(published.at(-1)?.pomodoros.find((timer) => timer.card_id === "pomodoro")?.state).toBe(
      firstState,
    );
    await finish(backend.controlPomodoro("pomodoro-two", "pause"));
    expect(
      published.at(-1)?.pomodoros.find((timer) => timer.card_id === "pomodoro-two")?.state,
    ).toBe("paused");
    await finish(backend.controlPomodoro("pomodoro-two", "toggle"));
    expect(
      published.at(-1)?.pomodoros.find((timer) => timer.card_id === "pomodoro-two")?.state,
    ).toBe("running");
    await finish(backend.controlPomodoro("pomodoro-two", "toggle"));
    expect(
      published.at(-1)?.pomodoros.find((timer) => timer.card_id === "pomodoro-two")?.state,
    ).toBe("paused");
    await finish(backend.controlPomodoro("pomodoro-two", "reset"));
    const reset = published.at(-1)?.pomodoros.find((timer) => timer.card_id === "pomodoro-two");
    expect(reset).toMatchObject({ state: "idle", duration_seconds: 600, remaining_seconds: 600 });

    const beforeUnknown = JSON.parse(
      JSON.stringify((await finish(backend.getAppSnapshot())).pomodoros),
    );
    const publicationCount = published.length;
    await expect(backend.controlPomodoro("missing", "start")).rejects.toEqual({
      category: "device",
      message: 'UnknownCard { card_id: "missing" }',
    });
    expect(published).toHaveLength(publicationCount);
    expect((await finish(backend.getAppSnapshot())).pomodoros).toEqual(beforeUnknown);
  });

  test("reconciles pomodoros across unchanged, edited, added, and removed cards", async () => {
    await finish(backend.request("POST", "/v1/app/auth/link", { token: "token" }));
    const { draft, first, second } = twoPomodoroConfig();
    await finish(backend.saveConfig(draft), 350);
    await finish(backend.controlPomodoro("pomodoro-two", "start"));
    await finish(backend.controlPomodoro("pomodoro", "pause"));

    await finish(backend.saveConfig(draft), 350);
    let snapshots = (await finish(backend.getAppSnapshot())).pomodoros;
    expect(snapshots.find((timer) => timer.card_id === "pomodoro")?.state).toBe("paused");
    expect(snapshots.find((timer) => timer.card_id === "pomodoro-two")?.state).toBe("running");

    first.label = "Edited label";
    second.duration_seconds = 900;
    await finish(backend.saveConfig(draft), 350);
    snapshots = (await finish(backend.getAppSnapshot())).pomodoros;
    expect(snapshots.find((timer) => timer.card_id === "pomodoro")).toMatchObject({
      state: "idle",
      duration_seconds: 1500,
      remaining_seconds: 1500,
    });
    expect(snapshots.find((timer) => timer.card_id === "pomodoro-two")).toMatchObject({
      state: "idle",
      duration_seconds: 900,
      remaining_seconds: 900,
    });

    draft.cards = draft.cards.filter((card) => card.id !== "pomodoro");
    const third = { ...second, id: "pomodoro-three", label: "New timer" };
    draft.cards.push(third);
    await finish(backend.saveConfig(draft), 350);
    snapshots = (await finish(backend.getAppSnapshot())).pomodoros;
    expect(snapshots.map((timer) => timer.card_id)).toEqual(["pomodoro-two", "pomodoro-three"]);
    expect(snapshots.find((timer) => timer.card_id === "pomodoro-three")).toMatchObject({
      state: "idle",
      duration_seconds: 900,
      remaining_seconds: 900,
    });
  });

  test("ticks every running pomodoro, completes at zero, and publishes once", async () => {
    const ticks: (() => void)[] = [];
    const browserWindow: Window = window;
    const interval = spyOn(browserWindow, "setInterval").mockImplementation((handler) => {
      ticks.push(() => (handler as () => void)());
      return ticks.length;
    });
    try {
      const modulePath = "../src/dev/mockBackend.ts?pomodoro-tick-regression";
      const tickingBackend: typeof import("../src/dev/mockBackend") = await import(modulePath);
      expect(ticks).toHaveLength(1);
      const draft = mockConfig();
      const first = draft.cards.find((card) => card.kind === "pomodoro");
      if (first?.kind !== "pomodoro") throw new Error("missing timer fixture");
      first.duration_seconds = 1;
      const second = { ...first, id: "pomodoro-two", label: "Second timer" };
      draft.cards.push(second);
      await finish(tickingBackend.saveConfig(draft), 350);
      await finish(tickingBackend.controlPomodoro("pomodoro", "start"));
      await finish(tickingBackend.controlPomodoro("pomodoro-two", "start"));
      const published: AppSnapshot[] = [];
      cleanup.push(await tickingBackend.listenToAppState((next) => published.push(next)));

      ticks[0]();
      expect(published).toHaveLength(1);
      expect(published[0].pomodoros).toEqual([
        { card_id: "pomodoro", state: "completed", duration_seconds: 1, remaining_seconds: 0 },
        {
          card_id: "pomodoro-two",
          state: "completed",
          duration_seconds: 1,
          remaining_seconds: 0,
        },
      ]);

      for (const action of ["start", "pause", "toggle"] as const) {
        await finish(tickingBackend.controlPomodoro("pomodoro", action));
        expect(
          (await finish(tickingBackend.mockGetAppSnapshot())).pomodoros.find(
            (timer) => timer.card_id === "pomodoro",
          )?.state,
        ).toBe("completed");
      }
      await finish(tickingBackend.controlPomodoro("pomodoro", "reset"));
      expect(
        (await finish(tickingBackend.mockGetAppSnapshot())).pomodoros.find(
          (timer) => timer.card_id === "pomodoro",
        ),
      ).toMatchObject({ state: "idle", remaining_seconds: 1 });
    } finally {
      interval.mockRestore();
    }
  });
});
