import { afterEach, beforeEach, expect, test } from "bun:test";
import { act } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { App } from "../src/App";

import type {
  AppConfig,
  AppSnapshot,
  DraftValidation,
  NetworkSettings,
  ValidationIssue,
} from "../src/lib/types";
import { apiContractFixtures } from "../src/lib/types.contract";

import { backendModule, backendMocks, resetBackendMocks } from "./support/backendMock";
import { snapshot, clockCard, pictureCard, pomodoroCard, cardListConfig } from "./support/fixtures";
import { installDomLifecycle, waitFor, buttonWithText } from "./support/dom";
import { installWindowClock } from "./support/clock";
import {
  installHttpLifecycle,
  httpState,
  httpCalls,
  httpHandlers,
  expectJsonRequest,
  allowImageMint,
  jsonResponse,
} from "./support/http";

beforeEach(resetBackendMocks);
const { mount, cleanupMountedRoots } = installDomLifecycle();
installHttpLifecycle(cleanupMountedRoots);
const clock = installWindowClock(cleanupMountedRoots);
const VALIDATION_DELAY_MS = 180;
const AUTOSAVE_DELAY_MS = 600;
const SAVE_SETTLE_MS = VALIDATION_DELAY_MS + AUTOSAVE_DELAY_MS;
afterEach(resetBackendMocks);

const previewFrame = { png_base64: "cHJldmlldw==", sample: false, state: null };
const emptyPreviewFrame = { png_base64: null, sample: false, state: null };

test("renders a non-blocking loading state before the first backend snapshot", () => {
  const html = renderToStaticMarkup(<App />);
  expect(html).toContain("Waking the display");
  // The server owns display updates, so closing the page does not interrupt them.
  expect(html).toContain("whether or not this page is open");
});

function recordSaves(config = cardListConfig([clockCard("clock")])) {
  let liveSnapshot: AppSnapshot = {
    ...structuredClone(snapshot),
    config,
    has_saved_config: true,
    pomodoros: [],
    card_data: [],
    card_errors: [],
  };
  const saved: AppConfig[] = [];
  backendMocks.snapshotImpl = async () => liveSnapshot;
  backendMocks.previewImpl = async () => emptyPreviewFrame;
  backendMocks.saveConfigImpl = async (next) => {
    saved.push(next);
    liveSnapshot = { ...liveSnapshot, config: next };
    return { save: { generation: saved.length, warning: null } };
  };
  return saved;
}

test("removing a picture automatically saves the card and source removal without a save click", async () => {
  const saved = recordSaves(cardListConfig([clockCard("clock"), pictureCard()]));
  const { container } = await mount(<App />);
  await clock.advance(SAVE_SETTLE_MS * 2);
  expect(saved).toHaveLength(0);
  expect(container.textContent).toContain("Everything is saved");
  expect(buttonWithText(container, "Save to server")).toBeUndefined();
  expect(buttonWithText(container, "Try again")).toBeUndefined();

  const remove = container.querySelector<HTMLButtonElement>(
    'button[aria-label="Remove Picture — Claude limits"]',
  );
  expect(remove).not.toBeNull();
  await act(async () => remove?.click());
  expect(container.querySelectorAll(".card-tile__body")).toHaveLength(1);
  await clock.advance(VALIDATION_DELAY_MS);
  expect(container.textContent).toContain("Saving shortly…");
  await clock.advance(AUTOSAVE_DELAY_MS - 1);
  expect(saved).toHaveLength(0);
  await clock.advance(1);
  expect(saved).toHaveLength(1);
  expect(saved[0].cards.map((card) => card.id)).toEqual(["clock"]);
  expect(saved[0].image_sources).toEqual([]);
  expect(container.textContent).toContain(
    "Saved to the server. The server will update your display.",
  );
  await clock.advance(SAVE_SETTLE_MS * 2);
  expect(saved).toHaveLength(1);
});

test("a burst of edits within the delay sends exactly one save containing the latest draft", async () => {
  const saved = recordSaves();
  const { container } = await mount(<App />);
  await act(async () => buttonWithText(container, "Manual")?.click());
  await clock.advance(250);
  expect(saved).toHaveLength(0);
  await act(async () => buttonWithText(container, "Timed")?.click());
  await clock.advance(250);
  expect(saved).toHaveLength(0);
  const seconds = container.querySelector<HTMLInputElement>('input[type="checkbox"]');
  expect(seconds?.checked).toBe(true);
  await act(async () => seconds?.click());
  await clock.advance(SAVE_SETTLE_MS - 1);
  expect(saved).toHaveLength(0);
  await clock.advance(1);
  expect(saved).toHaveLength(1);
  expect(saved[0].advance.kind).toBe("timed");
  expect(saved[0].cards[0]).toMatchObject({ id: "clock", show_seconds: false });
  await clock.advance(SAVE_SETTLE_MS * 2);
  expect(saved).toHaveLength(1);
});

test("an invalid draft is never auto-saved and can save after its issue is corrected", async () => {
  const saved = recordSaves();
  backendMocks.validateImpl = async (config) =>
    config.preferences.timezone === "Invalid/Timezone"
      ? {
          valid: false,
          issues: [
            {
              path: "preferences.timezone",
              code: "invalid-timezone",
              message: "Unknown timezone.",
            },
          ],
        }
      : { valid: true, issues: [] };
  const { container } = await mount(<App />);
  await clock.advance(VALIDATION_DELAY_MS);
  await act(async () => container.querySelector<HTMLButtonElement>(".topbar__settings")?.click());
  const timezone = container.querySelector<HTMLInputElement>('input[list="common-timezones"]');
  expect(timezone).not.toBeNull();
  const changeTimezone = async (value: string) =>
    act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(
        timezone,
        value,
      );
      timezone?.dispatchEvent(new Event("input", { bubbles: true }));
    });
  await changeTimezone("Invalid/Timezone");
  await clock.advance(SAVE_SETTLE_MS * 3);
  expect(saved).toHaveLength(0);
  expect(timezone?.value).toBe("Invalid/Timezone");
  expect(container.textContent).toContain("Unknown timezone.");
  expect(container.textContent).toContain("Fix 1 highlighted issue before saving.");
  expect(buttonWithText(container, "Try again")).toBeUndefined();
  await changeTimezone("Europe/Berlin");
  await clock.advance(SAVE_SETTLE_MS);
  expect(saved).toHaveLength(1);
  expect(saved[0].preferences.timezone).toBe("Europe/Berlin");
});

for (const outcome of ["valid", "invalid", "unavailable"] as const) {
  test(`an edit just before autosave waits for its own validation (${outcome})`, async () => {
    const saved = recordSaves();
    const validation = Promise.withResolvers<DraftValidation>();
    const { container } = await mount(<App />);
    await act(async () => buttonWithText(container, "Manual")?.click());
    await clock.advance(VALIDATION_DELAY_MS + AUTOSAVE_DELAY_MS - 1);
    expect(container.textContent).toContain("Saving shortly…");
    backendMocks.validateImpl = () => validation.promise;
    await act(async () => buttonWithText(container, "Timed")?.click());
    await clock.advance(1);
    expect(saved).toHaveLength(0);
    await clock.advance(SAVE_SETTLE_MS * 2);
    expect(saved).toHaveLength(0);
    expect(container.textContent).toContain("Checking settings…");
    await act(async () => {
      if (outcome === "unavailable") validation.reject(new Error("Validation offline"));
      else
        validation.resolve({
          valid: outcome === "valid",
          issues:
            outcome === "valid"
              ? []
              : [{ path: "advance", code: "out-of-range", message: "Invalid pacing." }],
        });
    });
    await clock.advance(AUTOSAVE_DELAY_MS - 1);
    expect(saved).toHaveLength(0);
    await clock.advance(1);
    expect(saved).toHaveLength(outcome === "valid" ? 1 : 0);
    if (outcome === "valid") expect(saved[0].advance.kind).toBe("timed");
    else {
      expect(container.textContent).toContain(
        outcome === "invalid" ? "Invalid pacing." : "Validation unavailable: Validation offline",
      );
      await clock.advance(SAVE_SETTLE_MS * 2);
      expect(saved).toHaveLength(0);
    }
  });
}

for (const retry of ["Try again", "edit"] as const) {
  test(`a failed autosave waits without retrying until ${retry}`, async () => {
    const saved = recordSaves();
    const successfulSave = backendMocks.saveConfigImpl;
    let attempts = 0;
    backendMocks.saveConfigImpl = async (config) => {
      attempts += 1;
      if (attempts === 1) throw new Error("Server unavailable");
      return successfulSave(config);
    };
    const { container } = await mount(<App />);
    await act(async () => buttonWithText(container, "Manual")?.click());
    await clock.advance(SAVE_SETTLE_MS);
    expect(attempts).toBe(1);
    expect(container.textContent).toContain("Server unavailable");
    expect(buttonWithText(container, "Try again")?.disabled).toBe(false);
    await clock.advance(SAVE_SETTLE_MS * 5);
    expect(attempts).toBe(1);
    expect(saved).toHaveLength(0);
    if (retry === "Try again") {
      await act(async () => buttonWithText(container, "Try again")?.click());
    } else {
      await act(async () => buttonWithText(container, "Timed")?.click());
      await clock.advance(SAVE_SETTLE_MS);
    }
    expect(attempts).toBe(2);
    expect(saved).toHaveLength(1);
    expect(saved[0].advance.kind).toBe(retry === "Try again" ? "manual" : "timed");
    expect(container.textContent).toContain(
      "Saved to the server. The server will update your display.",
    );
    expect(buttonWithText(container, "Try again")).toBeUndefined();
    await clock.advance(SAVE_SETTLE_MS * 2);
    expect(attempts).toBe(2);
  });
}

test("a new edit blocks an already-due autosave before validation state paints", async () => {
  const saved = recordSaves();
  const { container } = await mount(<App />);
  await act(async () => buttonWithText(container, "Manual")?.click());
  await clock.advance(VALIDATION_DELAY_MS + AUTOSAVE_DELAY_MS - 1);
  await act(async () => {
    buttonWithText(container, "Timed")?.click();
    clock.tick(1);
  });
  expect(saved).toHaveLength(0);
  await clock.advance(SAVE_SETTLE_MS);
  expect(saved).toHaveLength(1);
  expect(saved[0].advance.kind).toBe("timed");
});

test("the unload warning lasts while dirty and is removed after saving or unmounting", async () => {
  recordSaves();
  const { container, cleanup } = await mount(<App />);
  const warnsOnUnload = () => {
    const event = new Event("beforeunload", { cancelable: true });
    window.dispatchEvent(event);
    return event.defaultPrevented;
  };
  expect(warnsOnUnload()).toBe(false);
  await act(async () => buttonWithText(container, "Manual")?.click());
  expect(warnsOnUnload()).toBe(true);
  await clock.advance(SAVE_SETTLE_MS);
  expect(warnsOnUnload()).toBe(false);
  await act(async () => buttonWithText(container, "Timed")?.click());
  expect(warnsOnUnload()).toBe(true);
  await cleanup();
  expect(warnsOnUnload()).toBe(false);
});

test("the page mints and adds a picture while keeping the token ephemeral", async () => {
  allowImageMint("Picture");
  const clock = clockCard("clock", "Desk");
  backendMocks.snapshotImpl = async () => ({
    ...snapshot,
    config: cardListConfig([clock]),
    device: { ...snapshot.device, tier: "networked" },
    pomodoros: [],
    card_data: [],
    card_errors: [],
  });
  backendMocks.previewImpl = async () => emptyPreviewFrame;
  httpCalls.length = 0;

  const { container } = await mount(<App />);
  await act(async () => container.querySelector<HTMLButtonElement>(".card-tile__add")?.click());
  const pictureMenuItem = [
    ...container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]'),
  ].find((button) => button.textContent?.includes("New picture source"));
  await act(async () => pictureMenuItem?.click());

  await waitFor(() => {
    expect(container.querySelectorAll(".card-tile__body")).toHaveLength(2);
    expect(container.querySelector<HTMLInputElement>("#picture-source-token")?.value).toBe(
      "plaintext-once",
    );
  });
  expect(httpCalls).toContainEqual({
    method: "POST",
    path: "/v1/images",
    body: { name: "Picture", face_kind: null },
  });

  const tiles = container.querySelectorAll<HTMLButtonElement>(".card-tile__body");
  await act(async () => tiles[0]?.click());
  expect(container.querySelector("#picture-source-token")).toBeNull();
  await act(async () => tiles[1]?.click());
  expect(container.querySelector("#picture-source-token")).toBeNull();
});

test("the page mints a server-listed face as a picture without producer credentials", async () => {
  const mint = Promise.withResolvers<Response>();
  httpHandlers.set("POST /v1/images", (body, init) => {
    expectJsonRequest(init);
    expect(body).toEqual({ name: "Weather", face_kind: "weather" });
    return mint.promise;
  });
  const clock = clockCard("clock", "Desk");
  backendMocks.snapshotImpl = async () => ({
    ...snapshot,
    config: cardListConfig([clock]),
    device: { ...snapshot.device, tier: "networked" },
    pomodoros: [],
    card_data: [],
    card_errors: [],
  });
  httpState.creatableFacesResponse = [{ kind: "weather", label: "Weather", fields: [] }];
  backendMocks.previewImpl = async () => emptyPreviewFrame;
  httpCalls.length = 0;

  const { container } = await mount(<App />);
  await act(async () => container.querySelector<HTMLButtonElement>(".card-tile__add")?.click());
  await waitFor(() => expect(container.textContent).toContain("Weather"));
  const weather = [...container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(
    (button) => button.querySelector("strong")?.textContent === "Weather",
  );
  await act(async () => weather?.click());
  expect(container.querySelector('[role="menu"]') === null).toBe(true);
  const add = container.querySelector<HTMLButtonElement>(".card-tile__add");
  expect(add?.disabled).toBe(true);
  await act(async () => add?.click());
  expect(container.querySelector('[role="menu"]')).toBeNull();
  expect(httpCalls.filter((call) => call.path === "/v1/images")).toHaveLength(1);
  await act(async () =>
    mint.resolve(jsonResponse({ id: "picture-source", token: "plaintext-once" })),
  );

  await waitFor(() => {
    const tiles = container.querySelectorAll<HTMLButtonElement>(".card-tile__body");
    expect(tiles).toHaveLength(2);
    expect(tiles[1]?.textContent).toContain("Weather");
    expect(tiles[1]?.getAttribute("aria-pressed")).toBe("true");
    expect(container.querySelector("#editor-heading")?.textContent).toBe("Weather");
    expect(container.textContent).toContain("Picture source");
    expect(container.textContent).toContain("picture-source");
  });
  expect(httpCalls).toContainEqual({
    method: "POST",
    path: "/v1/images",
    body: { name: "Weather", face_kind: "weather" },
  });
  expect(container.textContent).toContain("Weather");
  expect(container.querySelector("#picture-push-url")).toBeNull();
  expect(container.querySelector("#picture-source-token")).toBeNull();
});

test("editing an already-saved config does not resurface first-run guidance", async () => {
  let liveSnapshot = {
    ...(structuredClone(snapshot) as AppSnapshot),
    has_saved_config: true,
  };
  const saved: AppConfig[] = [];
  backendMocks.snapshotImpl = async () => liveSnapshot;
  backendMocks.validateImpl = async () => ({ valid: true, issues: [] });
  backendMocks.saveConfigImpl = async (config) => {
    saved.push(config);
    liveSnapshot = { ...liveSnapshot, config };
    return { save: { generation: 2, warning: null } };
  };
  backendMocks.previewImpl = async () => previewFrame;

  const { container } = await mount(<App />);
  await waitFor(() => expect(buttonWithText(container, "Manual")).toBeDefined());
  expect(container.textContent).not.toContain("Make the display yours");

  const manualTab = buttonWithText(container, "Manual");
  expect(manualTab).not.toBeUndefined();
  await act(async () => manualTab?.click());

  await clock.advance(VALIDATION_DELAY_MS);
  expect(container.textContent).toContain("Saving shortly…");
  expect(container.textContent).not.toContain("Make the display yours");
  expect(buttonWithText(container, "Manual")?.getAttribute("aria-pressed")).toBe("true");
  await clock.advance(AUTOSAVE_DELAY_MS);
  await waitFor(() => expect(saved).toHaveLength(1));
  expect(saved[0].advance).toEqual({ kind: "manual" });
});

test("fresh default settings show first-run guidance until they have been saved", async () => {
  let liveSnapshot = {
    ...(structuredClone(snapshot) as AppSnapshot),
    has_saved_config: false,
  };
  backendMocks.snapshotImpl = async () => liveSnapshot;
  backendMocks.saveConfigImpl = async (config) => {
    liveSnapshot = { ...liveSnapshot, config, has_saved_config: true };
    return { save: { generation: 1, warning: null } };
  };
  backendMocks.previewImpl = async () => previewFrame;

  const { container } = await mount(<App />);
  await waitFor(() => expect(container.textContent).toContain("Make the display yours"));
  expect(container.textContent).toContain("Save your settings");

  await act(async () => buttonWithText(container, "Manual")?.click());
  await clock.advance(VALIDATION_DELAY_MS);
  expect(container.textContent).toContain("Saving shortly…");
  expect(container.textContent).toContain("Make the display yours");
  await clock.advance(AUTOSAVE_DELAY_MS);
  await waitFor(() => expect(container.textContent).not.toContain("Make the display yours"));

  await act(async () => buttonWithText(container, "Timed")?.click());
  await clock.advance(VALIDATION_DELAY_MS);
  expect(container.textContent).toContain("Saving shortly…");
  expect(container.textContent).not.toContain("Make the display yours");
});

test("the mounted app saves an offline display configuration through the server", async () => {
  let liveSnapshot: AppSnapshot = {
    ...(structuredClone(snapshot) as AppSnapshot),
    has_saved_config: true,
    device: {
      ...(structuredClone(snapshot.device) as AppSnapshot["device"]),
      tier: null,
    },
  };
  let serverWrites = 0;
  backendMocks.snapshotImpl = async () => liveSnapshot;
  backendMocks.saveConfigImpl = async (config) => {
    serverWrites += 1;
    liveSnapshot = { ...liveSnapshot, config };
    return { save: { generation: 2, warning: null } };
  };
  backendMocks.previewImpl = async () => previewFrame;

  const { container } = await mount(<App />);
  await waitFor(() => expect(buttonWithText(container, "Manual")).toBeDefined());
  await act(async () => buttonWithText(container, "Manual")?.click());
  await clock.advance(SAVE_SETTLE_MS);
  await waitFor(() => expect(serverWrites).toBe(1));
});

test("no automatic save is sent until server ownership loads", async () => {
  let liveSnapshot: AppSnapshot = {
    ...structuredClone(snapshot),
    device: { ...structuredClone(snapshot.device), tier: null },
  };
  const ownership = Promise.withResolvers<NetworkSettings>();
  const saved: AppConfig[] = [];
  backendMocks.snapshotImpl = async () => liveSnapshot;
  backendMocks.networkSettingsImpl = () => ownership.promise;
  backendMocks.saveConfigImpl = async (config) => {
    saved.push(config);
    liveSnapshot = { ...liveSnapshot, config };
    return { save: { generation: 1, warning: null } };
  };
  backendMocks.previewImpl = async () => previewFrame;

  const { container } = await mount(<App />);
  await act(async () => buttonWithText(container, "Manual")?.click());
  await clock.advance(SAVE_SETTLE_MS * 2);
  expect(saved).toHaveLength(0);
  expect(container.textContent).toContain(
    "Server ownership is unavailable. Check the device link before saving.",
  );
  expect(buttonWithText(container, "Save & apply")).toBeUndefined();
  await act(async () =>
    ownership.resolve({
      server_url: "https://desk.example",
      device_id: "desk-1",
      tier: "networked",
    }),
  );
  await clock.advance(AUTOSAVE_DELAY_MS);
  expect(saved).toHaveLength(1);
  expect(saved[0].advance).toEqual({ kind: "manual" });
});

test("a local tier renders the neutral ownership fallback and never auto-saves", async () => {
  let saves = 0;
  backendMocks.snapshotImpl = async () => ({
    ...(structuredClone(snapshot) as AppSnapshot),
    device: {
      ...(structuredClone(snapshot.device) as AppSnapshot["device"]),
      tier: "local",
    },
  });
  backendMocks.saveConfigImpl = async () => {
    saves += 1;
    return { save: { generation: 1, warning: null } };
  };
  backendMocks.previewImpl = async () => previewFrame;

  const { container } = await mount(<App />);
  await waitFor(() => expect(buttonWithText(container, "Manual")).toBeDefined());
  const settingsButton = [...container.querySelectorAll<HTMLButtonElement>("button")].find(
    (button) => button.textContent?.includes("Settings"),
  );
  await act(async () => settingsButton?.click());
  const badge = container.querySelector(".ownership-badge");
  expect(badge?.textContent).toBe("Ownership unavailable");
  expect(badge?.classList.contains("ownership-badge--unknown")).toBe(true);
  await act(async () => buttonWithText(container, "Manual")?.click());
  await clock.advance(SAVE_SETTLE_MS * 2);
  expect(saves).toBe(0);
});

test("server validation rejections show a bounded issue list beside Try again", async () => {
  const networkedSnapshot: AppSnapshot = {
    ...(structuredClone(snapshot) as AppSnapshot),
    has_saved_config: true,
    device: {
      ...(structuredClone(snapshot.device) as AppSnapshot["device"]),
      tier: "networked",
    },
  };
  backendMocks.snapshotImpl = async () => networkedSnapshot;
  backendMocks.saveConfigImpl = async () => {
    throw new backendModule.DeskmateApiError({
      category: "validation",
      message: "the server rejected this configuration with 7 validation issue(s)",
      issues: Array.from({ length: 7 }, (_, index) => ({
        path: `cards[${index}].title`,
        code: "empty" as const,
        message: `Server issue ${index + 1}.`,
      })),
    });
  };
  backendMocks.previewImpl = async () => previewFrame;

  const { container } = await mount(<App />);
  await waitFor(() => expect(buttonWithText(container, "Manual")).toBeDefined());
  await act(async () => buttonWithText(container, "Manual")?.click());
  await clock.advance(SAVE_SETTLE_MS);

  await waitFor(() =>
    expect(container.textContent).toContain(
      "the server rejected this configuration with 7 validation issue(s)",
    ),
  );
  expect(container.textContent).toContain("Server issue 1.");
  expect(container.textContent).toContain("Server issue 5.");
  expect(container.textContent).not.toContain("Server issue 6.");
  expect(container.textContent).not.toContain("Server issue 7.");
  expect(container.textContent).toContain("and 2 more");
  expect(container.textContent).not.toContain("last working");
  expect(buttonWithText(container, "Try again")?.disabled).toBe(false);
});

test("validation-failed persistence shows the saved-settings banner and issue messages", async () => {
  const invalidSnapshot: AppSnapshot = {
    ...(structuredClone(snapshot) as AppSnapshot),
    persistence: {
      ...structuredClone(apiContractFixtures.snapshot.persistence),
      issues: [
        {
          path: "cards[0].source_id",
          code: "missing-reference",
          message: "The picture card references a missing image source.",
        },
      ],
    },
  };
  backendMocks.snapshotImpl = async () => invalidSnapshot;
  backendMocks.previewImpl = async () => previewFrame;

  const { container } = await mount(<App />);
  await waitFor(() =>
    expect(container.textContent).toContain(
      "Your saved settings failed validation and were not applied",
    ),
  );
  expect(container.textContent).toContain("The picture card references a missing image source.");
  expect(container.textContent).not.toContain("Using your last working settings");
});

test("unsupported saved settings use the generic recoverable-error banner", async () => {
  const message = "config schema version 12 is unsupported; expected 11";
  backendMocks.snapshotImpl = async () => ({
    ...(structuredClone(snapshot) as AppSnapshot),
    has_saved_config: true,
    persistence: { kind: "recoverable-error", message },
  });

  const { container } = await mount(<App />);
  await waitFor(() => expect(container.textContent).toContain("Settings file needs attention"));
  expect(container.textContent).toContain(message);
  expect(container.textContent).toContain("The unreadable file was left untouched");
  expect(container.textContent).not.toContain("Make the display yours");
});

test("a display speaking the server's own protocol raises nothing", async () => {
  // Compatibility compares the device with the server-reported version rather
  // than a client literal, so matching peers never produce a false warning.
  backendMocks.snapshotImpl = async () => ({
    ...(structuredClone(snapshot) as AppSnapshot),
    host_protocol_version: 2,
    device: { ...snapshot.device, protocol_version: 2 },
  });
  backendMocks.previewImpl = async () => previewFrame;

  const { container } = await mount(<App />);
  // Wait for the main page before asserting the absence of a warning.
  await waitFor(() =>
    expect(
      [...container.querySelectorAll("button")].some((button) =>
        button.textContent?.includes("Settings"),
      ),
    ).toBe(true),
  );
  expect(container.textContent).not.toContain("speaks protocol");
});

test("a protocol the server cannot speak is stated in the work column and flags Settings", async () => {
  // A mismatch must remain visible without opening Settings. A device ahead of
  // the server is the realistic shape: firmware is flashed first.
  backendMocks.snapshotImpl = async () => ({
    ...(structuredClone(snapshot) as AppSnapshot),
    host_protocol_version: 2,
    device: { ...snapshot.device, protocol_version: 3 },
  });
  backendMocks.previewImpl = async () => previewFrame;

  const { container } = await mount(<App />);
  await waitFor(() =>
    expect(container.textContent).toContain(
      "This display speaks protocol 3; this server speaks protocol 2.",
    ),
  );
  // And the door to the settings sheet says something is wrong with it.
  expect(container.querySelector(".topbar__settings.has-attention")).not.toBeNull();
});

test.each([
  {
    name: "the global card banner does not blame the display for a host-side scene failure",
    cardErrors: [
      {
        kind: "scene-refused" as const,
        card_id: "clock",
        message: "the configured timezone is not recognized",
      },
    ],
    heading: "One card could not be rendered",
    message: "the configured timezone is not recognized",
    absent: "The display refused one card update",
  },
  {
    name: "the global card banner names the display only for a typed data refusal",
    cardErrors: structuredClone(apiContractFixtures.snapshot.card_errors),
    heading: "The display refused one card update",
    message: apiContractFixtures.snapshot.card_errors[0].message,
    absent: "One card could not be rendered",
  },
])("$name", async ({ cardErrors, heading, message, absent }) => {
  backendMocks.snapshotImpl = async () => ({
    ...(structuredClone(snapshot) as AppSnapshot),
    card_errors: structuredClone([...cardErrors]),
  });
  backendMocks.previewImpl = async () => previewFrame;

  const { container } = await mount(<App />);
  await waitFor(() => expect(container.textContent).toContain(heading));
  expect(container.textContent).toContain(message);
  expect(container.textContent).not.toContain(absent);
});

test("the mounted app routes validation issues to their visible owning surfaces", async () => {
  const config = cardListConfig([pomodoroCard("first", "Desk"), pomodoroCard("second", "Up next")]);
  const issues: ValidationIssue[] = [
    { path: "cards", code: "empty", message: "The cards container needs attention." },
    { path: "cards[0].label", code: "too-long", message: "The first timer label is too long." },
    { path: "cards[1]", code: "invalid-composition", message: "The second timer needs attention." },
    {
      path: "cards[1].dwell_seconds",
      code: "out-of-range",
      message: "The second timer dwell is invalid.",
    },
    {
      path: "advance.default_dwell_seconds",
      code: "out-of-range",
      message: "The loop pacing is invalid.",
    },
    {
      path: "preferences.timezone",
      code: "invalid-timezone",
      message: "The display timezone is unknown.",
    },
    {
      path: "device.capabilities",
      code: "requires-capability",
      message: "The connected firmware does not support asset transfer.",
    },
  ];
  backendMocks.snapshotImpl = async () => ({ ...snapshot, config, pomodoros: [], card_errors: [] });
  backendMocks.validateImpl = async () => ({ valid: false, issues });
  backendMocks.previewImpl = async () => emptyPreviewFrame;
  const { container } = await mount(<App />);
  await clock.advance(VALIDATION_DELAY_MS);
  await waitFor(() =>
    expect(container.querySelector(".library > .field-errors")?.textContent).toBe(
      issues[0].message,
    ),
  );
  const tiles = [...container.querySelectorAll<HTMLLIElement>(".card-tile")];
  const first = tiles.find((tile) => tile.textContent?.includes("Desk"));
  const second = tiles.find((tile) => tile.textContent?.includes("Up next"));
  if (!first || !second) throw new Error("Both timer tiles must be rendered");
  expect(first.querySelector(".card-tile__issues")?.textContent).toBe(issues[1].message);
  expect(first.textContent).not.toContain(issues[2].message);
  expect(first.textContent).not.toContain(issues[3].message);
  expect(second.querySelector(".card-tile__issues")?.textContent).toBe(
    issues[2].message + issues[3].message,
  );
  expect(second.textContent).not.toContain(issues[1].message);
  expect(container.querySelector("#loop-pacing-issues")?.textContent).toBe(issues[4].message);

  const selectSecond = second.querySelector<HTMLButtonElement>(".card-tile__body");
  if (!selectSecond) throw new Error("Missing second timer selection control");
  await act(async () => selectSecond.click());
  const editor = container.querySelector('[aria-labelledby="editor-heading"]');
  expect(editor?.querySelector("#editor-heading")?.textContent).toBe("Up next");
  expect(
    editor
      ?.querySelector('input[aria-label="Stays on the panel for Up next"]')
      ?.closest("label")
      ?.querySelector(".field-errors")?.textContent,
  ).toBe(issues[3].message);

  const settings = container.querySelector<HTMLButtonElement>(".topbar__settings");
  if (!settings) throw new Error("Missing Settings control");
  await act(async () => settings.click());
  const dialog = container.querySelector("dialog");
  expect(dialog?.open).toBe(true);
  const timezone = dialog?.querySelector('input[list="common-timezones"]');
  expect(timezone?.closest("label")?.querySelector(".field-error")?.textContent).toBe(
    issues[5].message,
  );

  const heading = [...container.querySelectorAll("aside strong")].find(
    (node) => node.textContent === "One more thing needs attention",
  );
  const fallback = heading?.closest("aside");
  expect(fallback).toBeDefined();
  expect([...(fallback?.querySelectorAll("p") ?? [])].map((node) => node.textContent)).toEqual([
    issues[6].message,
  ]);
  for (const issue of issues.slice(0, -1))
    expect(fallback?.textContent).not.toContain(issue.message);
});

for (const outcome of ["success", "rejection"] as const) {
  test(`a pending save ${outcome} preserves newer edits and automatically saves them next`, async () => {
    let liveSnapshot: AppSnapshot = structuredClone(snapshot);
    const pending = Promise.withResolvers<void>();
    const refreshing = Promise.withResolvers<AppSnapshot>();
    let refreshStarted = false;
    const saved: AppConfig[] = [];
    backendMocks.snapshotImpl = async () => liveSnapshot;
    backendMocks.previewImpl = async () => emptyPreviewFrame;
    backendMocks.saveConfigImpl = async (config) => {
      saved.push(config);
      if (saved.length === 1) {
        await pending.promise;
        liveSnapshot = { ...liveSnapshot, config };
        backendMocks.snapshotImpl = () => {
          refreshStarted = true;
          return refreshing.promise;
        };
      } else {
        liveSnapshot = { ...liveSnapshot, config };
      }
      return { save: { generation: saved.length, warning: null } };
    };
    const { container } = await mount(<App />);
    await act(async () => buttonWithText(container, "Manual")?.click());
    await clock.advance(SAVE_SETTLE_MS);
    expect(saved).toHaveLength(1);
    expect(saved[0].advance).toEqual({ kind: "manual" });
    await act(async () => buttonWithText(container, "Timed")?.click());
    await clock.advance(SAVE_SETTLE_MS * 2);
    expect(saved).toHaveLength(1);
    expect(container.querySelector(".save-bar")?.textContent).toContain("Saving…");
    if (outcome === "success") {
      await act(async () => pending.resolve());
      await waitFor(() => expect(refreshStarted).toBe(true));
      expect(buttonWithText(container, "Timed")?.getAttribute("aria-pressed")).toBe("true");
      await clock.advance(SAVE_SETTLE_MS * 2);
      expect(saved).toHaveLength(1);
      await act(async () => refreshing.resolve(liveSnapshot));
    } else {
      await act(async () => pending.reject(new Error("old draft rejected")));
    }
    expect(container.textContent).not.toContain("old draft rejected");
    expect(buttonWithText(container, "Timed")?.getAttribute("aria-pressed")).toBe("true");
    expect(container.textContent).toContain("Saving shortly…");
    backendMocks.snapshotImpl = async () => liveSnapshot;
    await clock.advance(AUTOSAVE_DELAY_MS);
    expect(saved).toHaveLength(2);
    expect(saved[1].advance.kind).toBe("timed");
    expect(container.textContent).toContain(
      "Saved to the server. The server will update your display.",
    );
  });
}

test("both SaveBar retries reject reentry before paint and share saving status after a pending edit", async () => {
  const pending = Promise.withResolvers<void>();
  let saves = 0;
  backendMocks.saveConfigImpl = async () => {
    saves += 1;
    if (saves === 1) throw new Error("Save failed");
    await pending.promise;
    return { save: { generation: 1, warning: null } };
  };
  const { container } = await mount(<App />);
  await act(async () => buttonWithText(container, "Manual")?.click());
  await act(async () => container.querySelector<HTMLButtonElement>(".topbar__settings")?.click());
  await clock.advance(SAVE_SETTLE_MS);
  const savesButtons = [...container.querySelectorAll<HTMLButtonElement>("button")].filter(
    (button) => button.textContent === "Try again",
  );
  expect(savesButtons).toHaveLength(2);
  expect(savesButtons.every((button) => !button.disabled)).toBe(true);
  await act(async () => {
    savesButtons[0].click();
    savesButtons[1].click();
  });
  expect(saves).toBe(2);
  await act(async () => buttonWithText(container, "Timed")?.click());
  await clock.advance(SAVE_SETTLE_MS * 2);
  expect(saves).toBe(2);
  expect(buttonWithText(container, "Try again")).toBeUndefined();
  const bars = [...container.querySelectorAll(".save-bar")];
  expect(bars).toHaveLength(2);
  expect(bars.every((bar) => bar.textContent === "Saving…")).toBe(true);
  await act(async () => pending.resolve());
});

function sourceRaceSnapshot(cardCount = 1): AppSnapshot {
  const config = cardListConfig(
    Array.from({ length: cardCount }, (_, index) => clockCard(`clock-${index}`)),
  );
  config.preferences.paused = true;
  config.image_sources = [{ id: "existing-source", name: "Existing picture" }];
  return {
    ...snapshot,
    config,
    runtime: { kind: "running" },
    pomodoros: [],
    card_data: [],
    card_errors: [],
  };
}

function menuItem(container: ParentNode, label: string): HTMLButtonElement {
  const item = [...container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(
    (button) => button.querySelector("strong")?.textContent === label,
  );
  if (!item) throw new Error(`Missing menu item: ${label}`);
  return item;
}

for (const write of ["save", "resume"] as const) {
  for (const source of ["New picture source", "Weather"]) {
    test(`pending ${write} excludes ${source} minting before paint and permits ordinary additions`, async () => {
      let liveSnapshot = sourceRaceSnapshot();
      const pending = Promise.withResolvers<void>();
      const saved: AppConfig[] = [];
      let resumes = 0;
      backendMocks.snapshotImpl = async () => liveSnapshot;
      backendMocks.saveConfigImpl = async (config) => {
        saved.push(config);
        await pending.promise;
        liveSnapshot = { ...liveSnapshot, config };
        return { save: { generation: saved.length, warning: null } };
      };
      backendMocks.resumeImpl = async () => {
        resumes += 1;
        await pending.promise;
      };
      httpState.creatableFacesResponse = [{ kind: "weather", label: "Weather", fields: [] }];
      httpHandlers.set("POST /v1/images", () =>
        jsonResponse({ id: "picture-source", token: "plaintext-once" }),
      );
      const { container } = await mount(<App />);
      if (write === "save") {
        await act(async () => buttonWithText(container, "Manual")?.click());
        await clock.advance(VALIDATION_DELAY_MS);
        expect(container.textContent).toContain("Saving shortly…");
      } else {
        expect(buttonWithText(container, "Resume sending")?.disabled).toBe(false);
      }
      await act(async () => container.querySelector<HTMLButtonElement>(".card-tile__add")?.click());
      const mint = menuItem(container, source);
      await act(async () => {
        if (write === "save") clock.tick(AUTOSAVE_DELAY_MS);
        else buttonWithText(container, "Resume sending")?.click();
        mint.click();
      });
      expect(saved.length + resumes).toBe(1);
      expect(httpCalls.filter((call) => call.path === "/v1/images")).toHaveLength(0);
      await act(async () => container.querySelector<HTMLButtonElement>(".card-tile__add")?.click());
      for (const label of ["New picture source", "Weather"]) {
        expect(menuItem(container, label).disabled).toBe(true);
        await act(async () => menuItem(container, label).click());
      }
      expect(menuItem(container, "Digital clock").disabled).toBe(false);
      expect(menuItem(container, "Pomodoro").disabled).toBe(false);
      expect(menuItem(container, "Existing picture").disabled).toBe(false);
      await act(async () => menuItem(container, "Digital clock").click());
      expect(container.querySelectorAll(".card-tile__body")).toHaveLength(2);
      await clock.advance(SAVE_SETTLE_MS * 2);
      expect(saved.length + resumes).toBe(1);
      await act(async () => pending.resolve());
      expect(container.querySelectorAll(".card-tile__body")).toHaveLength(2);
      await act(async () => container.querySelector<HTMLButtonElement>(".card-tile__add")?.click());
      expect(menuItem(container, source).disabled).toBe(false);
      await act(async () => menuItem(container, source).click());
      expect(httpCalls.filter((call) => call.path === "/v1/images")).toHaveLength(1);
      await clock.advance(SAVE_SETTLE_MS);
      expect(saved).toHaveLength(write === "save" ? 2 : 1);
      const config = saved.at(-1);
      expect(config?.cards).toHaveLength(3);
      expect(config?.cards.at(-1)).toMatchObject({ kind: "picture", source_id: "picture-source" });
      expect(config?.image_sources).toContainEqual({
        id: "picture-source",
        name: source === "Weather" ? "Weather" : "Picture",
      });
    });
  }
}

for (const trigger of ["autosave", "retry"] as const) {
  test(`pending mint excludes ${trigger} and Resume sending before paint until draft insertion`, async () => {
    let liveSnapshot = sourceRaceSnapshot();
    const mint = Promise.withResolvers<Response>();
    const saved: AppConfig[] = [];
    let resumes = 0;
    backendMocks.snapshotImpl = async () => liveSnapshot;
    backendMocks.saveConfigImpl = async (config) => {
      saved.push(config);
      if (trigger === "retry" && saved.length === 1) throw new Error("Save failed");
      liveSnapshot = { ...liveSnapshot, config };
      return { save: { generation: 1, warning: null } };
    };
    backendMocks.resumeImpl = async () => {
      resumes += 1;
    };
    httpHandlers.set("POST /v1/images", () => mint.promise);
    const { container } = await mount(<App />);
    await act(async () => buttonWithText(container, "Manual")?.click());
    await act(async () => container.querySelector<HTMLButtonElement>(".topbar__settings")?.click());
    await clock.advance(trigger === "retry" ? SAVE_SETTLE_MS : VALIDATION_DELAY_MS);
    const saves = [...container.querySelectorAll<HTMLButtonElement>("button")].filter(
      (button) => button.textContent === "Try again",
    );
    expect(saves).toHaveLength(trigger === "retry" ? 2 : 0);
    await act(async () => container.querySelector<HTMLButtonElement>(".card-tile__add")?.click());
    const create = menuItem(container, "New picture source");
    await act(async () => {
      create.click();
      create.click();
      if (trigger === "autosave") clock.tick(AUTOSAVE_DELAY_MS);
      for (const save of saves) save.click();
      buttonWithText(container, "Resume sending")?.click();
    });
    const attemptsBeforeMint = trigger === "retry" ? 1 : 0;
    expect(saved).toHaveLength(attemptsBeforeMint);
    expect(resumes).toBe(0);
    expect(httpCalls.filter((call) => call.path === "/v1/images")).toHaveLength(1);
    expect(saves.every((save) => save.disabled)).toBe(true);
    expect(buttonWithText(container, "Resume sending")?.disabled).toBe(true);
    await clock.advance(SAVE_SETTLE_MS * 2);
    expect(saved).toHaveLength(attemptsBeforeMint);
    await act(async () => mint.resolve(jsonResponse({ id: "picture-source", token: "once" })));
    expect(container.querySelectorAll(".card-tile__body")).toHaveLength(2);
    await clock.advance(SAVE_SETTLE_MS);
    expect(saved).toHaveLength(attemptsBeforeMint + 1);
    expect(saved.at(-1)?.cards.at(-1)).toMatchObject({
      kind: "picture",
      source_id: "picture-source",
    });
    expect(saved.at(-1)?.image_sources).toContainEqual({ id: "picture-source", name: "Picture" });
  });
}

test("pending mint reserves the seventh draft's final card slot", async () => {
  const mint = Promise.withResolvers<Response>();
  backendMocks.snapshotImpl = async () => sourceRaceSnapshot(7);
  httpHandlers.set("POST /v1/images", () => mint.promise);
  const { container } = await mount(<App />);
  const add = container.querySelector<HTMLButtonElement>(".card-tile__add");
  await act(async () => add?.click());
  await act(async () => menuItem(container, "New picture source").click());
  await act(async () => add?.click());
  const competing = [...container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(
    (button) => button.querySelector("strong")?.textContent === "Digital clock",
  );
  await act(async () => competing?.click());
  expect(container.querySelectorAll(".card-tile__body")).toHaveLength(7);
  expect(add?.disabled).toBe(true);
  expect(container.querySelector('[role="menu"]')).toBeNull();
  await act(async () => mint.resolve(jsonResponse({ id: "picture-source", token: "once" })));
  expect(container.querySelectorAll(".card-tile__body")).toHaveLength(8);
  expect(container.querySelector<HTMLInputElement>("#picture-source-token")?.value).toBe("once");
});

for (const operation of ["mint", "save", "resume"] as const) {
  test(`a rejected ${operation} releases the source/configuration gate`, async () => {
    const pending = Promise.withResolvers<void>();
    let liveSnapshot = sourceRaceSnapshot();
    const saved: AppConfig[] = [];
    backendMocks.snapshotImpl = async () => liveSnapshot;
    backendMocks.saveConfigImpl = async (config) => {
      saved.push(config);
      await pending.promise;
      return { save: { generation: 1, warning: null } };
    };
    backendMocks.resumeImpl = () => pending.promise;
    httpHandlers.set("POST /v1/images", async () => {
      await pending.promise;
      return jsonResponse({ id: "picture-source", token: "once" });
    });
    const { container } = await mount(<App />);
    if (operation !== "resume") {
      await act(async () => buttonWithText(container, "Manual")?.click());
      await clock.advance(VALIDATION_DELAY_MS);
    } else {
      expect(buttonWithText(container, "Resume sending")?.disabled).toBe(false);
    }
    if (operation === "mint") {
      await act(async () => container.querySelector<HTMLButtonElement>(".card-tile__add")?.click());
      await act(async () => menuItem(container, "New picture source").click());
      await clock.advance(SAVE_SETTLE_MS * 2);
      expect(saved).toHaveLength(0);
    } else if (operation === "save") {
      await clock.advance(AUTOSAVE_DELAY_MS);
      expect(saved).toHaveLength(1);
    } else {
      await act(async () => buttonWithText(container, "Resume sending")?.click());
    }
    await act(async () => pending.reject(new Error(`${operation} rejected`)));
    expect(container.textContent).toContain(`${operation} rejected`);
    expect(buttonWithText(container, "Resume sending")?.disabled).toBe(operation !== "resume");
    if (operation === "resume") {
      await act(async () => buttonWithText(container, "Manual")?.click());
    }
    if (operation === "save") {
      expect(buttonWithText(container, "Try again")?.disabled).toBe(false);
    }
    expect(buttonWithText(container, "Resume sending")?.disabled).toBe(true);
    backendMocks.saveConfigImpl = async (config) => {
      saved.push(config);
      liveSnapshot = { ...liveSnapshot, config };
      return { save: { generation: 2, warning: null } };
    };
    allowImageMint("Picture");
    await act(async () => container.querySelector<HTMLButtonElement>(".card-tile__add")?.click());
    expect(menuItem(container, "New picture source").disabled).toBe(false);
    await act(async () => menuItem(container, "New picture source").click());
    expect(container.querySelectorAll(".card-tile__body")).toHaveLength(2);
    await clock.advance(SAVE_SETTLE_MS);
    expect(saved).toHaveLength(operation === "save" ? 2 : 1);
    expect(saved.at(-1)?.cards.at(-1)).toMatchObject({
      kind: "picture",
      source_id: "picture-source",
    });
    expect(saved.at(-1)?.image_sources).toContainEqual({ id: "picture-source", name: "Picture" });
  });
}

test("a completed mint keeps Resume disabled until the picture draft is saved", async () => {
  let liveSnapshot = sourceRaceSnapshot();
  const mint = Promise.withResolvers<Response>();
  const saving = Promise.withResolvers<void>();
  const resumed: AppConfig[] = [];
  backendMocks.snapshotImpl = async () => liveSnapshot;
  backendMocks.saveConfigImpl = async (config) => {
    await saving.promise;
    liveSnapshot = { ...liveSnapshot, config };
    return { save: { generation: 1, warning: null } };
  };
  backendMocks.resumeImpl = async () => {
    resumed.push(liveSnapshot.config);
  };
  httpHandlers.set("POST /v1/images", () => mint.promise);
  const { container } = await mount(<App />);
  const resume = buttonWithText(container, "Resume sending");
  expect(resume?.disabled).toBe(false);
  await act(async () => container.querySelector<HTMLButtonElement>(".card-tile__add")?.click());
  await act(async () => {
    menuItem(container, "New picture source").click();
    resume?.click();
  });
  expect(resumed).toHaveLength(0);
  expect(resume?.disabled).toBe(true);
  await act(async () => mint.resolve(jsonResponse({ id: "picture-source", token: "once" })));
  expect(container.querySelectorAll(".card-tile__body")).toHaveLength(2);
  expect(resume?.disabled).toBe(true);
  expect(container.textContent).toContain("Save your changes before resuming.");
  await act(async () => resume?.click());
  expect(resumed).toHaveLength(0);
  await clock.advance(SAVE_SETTLE_MS);
  expect(resume?.disabled).toBe(true);
  await act(async () => saving.resolve());
  expect(resume?.disabled).toBe(false);
  expect(container.textContent).not.toContain("Save your changes before resuming.");
  await act(async () => resume?.click());
  expect(resumed).toHaveLength(1);
  expect(resumed[0].cards.at(-1)).toMatchObject({ kind: "picture", source_id: "picture-source" });
  expect(resumed[0].image_sources).toContainEqual({ id: "picture-source", name: "Picture" });
});

test("Resume rejects a draft edit before its disabled state paints", async () => {
  let resumes = 0;
  backendMocks.snapshotImpl = async () => sourceRaceSnapshot();
  backendMocks.resumeImpl = async () => {
    resumes += 1;
  };
  const { container } = await mount(<App />);
  expect(buttonWithText(container, "Resume sending")?.disabled).toBe(false);
  await act(async () => {
    buttonWithText(container, "Manual")?.click();
    buttonWithText(container, "Resume sending")?.click();
  });
  expect(resumes).toBe(0);
  expect(buttonWithText(container, "Resume sending")?.disabled).toBe(true);
});

for (const supported of [false, true]) {
  test(`brightness auto-saves while the settings sheet stays open with firmware support=${supported}`, async () => {
    const live: AppSnapshot = structuredClone(snapshot);
    live.device.connection = { kind: "online" };
    live.device.capabilities = supported ? ["display-brightness"] : [];
    live.device.tier = "networked";
    const saved: AppConfig[] = [];
    backendMocks.snapshotImpl = async () => live;
    backendMocks.saveConfigImpl = async (config) => {
      saved.push(config);
      live.config = config;
      return { save: { generation: 1, warning: null } };
    };
    const { container } = await mount(<App />);
    await act(async () => container.querySelector<HTMLButtonElement>(".topbar__settings")?.click());
    expect(container.querySelector("dialog")?.open).toBe(true);
    const slider = container.querySelector<HTMLInputElement>("#display-brightness");
    expect(slider).not.toBeNull();
    expect(
      container.querySelector<HTMLLabelElement>("label[for=display-brightness]")?.control,
    ).toBe(slider);
    expect(slider?.min).toBe("10");
    expect(slider?.max).toBe("100");
    expect(slider?.disabled).toBe(false);
    expect(
      container
        .querySelector("#brightness-help")
        ?.textContent?.includes("does not support brightness yet"),
    ).toBe(!supported);
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(slider, "35");
      slider?.dispatchEvent(new Event("input", { bubbles: true }));
      slider?.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await waitFor(() =>
      expect(container.querySelector("output[for='display-brightness']")?.textContent).toBe("35%"),
    );
    const dialog = container.querySelector("dialog");
    if (!dialog) throw new Error("Settings sheet missing");
    await clock.advance(SAVE_SETTLE_MS);
    expect(dialog.open).toBe(true);
    expect(dialog.textContent).toContain(
      "Saved to the server. The server will update your display.",
    );
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0].preferences.brightness).toBe(35);
    expect(saved[0].schema_version).toBe(11);
  });
}
