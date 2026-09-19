import { afterEach, beforeAll, beforeEach, describe, expect, spyOn, test } from "bun:test";

import { mockConfig } from "../src/dev/fixture";
import type { AppSnapshot } from "../src/lib/types";

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
  const timeout = spyOn(window, "setTimeout").mockImplementation((handler, ms = 0) => {
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

describe("mock backend contract", () => {
  test("requires sign-in and selects a device only after sign-in succeeds", async () => {
    await expect(backend.getAppSnapshot()).rejects.toMatchObject({
      details: { category: "runtime-unavailable", message: backend.SESSION_REQUIRED_MESSAGE },
    });
    await expect(backend.signInAndSelectDevice("rejected-device", " ")).rejects.toBeInstanceOf(
      backend.DeskmateApiError,
    );
    expect((await finish(backend.getNetworkSettings())).device_id).toBe("desk-01");
    const selected = backend.signInAndSelectDevice(" chosen-device ", "token");
    expect(timers).toHaveLength(0);
    await Promise.resolve();
    expect((await finish(selected)).device_id).toBe(" chosen-device ");
    const blank = backend.signInAndSelectDevice(" ", "token");
    await Promise.resolve();
    expect((await finish(blank)).device_id).toBe(" chosen-device ");
    expect((await finish(backend.getAppSnapshot())).config).toBeDefined();
  });

  test("exposes the complete application backend shape", () => {
    expect(Object.keys(backend).sort()).toEqual(
      [
        "DeskmateApiError",
        "SESSION_REQUIRED_MESSAGE",
        "isSessionMissing",
        "toApiError",
        "signIn",
        "getAppSnapshot",
        "validateConfigDraft",
        "saveConfig",
        "getNetworkSettings",
        "signInAndSelectDevice",
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

  test("keeps the missing face-list operation rejection", async () => {
    await expect(backend.listCreatableFaces()).rejects.toEqual({
      category: "not-found",
      message: "mock backend has no operation `list_creatable_faces`",
    });
  });

  test("minting does not add a source or distinguish face kinds", async () => {
    const before = await finish(backend.listImageSources());
    const first = await finish(backend.mintImageSource("First", "weather"));
    expect(await finish(backend.mintImageSource("Second"))).toEqual(first);
    expect(first).toEqual({
      source_id: "picture-source-2",
      token: "dev-picture-token-2",
      push_url: "https://deskmate.rodi.one/v1/images/dev-picture-token-2",
    });
    expect(await finish(backend.listImageSources())).toEqual(before);
  });

  test("controls the first timer regardless of id, leaves toggle unchanged, and publishes", async () => {
    const published: AppSnapshot[] = [];
    cleanup.push(await backend.listenToAppState((next) => published.push(next)));
    for (const [action, state] of [
      ["pause", "paused"],
      ["toggle", "paused"],
      ["start", "running"],
      ["reset", "idle"],
    ] as const) {
      const count = published.length;
      const pending = backend.controlPomodoro("missing", action);
      expect(published).toHaveLength(count + 1);
      expect(published.at(-1)?.pomodoros[0].state).toBe(state);
      expect(timers.map((timer) => timer.at - now)).toEqual([90]);
      await finish(pending);
    }
    const timer = published.at(-1)?.pomodoros[0];
    expect(timer?.remaining_seconds).toBe(timer?.duration_seconds);
  });

  test("updates known face fields and rejects a missing source", async () => {
    const face = await finish(
      backend.updateImageSourceFace("studio", { place: "Tbilisi", unknown: "ignored" }),
    );
    expect(face.fields.map((field) => field.value)).toEqual(["Tbilisi", "metric"]);
    await expect(backend.updateImageSourceFace("missing", {})).rejects.toEqual({
      category: "not-found",
      message: "This picture source no longer exists.",
    });
    expect((await finish(backend.listImageSources()))[0].face).toBeNull();
  });
});
