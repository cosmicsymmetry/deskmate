import { afterEach, beforeEach, expect, test } from "bun:test";
import { act } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { App } from "../src/App";

import type { AppConfig, AppSnapshot, NetworkSettings, ValidationIssue } from "../src/lib/types";
import { apiContractFixtures } from "../src/lib/types.contract";

import { backendModule, backendMocks, resetBackendMocks } from "./support/backendMock";
import { snapshot, clockCard, pomodoroCard, cardListConfig } from "./support/fixtures";
import { installDomLifecycle, renderPreviewInto, waitFor, buttonWithText } from "./support/dom";
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
afterEach(resetBackendMocks);

test("renders a non-blocking loading state before the first backend snapshot", () => {
  const html = renderToStaticMarkup(<App />);
  expect(html).toContain("Waking the display");
  // The server owns display updates, so closing the page does not interrupt them.
  expect(html).toContain("whether or not this page is open");
});

test("a missing session offers the way in, not just a retry", async () => {
  // Found by driving the real browser: the screen named the admin token and
  // then gave nowhere to type it, because the sign-in lived inside Settings --
  // which needs the snapshot that had just failed to load. A retry button
  // cannot fix a missing credential, so the screen has to carry the field.
  backendMocks.snapshotImpl = async () => {
    throw new backendModule.DeskmateApiError({
      category: "runtime-unavailable",
      message: backendModule.SESSION_REQUIRED_MESSAGE,
    });
  };
  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<App />));
    await waitFor(() => {
      expect(container.textContent).toContain("Sign in to Deskmate");
      expect(container.querySelector('input[type="password"]')).not.toBeNull();
      expect(buttonWithText(container, "Sign in")).toBeDefined();
    });
    expect(buttonWithText(container, "Try again")).toBeUndefined();
  } finally {
    await cleanup();
    backendMocks.snapshotImpl = async () => snapshot;
  }
});

test("signing in fills the device id the first load could not know", async () => {
  // The first network-settings fetch happens before this browser has a
  // session, so it comes back with no device id. Refreshing only the snapshot
  // left the Settings sheet showing an empty Device ID for the rest of the
  // session -- visible only by actually signing in, which is how this was found.
  let signedIn = false;
  const registrations: string[] = [];
  let publishSnapshot: ((next: AppSnapshot) => void) | undefined;
  httpState.creatableFacesResponse = [{ kind: "future-face", label: "Future face", fields: [] }];
  backendMocks.listenImpl = async (publish) => {
    publishSnapshot = publish;
    registrations.push(signedIn ? "dev-0005" : "");
    if (!signedIn) throw new Error("pre-login listen rejected");
    return () => {};
  };
  httpHandlers.set("POST /v1/app/session", (body, init) => {
    expectJsonRequest(init);
    expect(body).toEqual({ token: "admin-secret" });
    signedIn = true;
    return new Response(null, { status: 204 });
  });
  backendMocks.networkSettingsImpl = async () => ({
    server_url: "https://desk.example",
    device_id: signedIn ? "dev-0005" : "",
    tier: "networked",
  });
  backendMocks.snapshotImpl = async () => {
    if (!signedIn) {
      throw new backendModule.DeskmateApiError({
        category: "runtime-unavailable",
        message: backendModule.SESSION_REQUIRED_MESSAGE,
      });
    }
    return snapshot;
  };

  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<App />));
    await waitFor(() => expect(container.textContent).toContain("Sign in to Deskmate"));
    expect(httpCalls.filter((call) => call.path === "/v1/faces")).toHaveLength(0);

    const token = container.querySelector<HTMLInputElement>('input[type="password"]');
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(
        token,
        "admin-secret",
      );
      token?.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => buttonWithText(container, "Sign in")?.click());

    const settingsButton = () =>
      [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) =>
        button.textContent?.includes("Settings"),
      );
    await waitFor(() => expect(settingsButton()).toBeDefined());
    expect(httpCalls.filter((call) => call.path === "/v1/app/session")).toHaveLength(1);
    expect(registrations).toEqual(["", "dev-0005"]);
    expect(httpCalls.filter((call) => call.path === "/v1/faces")).toHaveLength(1);
    await act(async () => container.querySelector<HTMLButtonElement>(".card-tile__add")?.click());
    expect(
      [...container.querySelectorAll('[role="menuitem"] strong')].map((node) => node.textContent),
    ).toContain("Future face");
    await act(async () => publishSnapshot?.({ ...snapshot }));
    expect(httpCalls.filter((call) => call.path === "/v1/faces")).toHaveLength(1);
    await act(async () => settingsButton()?.click());
    await waitFor(() => {
      const deviceId = [...container.querySelectorAll<HTMLInputElement>("input")].find(
        (input) => input.value === "dev-0005",
      );
      expect(deviceId).toBeDefined();
    });
  } finally {
    await cleanup();
    backendMocks.snapshotImpl = async () => snapshot;
    backendMocks.networkSettingsImpl = async () => ({
      server_url: "https://desk.example",
      device_id: "desk-1",
      tier: "networked",
    });
  }
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
  backendMocks.networkSettingsImpl = async () => ({
    server_url: "https://desk.example",
    device_id: "desk-1",
    tier: "networked",
  });
  backendMocks.previewImpl = async () => ({ png_base64: null, sample: false, state: null });
  httpCalls.length = 0;

  const { container, root, cleanup } = await mount();
  try {
    await renderPreviewInto(root, <App />);
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
  } finally {
    await cleanup();
    backendMocks.snapshotImpl = async () => snapshot;
    backendMocks.networkSettingsImpl = async () => ({
      server_url: "https://desk.example",
      device_id: "desk-1",
      tier: "networked",
    });
    backendMocks.previewImpl = () =>
      Promise.reject(new Error("renderCardPreview not configured for this test"));
  }
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
  backendMocks.previewImpl = async () => ({ png_base64: null, sample: false, state: null });
  httpCalls.length = 0;

  const { container, root, cleanup } = await mount();
  try {
    await renderPreviewInto(root, <App />);
    await act(async () => container.querySelector<HTMLButtonElement>(".card-tile__add")?.click());
    await waitFor(() => expect(container.textContent).toContain("Weather"));
    const weather = [...container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(
      (button) => button.querySelector("strong")?.textContent === "Weather",
    );
    await act(async () => weather?.click());
    expect(container.querySelector('[role="menu"]') === null).toBe(true);
    const add = container.querySelector<HTMLButtonElement>(".card-tile__add");
    expect(document.activeElement).toBe(add);
    await act(async () => add?.click());
    const busyFace = [...container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(
      (button) => button.querySelector("strong")?.textContent === "Weather",
    );
    expect(busyFace?.disabled).toBe(true);
    await act(async () => busyFace?.click());
    expect(httpCalls.filter((call) => call.path === "/v1/images")).toHaveLength(1);
    await act(async () => add?.click());
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
  } finally {
    await cleanup();
    backendMocks.snapshotImpl = async () => snapshot;
    httpState.creatableFacesResponse = [];
    backendMocks.previewImpl = () =>
      Promise.reject(new Error("renderCardPreview not configured for this test"));
  }
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
  backendMocks.previewImpl = async () => ({
    png_base64: "cHJldmlldw==",
    sample: false,
    state: null,
  });

  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<App />));
    await waitFor(() => expect(buttonWithText(container, "Manual")).toBeDefined());
    expect(container.textContent).not.toContain("Make the display yours");

    const manualTab = buttonWithText(container, "Manual");
    expect(manualTab).not.toBeUndefined();
    await act(async () => manualTab?.click());

    await waitFor(() => expect(container.textContent).toContain("Unsaved changes"));
    expect(container.textContent).not.toContain("Make the display yours");
    expect(buttonWithText(container, "Manual")?.getAttribute("aria-pressed")).toBe("true");
    await waitFor(() => {
      const save = buttonWithText(container, "Save to server");
      expect(save?.disabled).toBe(false);
    });
    await act(async () => buttonWithText(container, "Save to server")?.click());
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0].advance).toEqual({ kind: "manual" });
  } finally {
    await cleanup();
    backendMocks.snapshotImpl = async () => snapshot;
    backendMocks.validateImpl = async () => ({ valid: true, issues: [] });
    backendMocks.saveConfigImpl = async () => ({ save: { generation: 1, warning: null } });
  }
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
  backendMocks.previewImpl = async () => ({
    png_base64: "cHJldmlldw==",
    sample: false,
    state: null,
  });

  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<App />));
    await waitFor(() => expect(container.textContent).toContain("Make the display yours"));
    expect(container.textContent).toContain("Save your settings");

    await act(async () => buttonWithText(container, "Manual")?.click());
    await waitFor(() => expect(container.textContent).toContain("Unsaved changes"));
    expect(container.textContent).toContain("Make the display yours");
    await waitFor(() => {
      expect(buttonWithText(container, "Save to server")?.disabled).toBe(false);
    });
    await act(async () => buttonWithText(container, "Save to server")?.click());
    await waitFor(() => expect(container.textContent).not.toContain("Make the display yours"));

    await act(async () => buttonWithText(container, "Timed")?.click());
    await waitFor(() => expect(container.textContent).toContain("Unsaved changes"));
    expect(container.textContent).not.toContain("Make the display yours");
  } finally {
    await cleanup();
    backendMocks.snapshotImpl = async () => snapshot;
    backendMocks.saveConfigImpl = async () => ({ save: { generation: 1, warning: null } });
  }
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
  backendMocks.networkSettingsImpl = async () => ({
    server_url: "https://desk.example",
    device_id: "desk-1",
    tier: "networked",
  });
  backendMocks.saveConfigImpl = async (config) => {
    serverWrites += 1;
    liveSnapshot = { ...liveSnapshot, config };
    return { save: { generation: 2, warning: null } };
  };
  backendMocks.previewImpl = async () => ({
    png_base64: "cHJldmlldw==",
    sample: false,
    state: null,
  });

  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<App />));
    await waitFor(() => expect(buttonWithText(container, "Save to server")).toBeDefined());
    await act(async () => buttonWithText(container, "Manual")?.click());
    await waitFor(() => expect(buttonWithText(container, "Save to server")?.disabled).toBe(false));
    await act(async () => buttonWithText(container, "Save to server")?.click());
    await waitFor(() => expect(serverWrites).toBe(1));
  } finally {
    await cleanup();
    backendMocks.snapshotImpl = async () => snapshot;
    backendMocks.networkSettingsImpl = async () => ({
      server_url: "https://desk.example",
      device_id: "desk-1",
      tier: "networked",
    });
    backendMocks.saveConfigImpl = async () => ({ save: { generation: 1, warning: null } });
  }
});

test("the mounted app refuses saving beside the button until ownership loads", async () => {
  backendMocks.snapshotImpl = async () => ({
    ...(structuredClone(snapshot) as AppSnapshot),
    device: {
      ...(structuredClone(snapshot.device) as AppSnapshot["device"]),
      tier: null,
    },
  });
  backendMocks.networkSettingsImpl = () => new Promise<NetworkSettings>(() => {});
  backendMocks.previewImpl = async () => ({
    png_base64: "cHJldmlldw==",
    sample: false,
    state: null,
  });

  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<App />));
    await waitFor(() => expect(buttonWithText(container, "Manual")).toBeDefined());
    const unavailable = buttonWithText(container, "Ownership unavailable");
    expect(unavailable?.disabled).toBe(true);
    expect(container.textContent).toContain(
      "Server ownership is unavailable. Check the device link before saving.",
    );
    expect(buttonWithText(container, "Save & apply")).toBeUndefined();
  } finally {
    await cleanup();
    backendMocks.snapshotImpl = async () => snapshot;
    backendMocks.networkSettingsImpl = async () => ({
      server_url: "https://desk.example",
      device_id: "desk-1",
      tier: "networked",
    });
  }
});

test("a local tier in the server snapshot renders the neutral ownership fallback", async () => {
  backendMocks.snapshotImpl = async () => ({
    ...(structuredClone(snapshot) as AppSnapshot),
    device: {
      ...(structuredClone(snapshot.device) as AppSnapshot["device"]),
      tier: "local",
    },
  });
  backendMocks.previewImpl = async () => ({
    png_base64: "cHJldmlldw==",
    sample: false,
    state: null,
  });

  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<App />));
    await waitFor(() => expect(buttonWithText(container, "Ownership unavailable")).toBeDefined());
    const settingsButton = [...container.querySelectorAll<HTMLButtonElement>("button")].find(
      (button) => button.textContent?.includes("Settings"),
    );
    await act(async () => settingsButton?.click());
    const badge = container.querySelector(".ownership-badge");
    expect(badge?.textContent).toBe("Ownership unavailable");
    expect(badge?.classList.contains("ownership-badge--unknown")).toBe(true);
  } finally {
    await cleanup();
    backendMocks.snapshotImpl = async () => snapshot;
  }
});

test("server validation rejections show a bounded issue list beside the save action", async () => {
  const networkedSnapshot: AppSnapshot = {
    ...(structuredClone(snapshot) as AppSnapshot),
    has_saved_config: true,
    device: {
      ...(structuredClone(snapshot.device) as AppSnapshot["device"]),
      tier: "networked",
    },
  };
  backendMocks.snapshotImpl = async () => networkedSnapshot;
  backendMocks.networkSettingsImpl = async () => ({
    server_url: "https://desk.example",
    device_id: "desk-1",
    tier: "networked",
  });
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
  backendMocks.previewImpl = async () => ({
    png_base64: "cHJldmlldw==",
    sample: false,
    state: null,
  });

  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<App />));
    await waitFor(() => expect(buttonWithText(container, "Manual")).toBeDefined());
    await act(async () => buttonWithText(container, "Manual")?.click());
    await waitFor(() => expect(buttonWithText(container, "Save to server")?.disabled).toBe(false));
    await act(async () => buttonWithText(container, "Save to server")?.click());

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
  } finally {
    await cleanup();
    backendMocks.snapshotImpl = async () => snapshot;
    backendMocks.networkSettingsImpl = async () => ({
      server_url: "https://desk.example",
      device_id: "desk-1",
      tier: "networked",
    });
    backendMocks.saveConfigImpl = async () => ({ save: { generation: 1, warning: null } });
  }
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
  backendMocks.previewImpl = async () => ({
    png_base64: "cHJldmlldw==",
    sample: false,
    state: null,
  });

  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<App />));
    await waitFor(() =>
      expect(container.textContent).toContain(
        "Your saved settings failed validation and were not applied",
      ),
    );
    expect(container.textContent).toContain("The picture card references a missing image source.");
    expect(container.textContent).not.toContain("Using your last working settings");
  } finally {
    await cleanup();
    backendMocks.snapshotImpl = async () => snapshot;
  }
});

test("a display speaking the server's own protocol raises nothing", async () => {
  // Compatibility compares the device with the server-reported version rather
  // than a client literal, so matching peers never produce a false warning.
  backendMocks.snapshotImpl = async () => ({
    ...(structuredClone(snapshot) as AppSnapshot),
    host_protocol_version: 2,
    device: { ...snapshot.device, protocol_version: 2 },
  });
  backendMocks.previewImpl = async () => ({
    png_base64: "cHJldmlldw==",
    sample: false,
    state: null,
  });

  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<App />));
    // Wait for the main page, then assert by absence. The save button's
    // label depends on ownership, so it is the wrong thing to wait on here.
    await waitFor(() =>
      expect(
        [...container.querySelectorAll("button")].some((button) =>
          button.textContent?.includes("Settings"),
        ),
      ).toBe(true),
    );
    expect(container.textContent).not.toContain("speaks protocol");
  } finally {
    await cleanup();
    backendMocks.snapshotImpl = async () => snapshot;
  }
});

test("a protocol the server cannot speak is stated in the work column and flags Settings", async () => {
  // A mismatch must remain visible without opening Settings. A device ahead of
  // the server is the realistic shape: firmware is flashed first.
  backendMocks.snapshotImpl = async () => ({
    ...(structuredClone(snapshot) as AppSnapshot),
    host_protocol_version: 2,
    device: { ...snapshot.device, protocol_version: 3 },
  });
  backendMocks.previewImpl = async () => ({
    png_base64: "cHJldmlldw==",
    sample: false,
    state: null,
  });

  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<App />));
    await waitFor(() =>
      expect(container.textContent).toContain(
        "This display speaks protocol 3; this server speaks protocol 2.",
      ),
    );
    // And the door to the settings sheet says something is wrong with it.
    expect(container.querySelector(".topbar__settings.has-attention")).not.toBeNull();
  } finally {
    await cleanup();
    backendMocks.snapshotImpl = async () => snapshot;
  }
});

test("the global card banner does not blame the display for a host-side scene failure", async () => {
  backendMocks.snapshotImpl = async () => ({
    ...(structuredClone(snapshot) as AppSnapshot),
    card_errors: [
      {
        kind: "scene-refused",
        card_id: "clock",
        message: "the configured timezone is not recognized",
      },
    ],
  });
  backendMocks.previewImpl = async () => ({
    png_base64: "cHJldmlldw==",
    sample: false,
    state: null,
  });

  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<App />));
    await waitFor(() => expect(container.textContent).toContain("One card could not be rendered"));
    expect(container.textContent).toContain("the configured timezone is not recognized");
    expect(container.textContent).not.toContain("The display refused one card update");
  } finally {
    await cleanup();
    backendMocks.snapshotImpl = async () => snapshot;
  }
});

test("the global card banner names the display only for a typed data refusal", async () => {
  backendMocks.snapshotImpl = async () => ({
    ...(structuredClone(snapshot) as AppSnapshot),
    card_errors: structuredClone(apiContractFixtures.snapshot.card_errors),
  });
  backendMocks.previewImpl = async () => ({
    png_base64: "cHJldmlldw==",
    sample: false,
    state: null,
  });

  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<App />));
    await waitFor(() =>
      expect(container.textContent).toContain("The display refused one card update"),
    );
    expect(container.textContent).toContain(apiContractFixtures.snapshot.card_errors[0].message);
    expect(container.textContent).not.toContain("One card could not be rendered");
  } finally {
    await cleanup();
    backendMocks.snapshotImpl = async () => snapshot;
  }
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
  const previous = { ...backendMocks };
  backendMocks.snapshotImpl = async () => ({ ...snapshot, config, pomodoros: [], card_errors: [] });
  backendMocks.validateImpl = async () => ({ valid: false, issues });
  backendMocks.previewImpl = async () => ({ png_base64: null, sample: false, state: null });
  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<App />));
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
  } finally {
    try {
      await cleanup();
    } finally {
      Object.assign(backendMocks, previous);
    }
  }
});

for (const outcome of ["success", "rejection"] as const) {
  test(`a pending save ${outcome} preserves newer edits and permits saving them next`, async () => {
    let liveSnapshot: AppSnapshot = structuredClone(snapshot);
    const pending = Promise.withResolvers<void>();
    const refreshing = Promise.withResolvers<AppSnapshot>();
    let refreshStarted = false;
    const saved: AppConfig[] = [];
    backendMocks.snapshotImpl = async () => liveSnapshot;
    backendMocks.previewImpl = async () => ({ png_base64: null, sample: false, state: null });
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
    await waitFor(() => expect(buttonWithText(container, "Save to server")?.disabled).toBe(false));
    await act(async () => buttonWithText(container, "Save to server")?.click());
    expect(saved).toHaveLength(1);
    expect(saved[0].advance).toEqual({ kind: "manual" });
    await act(async () => buttonWithText(container, "Timed")?.click());
    if (outcome === "success") {
      await act(async () => pending.resolve());
      await waitFor(() => expect(refreshStarted).toBe(true));
      expect(buttonWithText(container, "Timed")?.getAttribute("aria-pressed")).toBe("true");
      await act(async () => refreshing.resolve(liveSnapshot));
    } else {
      await act(async () => pending.reject(new Error("old draft rejected")));
    }
    expect(container.textContent).not.toContain("old draft rejected");
    expect(buttonWithText(container, "Timed")?.getAttribute("aria-pressed")).toBe("true");
    expect(container.textContent).toContain("Unsaved changes");
    backendMocks.snapshotImpl = async () => liveSnapshot;
    await waitFor(() => expect(buttonWithText(container, "Save to server")?.disabled).toBe(false));
    await act(async () => buttonWithText(container, "Save to server")?.click());
    expect(saved).toHaveLength(2);
    expect(saved[1].advance.kind).toBe("timed");
    expect(container.textContent).toContain(
      "Saved to the server. The server will update your display.",
    );
  });
}

test("both SaveBars reject reentry before paint and stay disabled after a pending edit", async () => {
  const pending = Promise.withResolvers<void>();
  let saves = 0;
  backendMocks.saveConfigImpl = async () => {
    saves += 1;
    await pending.promise;
    return { save: { generation: 1, warning: null } };
  };
  const { container } = await mount(<App />);
  await act(async () => buttonWithText(container, "Manual")?.click());
  await act(async () => container.querySelector<HTMLButtonElement>(".topbar__settings")?.click());
  await waitFor(() => expect(buttonWithText(container, "Save to server")?.disabled).toBe(false));
  const savesButtons = [...container.querySelectorAll<HTMLButtonElement>("button")].filter(
    (button) => button.textContent === "Save to server",
  );
  expect(savesButtons).toHaveLength(2);
  await act(async () => {
    savesButtons[0].click();
    savesButtons[1].click();
  });
  expect(saves).toBe(1);
  await act(async () => buttonWithText(container, "Timed")?.click());
  expect(savesButtons.every((button) => button.disabled)).toBe(true);
  expect(savesButtons.every((button) => button.textContent === "Saving to server…")).toBe(true);
  await act(async () => pending.resolve());
});

async function changeInput(input: HTMLInputElement | null, value: string) {
  expect(input).not.toBeNull();
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, value);
    input?.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

for (const nextDevice of ["desk-B", "desk-A"]) {
  for (const pendingListen of [false, true]) {
    test(`subscription regenerates for ${nextDevice} with pending listen ${pendingListen}`, async () => {
      let deviceId = "desk-A";
      let changing = false;
      let holdOldRefresh = false;
      const oldFetch = Promise.withResolvers<AppSnapshot>();
      let staleDeliveredBeforeCleanup = false;
      const oldListen = Promise.withResolvers<() => void>();
      const registrations: {
        deviceId: string;
        publish: (next: AppSnapshot) => void;
        closes: number;
      }[] = [];
      const marked = (message: string): AppSnapshot => ({
        ...snapshot,
        runtime: { kind: "error", message },
      });
      backendMocks.networkSettingsImpl = async () => ({
        server_url: "https://desk.example",
        device_id: deviceId,
        tier: "networked",
      });
      backendMocks.signInAndSelectDeviceImpl = async (selected, token) => {
        expect(token).toBe("new-session");
        deviceId = selected;
        changing = true;
        return backendMocks.networkSettingsImpl();
      };
      backendMocks.listenImpl = async (publish) => {
        const registration = { deviceId, publish, closes: 0 };
        registrations.push(registration);
        if (registrations.length === 1 && pendingListen) return oldListen.promise;
        return () => {
          registration.closes += 1;
        };
      };
      backendMocks.snapshotImpl = () => {
        if (!changing)
          return holdOldRefresh ? oldFetch.promise : Promise.resolve(marked("original device"));
        oldFetch.reject(new Error("stale subscription error"));
        if (registrations[0].closes === 0) {
          staleDeliveredBeforeCleanup = true;
          registrations[0].publish(marked("stale before cleanup"));
        }
        return new Promise<AppSnapshot>(() => {});
      };
      const { container } = await mount(<App />);
      if (pendingListen) await act(async () => registrations[0].publish(marked("original device")));
      await waitFor(() => expect(container.textContent).toContain("original device"));
      holdOldRefresh = true;
      await act(async () => window.dispatchEvent(new Event("focus")));
      await act(async () =>
        container.querySelector<HTMLButtonElement>(".topbar__settings")?.click(),
      );
      await changeInput(container.querySelector('.network-form input[maxlength="32"]'), nextDevice);
      await changeInput(
        container.querySelector('.network-form input[type="password"]'),
        "new-session",
      );
      await act(async () => buttonWithText(container, "Sign in")?.click());
      expect(staleDeliveredBeforeCleanup).toBe(true);
      expect(container.textContent).not.toContain("stale before cleanup");
      expect(container.textContent).not.toContain("stale subscription error");
      expect(registrations.map((entry) => entry.deviceId)).toEqual(["desk-A", nextDevice]);
      if (pendingListen) {
        await act(async () =>
          oldListen.resolve(() => {
            registrations[0].closes += 1;
          }),
        );
      }
      expect(registrations[0].closes).toBe(1);
      await act(async () => registrations[1].publish(marked("current stream")));
      await act(async () => registrations[0].publish(marked("stale after cleanup")));
      expect(container.textContent).toContain("current stream");
      expect(container.textContent).not.toContain("stale after cleanup");
      await act(async () => window.dispatchEvent(new Event("focus")));
      expect(registrations).toHaveLength(2);
    });
  }
}
