import { describe, expect, mock, test } from "bun:test";
import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";

import { App } from "../src/App";
import { CardEditor } from "../src/components/CardEditor";
import { CardList } from "../src/components/CardList";
import { DevicePreview } from "../src/components/DevicePreview";
import { LoopRing } from "../src/components/LoopRing";
import { NetworkPanel, ownershipLabel } from "../src/components/NetworkPanel";
import {
  addCard,
  cardsContainerIssues,
  issuesForCard,
  issuesForPath,
  unclaimedIssues,
} from "../src/lib/configDraft";
import * as backendModule from "../src/lib/backend";

// Keep a reference to the real picture-source wrapper before `mock.module`
// replaces the module namespace in place, then stub `fetch` beneath it. Recording
// the browser request verifies that the mint reaches `/v1/images` with the name
// and face kind in the body, not merely that a client function was called.
const httpCalls: { method: string; path: string; body: unknown }[] = [];
const jsonResponse = (value: unknown) =>
  new Response(JSON.stringify(value), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
  const path = typeof input === "string" ? input : input.toString();
  const method = init?.method ?? "GET";
  const body = typeof init?.body === "string" ? JSON.parse(init.body) : null;
  httpCalls.push({ method, path, body });
  if (method === "POST" && path === "/v1/images") {
    // Exactly what the route answers: `{id, token}`, nothing else. Returning the
    // UI's translated shape here would let the client and test share the same
    // wrong assumption about the server contract.
    return jsonResponse({ id: "picture-source", token: "plaintext-once" });
  }
  return jsonResponse([]);
}) as typeof fetch;

const realMintImageSource = backendModule.mintImageSource;

import type {
  AppConfig,
  AppSnapshot,
  CardError,
  CardSettings,
  ConfigApplyResult,
  DraftValidation,
  FaceDescriptor,
  ImageSourceDescriptor,
  MintedImageSource,
  NetworkSettings,
  PreviewFrame,
  ValidationIssue,
} from "../src/lib/types";
import { ipcContractFixtures } from "../src/lib/types.contract";
import { resolveDeviceTier, saveConfigForTier } from "../src/lib/useAppState";

const snapshot = ipcContractFixtures.snapshot;
const cards = snapshot.config.cards;

// `DevicePreview` calls `renderCardPreview` directly, so its behavior tests mock
// that one export at the backend seam while the exports used transitively by
// `App` and `useAppState` stay real.
let previewImpl: (cardId: string) => Promise<PreviewFrame> = () =>
  Promise.reject(new Error("renderCardPreview not configured for this test"));
let snapshotImpl: () => Promise<AppSnapshot> = async () => snapshot;
let validateImpl: (config: AppConfig) => Promise<DraftValidation> = async () => ({
  valid: true,
  issues: [],
});
let saveImpl: (config: AppConfig) => Promise<ConfigApplyResult> = async () => ({
  save: { generation: 1, warning: null },
});
let serverSaveImpl: (config: AppConfig) => Promise<ConfigApplyResult> = async () => ({
  save: { generation: 1, warning: null },
});
let networkSettingsImpl: () => Promise<NetworkSettings> = async () => ({
  server_url: "https://desk.example",
  device_id: "desk-1",
  tier: "local",
});
const setServerEndpointImpl: (
  serverUrl: string,
  deviceId: string,
  adminToken: string,
) => Promise<NetworkSettings> = async (serverUrl, deviceId) => ({
  server_url: serverUrl,
  device_id: deviceId,
  tier: "networked",
});
let imageSourcesImpl: () => Promise<ImageSourceDescriptor[]> = async () => [];
let updateImageSourceFaceImpl: (
  sourceId: string,
  fields: Record<string, string>,
) => Promise<FaceDescriptor> = async () => {
  throw new Error("updateImageSourceFace not configured for this test");
};

mock.module("../src/lib/backend", () => ({
  ...backendModule,
  renderCardPreview: (cardId: string) => previewImpl(cardId),
  getAppSnapshot: () => snapshotImpl(),
  listenToAppState: async () => () => {},
  validateConfigDraft: (config: AppConfig) => validateImpl(config),
  saveApplyConfig: (config: AppConfig) => saveImpl(config),
  saveServerConfig: (config: AppConfig) => serverSaveImpl(config),
  getNetworkSettings: () => networkSettingsImpl(),
  setServerEndpoint: (serverUrl: string, deviceId: string, adminToken: string) =>
    setServerEndpointImpl(serverUrl, deviceId, adminToken),
  listImageSources: () => imageSourcesImpl(),
  updateImageSourceFace: (sourceId: string, fields: Record<string, string>) =>
    updateImageSourceFaceImpl(sourceId, fields),
}));

/// Renders into a live DOM root (unlike this file's other `renderToStaticMarkup`
/// tests) because `DevicePreview` fetches its frame in a `useEffect` — SSR never
/// runs effects, so these are the only tests here that need one. The extra
/// microtask turn after `render` lets the mocked `renderCardPreview` promise (and
/// the `setState` it drives) settle inside this `act` call rather than after it.
async function renderPreviewInto(root: Root, element: Parameters<Root["render"]>[0]) {
  await act(async () => {
    root.render(element);
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

/// Polls `assertion` inside `act`, so the passive-effect state updates the polling
/// itself waits out (rather than the initial `renderPreviewInto` settle) are also
/// attributed to an `act` scope.
async function waitFor(assertion: () => void, timeoutMs = 500): Promise<void> {
  const start = Date.now();
  for (;;) {
    try {
      assertion();
      return;
    } catch (error) {
      if (Date.now() - start > timeoutMs) {
        throw error;
      }
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 5));
      });
    }
  }
}

function buttonWithText(container: ParentNode, text: string): HTMLButtonElement | undefined {
  return [...container.querySelectorAll("button")].find(
    (button) => button.textContent?.trim() === text,
  );
}

function clockCard(
  id: string,
  title = `Card ${id}`,
  alert: CardSettings["alert"] = { kind: "none" },
): CardSettings {
  return {
    kind: "clock",
    id,
    title,
    show_seconds: true,
    template: { kind: "digital-clock" },
    tap_action: { kind: "none" },
    refresh: { kind: "device-local" },
    alert,
    dwell_seconds: null,
  };
}

function pictureCard(id = "picture-card"): CardSettings {
  return {
    kind: "picture",
    id,
    title: "Limits",
    source_id: "limits-source",
    tap_action: { kind: "none" },
    refresh: { kind: "manual" },
    alert: { kind: "none" },
    dwell_seconds: null,
  };
}

/// `cards` IS the loop since schema v10, so there is no second argument: a card's
/// position in this list is its position in the loop.
/**
 * A pomodoro with a distinguishable label.
 *
 * Tests that need two tellable-apart cards use this rather than two clocks:
 * since the Name field went, every clock is called "Clock" and two of them are
 * genuinely indistinguishable on every surface. The timer label is the one name
 * an owner can still type -- and it is drawn on the panel.
 */
function pomodoroCard(id: string, label: string): CardSettings {
  return {
    kind: "pomodoro",
    id,
    label,
    duration_seconds: 1500,
    template: { kind: "progress-ring" },
    tap_action: { kind: "start-pause" },
    refresh: { kind: "device-local" },
    alert: { kind: "none" },
    dwell_seconds: null,
  };
}

function cardListConfig(cardList: CardSettings[]): AppConfig {
  return {
    schema_version: snapshot.config.schema_version,
    preferences: { timezone: "UTC", autostart: false, paused: false, orientation: "landscape" },
    cards: cardList,
    image_sources: cardList.flatMap((card) =>
      card.kind === "picture" ? [{ id: card.source_id, name: "Claude limits" }] : [],
    ),
    assets: [],
    advance: { kind: "timed", default_dwell_seconds: 20 },
    updater: { channel: "stable", checks: "notify" },
  };
}

describe("settings accessibility and states", () => {
  test("renders a non-blocking loading state before the first backend snapshot", () => {
    const html = renderToStaticMarkup(<App />);
    expect(html).toContain("Waking the display");
    // The reassurance is about the server now, not a Mac agent: closing this tab
    // does not stop the display being updated.
    expect(html).toContain("whether or not this window is open");
  });

  test("a missing session offers the way in, not just a retry", async () => {
    // Found by driving the real browser: the screen named the admin token and
    // then gave nowhere to type it, because the sign-in lived inside Settings --
    // which needs the snapshot that had just failed to load. A retry button
    // cannot fix a missing credential, so the screen has to carry the field.
    snapshotImpl = async () => {
      throw new backendModule.DeskmateCommandError({
        category: "runtime-unavailable",
        message: backendModule.SESSION_REQUIRED_MESSAGE,
      });
    };
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<App />));
      await waitFor(() => {
        expect(container.textContent).toContain("Sign in to Deskmate");
        expect(container.querySelector('input[type="password"]')).not.toBeNull();
        expect(buttonWithText(container, "Sign in")).toBeDefined();
      });
      expect(buttonWithText(container, "Try again")).toBeUndefined();
    } finally {
      await act(async () => root.unmount());
      container.remove();
      snapshotImpl = async () => snapshot;
    }
  });

  test("signing in fills the device id the first load could not know", async () => {
    // The first network-settings fetch happens before this browser has a
    // session, so it comes back with no device id. Refreshing only the snapshot
    // left the Settings sheet showing an empty Device ID for the rest of the
    // session -- visible only by actually signing in, which is how this was found.
    let signedIn = false;
    networkSettingsImpl = async () => ({
      server_url: "https://desk.example",
      device_id: signedIn ? "dev-0005" : "",
      tier: "networked",
    });
    snapshotImpl = async () => {
      if (!signedIn) {
        throw new backendModule.DeskmateCommandError({
          category: "runtime-unavailable",
          message: backendModule.SESSION_REQUIRED_MESSAGE,
        });
      }
      return snapshot;
    };

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<App />));
      await waitFor(() => expect(container.textContent).toContain("Sign in to Deskmate"));

      signedIn = true;
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
      await act(async () => settingsButton()?.click());
      await waitFor(() => {
        const deviceId = [...container.querySelectorAll<HTMLInputElement>("input")].find(
          (input) => input.value === "dev-0005",
        );
        expect(deviceId).toBeDefined();
      });
    } finally {
      await act(async () => root.unmount());
      container.remove();
      snapshotImpl = async () => snapshot;
      networkSettingsImpl = async () => ({
        server_url: "https://desk.example",
        device_id: "desk-1",
        tier: "local",
      });
    }
  });

  function renderCardEditor(
    card: CardSettings,
    issues: ValidationIssue[] = [],
    cardError: CardError | null = null,
    pictureAccess: MintedImageSource | null = null,
  ) {
    return renderToStaticMarkup(
      <CardEditor
        card={card}
        config={cardListConfig([card])}
        issues={issues}
        entryIssues={[]}
        cardError={cardError}
        pomodoro={null}
        timerBusy={false}
        pictureAccess={pictureAccess}
        onChange={() => {}}
        onConfigChange={() => {}}
        onRemove={() => {}}
        onTimerAction={() => {}}
      />,
    );
  }

  function renderNetworkPanel({
    tier,
    wifiState,
    ip,
  }: {
    tier: "local" | "networked";
    wifiState: "down" | "connected";
    ip: string;
  }) {
    // Deliberately include hostile extra properties at the runtime boundary. The
    // public prop type does not admit them, and the component must continue to ignore
    // them if a stale/malicious caller supplies a wider object anyway.
    const publicSettingsWithStoredSecrets = {
      serverUrl: "https://desk.example",
      deviceId: "desk-1",
      passphrase: "stored-wifi-secret",
      deviceToken: "stored-device-secret",
      adminToken: "stored-admin-secret",
    };
    return renderToStaticMarkup(
      <NetworkPanel
        device={{
          tier,
          link: "/dev/cu.usbmodem2101",
          wifiState,
          wifiRssi: wifiState === "connected" ? -54 : null,
          ip,
          lastNetworkError: null,
          otaState: "idle",
        }}
        settings={publicSettingsWithStoredSecrets}
        onSaveServerAccess={async () => {}}
      />,
    );
  }

  test("a cable-owned display is named as such, and still points saves at the server", () => {
    // Because the page cannot change ownership, it must state where an edit made
    // here will land. A display owned by a cable is the surprising case.
    const html = renderNetworkPanel({ tier: "local", wifiState: "down", ip: "" });
    expect(ownershipLabel("local")).toBe("Owned by a cable");
    expect(html).toContain("owned by a cable");
    expect(html).toContain("only once it is owned by the server");
  });

  test("network panel says where settings are written once networked", () => {
    // The destination changing invisibly is the failure mode worth pinning:
    // the same edit goes to a different place depending on tier.
    const html = renderNetworkPanel({
      tier: "networked",
      wifiState: "connected",
      ip: "192.168.1.42",
    });
    expect(ownershipLabel("networked")).toBe("Owned by the server");
    expect(html).toContain("192.168.1.42");
    expect(html).toContain("Settings are saved to the server");
  });

  test("network panel renders public settings but never a stored secret", () => {
    const html = renderNetworkPanel({
      tier: "networked",
      wifiState: "connected",
      ip: "192.168.1.42",
    });
    expect(html).toContain("https://desk.example");
    expect(html).toContain("desk-1");
    expect(html).not.toContain("stored-wifi-secret");
    expect(html).not.toContain("stored-device-secret");
    expect(html).not.toContain("stored-admin-secret");
  });

  test("signing in submits the admin token once and then clears it", async () => {
    const attempts: { serverUrl: string; deviceId: string; adminToken: string }[] = [];
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () =>
        root.render(
          <NetworkPanel
            device={{
              tier: "networked",
              link: "server link",
              wifiState: "connected",
              wifiRssi: -54,
              ip: "10.0.0.2",
              lastNetworkError: null,
              otaState: "idle",
            }}
            settings={{ serverUrl: "https://desk.example", deviceId: "desk-1" }}
            onSaveServerAccess={async (serverUrl, deviceId, adminToken) => {
              attempts.push({ serverUrl, deviceId, adminToken });
            }}
          />,
        ),
      );

      const token = container.querySelector<HTMLInputElement>('input[type="password"]');
      expect(token).not.toBeNull();
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(
          token,
          "admin-secret",
        );
        token?.dispatchEvent(new Event("input", { bubbles: true }));
      });
      await act(async () => buttonWithText(container, "Sign in")?.click());

      await waitFor(() => expect(attempts).toHaveLength(1));
      expect(attempts[0]?.adminToken).toBe("admin-secret");
      // Cleared after use: the token buys a session and is not kept in the DOM
      // where a later screenshot or a stray autofill could resurface it.
      await waitFor(() => expect(token?.value).toBe(""));
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  test("the tier guard selects exactly one configuration destination", async () => {
    let localWrites = 0;
    let serverWrites = 0;
    const destinations = {
      local: async () => {
        localWrites += 1;
        return { save: { generation: 1, warning: null } };
      },
      server: async () => {
        serverWrites += 1;
        return { save: { generation: 2, warning: null } };
      },
    };

    await saveConfigForTier("local", destinations);
    expect({ localWrites, serverWrites }).toEqual({ localWrites: 1, serverWrites: 0 });
    await saveConfigForTier("networked", destinations);
    expect({ localWrites, serverWrites }).toEqual({ localWrites: 1, serverWrites: 1 });
  });

  test("an unplugged networked display never falls back to the cable", async () => {
    let localWrites = 0;
    let serverWrites = 0;
    const tier = resolveDeviceTier(null, {
      server_url: "https://desk.example",
      device_id: "desk-1",
      tier: "networked",
    });

    await saveConfigForTier(tier, {
      local: async () => {
        localWrites += 1;
        return { save: { generation: 1, warning: null } };
      },
      server: async () => {
        serverWrites += 1;
        return { save: { generation: 2, warning: null } };
      },
    });

    expect({ localWrites, serverWrites }).toEqual({ localWrites: 0, serverWrites: 1 });
  });

  test("legacy server settings without a persisted tier still refuse the cable", async () => {
    const tier = resolveDeviceTier(null, {
      server_url: "https://desk.example",
      device_id: "desk-1",
      tier: null,
    });
    expect(tier).toBe("networked");
  });

  test("no card offers a Name field, and the pomodoro keeps its timer label", () => {
    // A clock is a clock and a picture is named by its source. "Timer label" is
    // different: it is editable and drawn on the panel face.
    const clockHtml = renderCardEditor(clockCard("clock-1", "Desk"));
    expect(clockHtml).not.toContain("<span>Name</span>");
    expect(renderCardEditor(pictureCard())).not.toContain("<span>Name</span>");
    const pomodoro: CardSettings = {
      kind: "pomodoro",
      id: "pomodoro-1",
      label: "Focus",
      duration_seconds: 1500,
      template: { kind: "progress-ring" },
      tap_action: { kind: "start-pause" },
      refresh: { kind: "device-local" },
      alert: { kind: "none" },
      dwell_seconds: null,
    };
    expect(renderCardEditor(pomodoro)).toContain("<span>Timer label</span>");
  });

  test("cardIdentity names every visible surface without a template label", () => {
    // Control labels still carry template and title together deliberately: a screen reader hearing
    // "Remove Digital clock — Desk" is better served than by "Remove Desk".
    const config = cardListConfig(
      [clockCard("internal-uuid-0001", "Desk")],
      [{ card_id: "internal-uuid-0001", dwell_seconds: 45 }],
    );
    const library = renderToStaticMarkup(
      <CardList
        config={config}
        issues={[]}
        pomodoros={[]}
        selectedCardId="internal-uuid-0001"
        onSelect={() => {}}
        onAdd={() => {}}
        onChange={() => {}}
        onRemove={() => {}}
      />,
    );
    const loop = renderToStaticMarkup(
      <LoopRing
        config={config}
        issues={[]}
        selectedCardId={null}
        onSelect={() => {}}
        onChange={() => {}}
      />,
    );
    const editor = renderCardEditor(clockCard("internal-uuid-0001", "Desk"));

    // One name, the same on all three surfaces. "Desk" is a frozen creation
    // default rather than an editable identity, so it is not displayed.
    expect(library).not.toContain('class="tile-label">Digital clock<');
    expect(library).toContain('<strong class="card-tile__value numeral">Clock</strong>');
    expect(library).not.toContain('class="card-tile__name">Desk<');
    expect(loop).toContain('class="loop__entry-name">Clock<');
    expect(editor).toContain('id="editor-heading">Clock<');
    // The accessible name keeps the template: a listener gets no tile to look at.
    expect(library).toContain('aria-label="Move Digital clock — Clock earlier"');
    expect(library).toContain('aria-label="Remove Digital clock — Clock');
  });

  test("an untitled card is not labelled with its template twice", () => {
    // The quiet line is the owner's words, so it is absent rather than a repeat of
    // the label sitting directly above it.
    const config = cardListConfig(
      [clockCard("internal-uuid-0002", "")],
      [{ card_id: "internal-uuid-0002", dwell_seconds: 20 }],
    );
    const library = renderToStaticMarkup(
      <CardList
        config={config}
        issues={[]}
        pomodoros={[]}
        selectedCardId={null}
        onSelect={() => {}}
        onAdd={() => {}}
        onChange={() => {}}
        onRemove={() => {}}
      />,
    );
    expect(library).toContain("Digital clock");
    expect(library).not.toContain("card-tile__name");
  });

  test("a picture tile leads with its source and names every entry control", () => {
    const picture = pictureCard();
    const html = renderToStaticMarkup(
      <CardList
        config={cardListConfig([picture])}
        issues={[]}
        pomodoros={[]}
        ownershipTier="local"
        selectedCardId={picture.id}
        onSelect={() => {}}
        onAdd={() => {}}
        onChange={() => {}}
        onRemove={() => {}}
      />,
    );

    expect(html).not.toContain('class="tile-label">Picture<');
    // The tile's one fact is the source; generic format and kind labels would not
    // distinguish one picture from another.
    expect(html).toContain('<strong class="card-tile__value numeral">Claude limits</strong>');
    expect(html).not.toContain('numeral">PNG<');
    // No second line: a picture's title is its source name, said once.
    expect(html).not.toContain('class="card-tile__name"');
    expect(html).toContain('aria-label="Move Picture — Claude limits earlier"');
    expect(html).toContain('aria-label="Remove Picture — Claude limits');
    expect(html).toContain('<span class="flag">needs the server</span>');
  });

  test("a picture card states its source instead of offering a menu of them", () => {
    // Rendering identity as a select would imply that changing it is a safe edit,
    // when a new source makes this a different card.
    const html = renderCardEditor(pictureCard());

    expect(html).toContain("Picture source");
    expect(html).toContain("Claude limits");
    expect(html).toContain("limits-source");
    expect(html).not.toContain("<select");
  });

  test("a picture card whose source no longer exists says so rather than silently renaming", () => {
    const orphan = { ...pictureCard(), source_id: "deleted-source" };
    const html = renderToStaticMarkup(
      <CardEditor
        card={orphan}
        config={cardListConfig([pictureCard()])}
        issues={[]}
        entryIssues={[]}
        cardError={null}
        pomodoro={null}
        timerBusy={false}
        pictureAccess={null}
        onChange={() => {}}
        onConfigChange={() => {}}
        onRemove={() => {}}
        onTimerAction={() => {}}
      />,
    );

    expect(html).toContain("deleted-source · Missing source");
  });

  test("a tile does not print the source name twice when the title repeats it", () => {
    const named = { ...pictureCard(), title: "Claude limits" };
    const html = renderToStaticMarkup(
      <CardList
        config={cardListConfig([named])}
        issues={[]}
        pomodoros={[]}
        ownershipTier="local"
        selectedCardId={named.id}
        onSelect={() => {}}
        onAdd={() => {}}
        onChange={() => {}}
        onRemove={() => {}}
      />,
    );

    expect(html).toContain('numeral">Claude limits</strong>');
    expect(html).not.toContain("card-tile__name");
  });

  test("a picture editor shows source access once, immediately after minting", () => {
    const picture = pictureCard();
    const access: MintedImageSource = {
      source_id: picture.source_id,
      token: "plaintext-once",
      push_url: "https://desk.example/v1/images/plaintext-once",
    };
    const firstRender = renderCardEditor(picture, [], null, access);

    expect(firstRender).toContain('id="editor-heading">Claude limits<');
    expect(firstRender).toContain("Claude limits");
    expect(firstRender).toContain(picture.source_id);
    expect(firstRender).toContain(access.push_url);
    expect(firstRender).toContain(access.token);
    expect(firstRender).toContain("Copy token");
    expect(firstRender).toContain("This plaintext token is shown once.");

    const laterRender = renderCardEditor(picture);
    expect(laterRender).not.toContain(access.token);
    expect(laterRender).not.toContain("Copy token");
    expect(laterRender).not.toContain("This plaintext token is shown once.");
  });

  test("a picture editor renders and saves server-described fields without face-specific logic", async () => {
    const picture = pictureCard();
    const descriptor: FaceDescriptor = {
      kind: "opaque-server-face",
      label: "Source settings",
      fields: [
        {
          key: "place",
          label: "Place",
          type: "text",
          value: "Dubai",
          placeholder: "Dubai",
        },
        {
          key: "units",
          label: "Units",
          type: "enum",
          value: "metric",
          options: [
            { value: "metric", label: "Metric" },
            { value: "imperial", label: "Imperial" },
          ],
        },
      ],
    };
    imageSourcesImpl = async () => [
      { id: picture.source_id, name: "Claude limits", face: descriptor },
    ];
    let savedFields: Record<string, string> | null = null;
    updateImageSourceFaceImpl = async (_sourceId, fields) => {
      savedFields = fields;
      return {
        ...descriptor,
        fields: descriptor.fields.map((field) => ({
          ...field,
          value: fields[field.key] ?? field.value,
        })),
      };
    };
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () =>
        root.render(
          <CardEditor
            card={picture}
            config={cardListConfig([picture])}
            issues={[]}
            cardError={null}
            pomodoro={null}
            timerBusy={false}
            onChange={() => {}}
            onConfigChange={() => {}}
            onRemove={() => {}}
            onTimerAction={() => {}}
          />,
        ),
      );
      await waitFor(() => expect(container.textContent).toContain("Source settings"));
      const place = container.querySelector<HTMLInputElement>('input[placeholder="Dubai"]');
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(
          place,
          "Berlin",
        );
        place?.dispatchEvent(new Event("input", { bubbles: true }));
      });
      const imperial = buttonWithText(container, "Imperial");
      await act(async () => imperial?.click());
      await act(async () => buttonWithText(container, "Save source settings")?.click());

      await waitFor(() => expect(savedFields).toEqual({ place: "Berlin", units: "imperial" }));
      expect(container.textContent).toContain("Saved on the server");
      expect(container.textContent).not.toContain(descriptor.kind);
    } finally {
      await act(async () => root.unmount());
      container.remove();
      imageSourcesImpl = async () => [];
      updateImageSourceFaceImpl = async () => {
        throw new Error("updateImageSourceFace not configured for this test");
      };
    }
  });

  test("an external picture producer adds no settings form", async () => {
    const picture = pictureCard();
    imageSourcesImpl = async () => [{ id: picture.source_id, name: "Claude limits", face: null }];
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () =>
        root.render(
          <CardEditor
            card={picture}
            config={cardListConfig([picture])}
            issues={[]}
            cardError={null}
            pomodoro={null}
            timerBusy={false}
            onChange={() => {}}
            onConfigChange={() => {}}
            onRemove={() => {}}
            onTimerAction={() => {}}
          />,
        ),
      );
      await waitFor(() => expect(container.textContent).not.toContain("Loading source settings"));
      expect(container.textContent).not.toContain("Save source settings");
    } finally {
      await act(async () => root.unmount());
      container.remove();
      imageSourcesImpl = async () => [];
    }
  });

  test("the add menu creates a picture through its own server action", async () => {
    let pictureAdds = 0;
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () =>
        root.render(
          <CardList
            config={cardListConfig([clockCard("clock", "Desk")])}
            issues={[]}
            pomodoros={[]}
            selectedCardId={null}
            onSelect={() => {}}
            onAdd={() => {}}
            onAddPicture={() => {
              pictureAdds += 1;
            }}
            onChange={() => {}}
            onRemove={() => {}}
          />,
        ),
      );
      await act(async () => container.querySelector<HTMLButtonElement>(".card-tile__add")?.click());
      expect(container.textContent).toContain("Pictures");
      const items = [...container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')];
      const picture = items.find((button) => button.textContent?.includes("New picture source"));
      expect(picture).toBeDefined();
      const beforePicture = items.at(-2);

      beforePicture?.focus();
      await act(async () =>
        beforePicture?.dispatchEvent(
          new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }),
        ),
      );
      expect(document.activeElement).toBe(picture);

      await act(async () => picture?.click());
      expect(pictureAdds).toBe(1);
      expect(container.querySelector('[role="menu"]')).toBeNull();
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  test("a server-drawn face is not filed under Built in", async () => {
    // Server-drawn faces produce picture cards, so filing them beside device-local
    // kinds would contradict the tile, editor heading, and `cardLabel` result.
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () =>
        root.render(
          <CardList
            config={cardListConfig([])}
            issues={[]}
            pomodoros={[]}
            ownershipTier="networked"
            selectedCardId={null}
            onSelect={() => {}}
            onAdd={() => {}}
            onChange={() => {}}
            onRemove={() => {}}
            creatableFaces={[{ kind: "weather", label: "Weather", fields: [] }]}
            onAddFace={() => {}}
          />,
        ),
      );
      await act(async () => container.querySelector<HTMLButtonElement>(".card-tile__add")?.click());

      const groups = [...container.querySelectorAll(".menu__group")].map((group) => ({
        legend: group.querySelector("legend")?.textContent?.trim() ?? "",
        items: [...group.querySelectorAll('[role="menuitem"]')].map(
          (item) => item.querySelector("strong")?.textContent?.trim() ?? "",
        ),
      }));

      const builtIn = groups.find((group) => group.legend === "Built in");
      const serverSide = groups.find((group) => group.legend === "Server-side");
      expect(builtIn).toBeDefined();
      expect(serverSide).toBeDefined();
      expect(builtIn?.items).toContain("Digital clock");
      expect(builtIn?.items).not.toContain("Weather");
      expect(serverSide?.items).toEqual(["Weather"]);
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  test("the add menu offers the faces the server says it can draw", async () => {
    // The app does not know what weather is: it renders whatever the server
    // listed, so a fourth face appears here with no app change at all.
    let chosen: { kind: string; label: string } | null = null;
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () =>
        root.render(
          <CardList
            config={cardListConfig([])}
            issues={[]}
            pomodoros={[]}
            ownershipTier="networked"
            selectedCardId={null}
            onSelect={() => {}}
            onAdd={() => {}}
            onChange={() => {}}
            onRemove={() => {}}
            creatableFaces={[
              { kind: "weather", label: "Weather", fields: [] },
              { kind: "sunrise", label: "Sunrise", fields: [] },
            ]}
            onAddFace={(kind, label) => {
              chosen = { kind, label };
            }}
          />,
        ),
      );
      await act(async () => container.querySelector<HTMLButtonElement>(".card-tile__add")?.click());
      const menu = container.querySelector('[role="menu"]');
      expect(menu?.textContent).toContain("Weather");
      // A face this app has never heard of still reaches the menu.
      expect(menu?.textContent).toContain("Sunrise");

      const weatherItem = [
        ...(menu?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? []),
      ].find((button) => button.textContent?.includes("Weather"));
      await act(async () => weatherItem?.click());
      expect(chosen).toEqual({ kind: "weather", label: "Weather" });
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  test("the add menu offers an unused picture source and omits one already in the loop", async () => {
    const used = { id: "used-source", name: "Already shown" };
    const unused = { id: "unused-source", name: "Bring this back" };
    const config: AppConfig = {
      ...cardListConfig([{ ...pictureCard(), source_id: used.id }]),
      image_sources: [used, unused],
    };
    let chosenSource: string | null = null;
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () =>
        root.render(
          <CardList
            config={config}
            issues={[]}
            pomodoros={[]}
            ownershipTier="networked"
            selectedCardId={null}
            onSelect={() => {}}
            onAdd={() => {}}
            onAddPicture={(source) => {
              chosenSource = source?.id ?? null;
            }}
            onChange={() => {}}
            onRemove={() => {}}
          />,
        ),
      );
      await act(async () => container.querySelector<HTMLButtonElement>(".card-tile__add")?.click());
      const menu = container.querySelector('[role="menu"]');
      expect(menu?.textContent).toContain(unused.name);
      expect(menu?.textContent).not.toContain(used.name);
      expect(menu?.textContent).toContain("New picture source");

      const unusedItem = [
        ...(menu?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? []),
      ].find((button) => button.textContent?.includes(unused.name));
      await act(async () => unusedItem?.click());
      expect(chosenSource).toBe(unused.id);
      expect(container.querySelector('[role="menu"]')).toBeNull();
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  test("the window mints and adds a picture while keeping the token ephemeral", async () => {
    const clock = clockCard("clock", "Desk");
    snapshotImpl = async () => ({
      ...snapshot,
      config: cardListConfig([clock]),
      device: { ...snapshot.device, tier: "networked" },
      pomodoros: [],
      card_data: [],
      card_errors: [],
    });
    networkSettingsImpl = async () => ({
      server_url: "https://desk.example",
      device_id: "desk-1",
      tier: "networked",
    });
    previewImpl = async () => ({ png_base64: null, sample: false, state: null });
    httpCalls.length = 0;

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
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
      await act(async () => root.unmount());
      container.remove();
      snapshotImpl = async () => snapshot;
      networkSettingsImpl = async () => ({
        server_url: "https://desk.example",
        device_id: "desk-1",
        tier: "local",
      });
      previewImpl = () =>
        Promise.reject(new Error("renderCardPreview not configured for this test"));
    }
  });

  test("never shows the wire id", () => {
    // A distinctive id with no overlap with any visible label (unlike the
    // fixture's plain "clock", which is also a substring of the visible
    // "Digital clock" kind name and would make this assertion meaningless).
    const clock = clockCard("internal-uuid-0001", "Desk");
    const html = renderCardEditor(clock);
    expect(html).toContain("Show seconds");
    // IDs are wire identifiers, not something a person should see or edit.
    expect(html).not.toContain("Widget ID");
    expect(html).not.toContain("internal-uuid-0001");
  });

  test("pomodoro cards offer their timer-finish alert controls", () => {
    const pomodoro = cards.find((card) => card.kind === "pomodoro");
    if (!pomodoro) {
      throw new Error("contract fixture is missing its pomodoro card");
    }
    expect(renderCardEditor(pomodoro)).toContain("Take over the screen when the timer ends");
  });

  test("states the card's tap gesture and the shared alert-dismiss behaviour", () => {
    const clock = cards.find((card) => card.kind === "clock");
    if (!clock) {
      throw new Error("contract fixture is missing its clock widget");
    }
    const html = renderCardEditor(clock);
    expect(html).toContain("Tapping this card does nothing.");
    expect(html).toContain("Swiping the screen moves through the loop.");
    expect(html).toContain("a tap dismisses it");
  });

  test("the selected card's timed-loop dwell field writes the card's own dwell", async () => {
    const card = clockCard("clock-1", "Desk");
    const initial = cardListConfig([card]);
    let latest = initial;

    function Harness() {
      const [config, setConfig] = useState(initial);
      latest = config;
      return (
        <CardEditor
          card={card}
          config={config}
          issues={[
            {
              path: "cards[0].dwell_seconds",
              code: "out-of-range",
              message: "Entry dwell is out of range.",
            },
          ]}
          cardError={null}
          pomodoro={null}
          timerBusy={false}
          onChange={() => {}}
          onConfigChange={setConfig}
          onRemove={() => {}}
          onTimerAction={() => {}}
        />
      );
    }

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<Harness />));
      const dwell = container.querySelector<HTMLInputElement>(
        'input[aria-label="Stays on the panel for Clock"]',
      );
      expect(dwell?.placeholder).toBe("20 s, the loop's default");
      expect(dwell?.getAttribute("aria-invalid")).toBe("true");
      expect(container.textContent).toContain("Entry dwell is out of range.");
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(
          dwell,
          "45",
        );
        dwell?.dispatchEvent(new Event("input", { bubbles: true }));
      });
      expect(latest.cards[0].dwell_seconds).toBe(45);
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  test("the dwell field is shown when the loop is timed or the value must be repaired", () => {
    const card = clockCard("clock-1", "Desk");
    const issue: ValidationIssue = {
      path: "cards[0].dwell_seconds",
      code: "out-of-range",
      message: "Entry dwell is out of range.",
    };
    const renderDwell = (config: AppConfig, issues: ValidationIssue[] = []) =>
      renderToStaticMarkup(
        <CardEditor
          card={config.cards[0]}
          config={config}
          issues={issues}
          cardError={null}
          pomodoro={null}
          timerBusy={false}
          onChange={() => {}}
          onConfigChange={() => {}}
          onRemove={() => {}}
          onTimerAction={() => {}}
        />,
      );

    // Manual advance with no value set: nothing to show.
    const manualNullConfig = cardListConfig([card]);
    manualNullConfig.advance = { kind: "manual" };
    expect(renderDwell(manualNullConfig)).not.toContain("Stays on the panel for");

    // Manual advance with an out-of-range value: shown so it can be repaired.
    const manualInvalidConfig = structuredClone(manualNullConfig);
    manualInvalidConfig.cards[0].dwell_seconds = 2;
    const manualInvalid = renderDwell(manualInvalidConfig, [issue]);
    expect(manualInvalid).toContain("Stays on the panel for");
    expect(manualInvalid).toContain('aria-invalid="true"');
    expect(manualInvalid).toContain("Dwell applies when the loop is timed.");

    const timed = renderDwell(cardListConfig([card]));
    expect(timed).toContain("Stays on the panel for");
  });

  test("the grid renders container issues and scopes a card's own issues to its tile", () => {
    const config = cardListConfig([
      pomodoroCard("first", "Desk"),
      pomodoroCard("second", "Up next"),
    ]);
    const containerIssue: ValidationIssue = {
      path: "cards",
      code: "out-of-range",
      message: "The loop entries need attention.",
    };
    const secondDwellIssue: ValidationIssue = {
      path: "cards[1].dwell_seconds",
      code: "out-of-range",
      message: "The second dwell is out of range.",
    };
    const container = document.createElement("div");
    container.innerHTML = renderToStaticMarkup(
      <CardList
        config={config}
        issues={[containerIssue, secondDwellIssue]}
        pomodoros={[]}
        selectedCardId={null}
        onSelect={() => {}}
        onAdd={() => {}}
        onChange={() => {}}
        onRemove={() => {}}
      />,
    );

    expect(container.textContent).toContain("The loop entries need attention.");
    const firstTile = [...container.querySelectorAll<HTMLLIElement>(".card-tile")].find((tile) =>
      tile.textContent?.includes("Desk"),
    );
    const secondTile = [...container.querySelectorAll<HTMLLIElement>(".card-tile")].find((tile) =>
      tile.textContent?.includes("Up next"),
    );
    expect(firstTile?.classList.contains("has-issue")).toBe(false);
    expect(firstTile?.textContent).not.toContain("The second dwell is out of range.");
    expect(secondTile?.classList.contains("has-issue")).toBe(true);
    expect(secondTile?.textContent).toContain("The second dwell is out of range.");
  });

  test("the add slot is disabled at capacity and names the limiting contract", () => {
    const config = cardListConfig(
      Array.from({ length: 8 }, (_, index) => clockCard(`card-${index}`)),
    );
    const html = renderToStaticMarkup(
      <CardList
        config={config}
        issues={[]}
        pomodoros={[]}
        selectedCardId={null}
        onSelect={() => {}}
        onAdd={() => {}}
        onChange={() => {}}
        onRemove={() => {}}
      />,
    );
    expect(html).toContain('class="card-tile__add"');
    expect(html).toContain('aria-describedby="add-card-capacity"');
    expect(html).toContain('disabled=""');
    expect(html).toContain("The limit is 8 cards.");
  });

  test("the add menu aligns to the slot end only when it would overflow the viewport", async () => {
    const config = cardListConfig([clockCard("clock", "Desk")]);
    const container = document.createElement("div");
    container.className = "face__work";
    // Measured against the viewport, not this column: the menu is `position:
    // fixed` precisely so the column's `overflow-y` cannot clip it.
    Object.defineProperty(document.documentElement, "clientWidth", {
      configurable: true,
      value: 1000,
    });
    document.body.appendChild(container);
    const root = createRoot(container);

    try {
      await act(async () =>
        root.render(
          <CardList
            config={config}
            issues={[]}
            pomodoros={[]}
            selectedCardId={null}
            onSelect={() => {}}
            onAdd={() => {}}
            onChange={() => {}}
            onRemove={() => {}}
          />,
        ),
      );
      const slotRoot = container.querySelector<HTMLLIElement>(".card-tile--add");
      const slot = container.querySelector<HTMLButtonElement>(".card-tile__add");
      if (!slotRoot) {
        throw new Error("add-card slot root was not rendered");
      }

      const slotAt = (left: number) =>
        ({ top: 300, bottom: 407, left, right: left + 148 }) as DOMRect;

      // 800 + 272 overflows 1000, so the menu hangs off its right edge instead.
      slotRoot.getBoundingClientRect = () => slotAt(800);
      await act(async () => slot?.click());
      expect(container.querySelector(".menu")?.classList.contains("menu--end")).toBe(true);

      await act(async () => slot?.click());
      slotRoot.getBoundingClientRect = () => slotAt(100);
      await act(async () => slot?.click());
      expect(container.querySelector(".menu")?.classList.contains("menu--end")).toBe(false);
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  /**
   * Opening the menu focuses its first item, and focusing something inside a
   * scrolling column makes the browser scroll it into view. A menu that closed
   * on scroll therefore closed itself the instant it opened — visible only as
   * "I picked a card and nothing happened", because by the time the pointer
   * arrived there was nothing under it. It repositions instead.
   */
  /**
   * WebKit does not move focus to a `<button>` on mousedown — a macOS
   * convention Chrome does not share. Opening the menu focuses its first item,
   * so pressing the mouse on any entry blurred that item with a **null**
   * `relatedTarget`. Reading that as "focus left the menu" unmounted the menu
   * between mousedown and click, so the click never landed and no card of any
   * kind could be added. Focus going nowhere is not focus leaving; a click
   * genuinely outside is caught by the document mousedown listener instead.
   *
   * This is invisible to the browser harness, which runs in Chrome.
   */
  test("a click inside the menu is not mistaken for focus leaving it", async () => {
    const config = cardListConfig([clockCard("clock", "Desk")]);
    const added: string[] = [];
    const container = document.createElement("div");
    container.className = "face__work";
    document.body.appendChild(container);
    const root = createRoot(container);

    try {
      await act(async () =>
        root.render(
          <CardList
            config={config}
            issues={[]}
            pomodoros={[]}
            selectedCardId={null}
            onSelect={() => {}}
            onAdd={(kind) => added.push(kind)}
            onChange={() => {}}
            onRemove={() => {}}
          />,
        ),
      );
      const slot = container.querySelector<HTMLButtonElement>(".card-tile__add");
      await act(async () => slot?.click());
      const firstItem = container.querySelector<HTMLButtonElement>('[role="menuitem"]');
      if (!firstItem) {
        throw new Error("the add menu rendered no items");
      }

      // What WebKit does when the mouse goes down on a menu item.
      await act(async () => {
        firstItem.dispatchEvent(new FocusEvent("focusout", { bubbles: true, relatedTarget: null }));
      });

      expect(container.querySelector(".menu")).not.toBeNull();

      await act(async () => firstItem.click());
      expect(added).toEqual(["clock"]);
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  test("the add menu survives the scroll that opening it causes", async () => {
    const config = cardListConfig([clockCard("clock", "Desk")]);
    const container = document.createElement("div");
    container.className = "face__work";
    document.body.appendChild(container);
    const root = createRoot(container);

    try {
      await act(async () =>
        root.render(
          <CardList
            config={config}
            issues={[]}
            pomodoros={[]}
            selectedCardId={null}
            onSelect={() => {}}
            onAdd={() => {}}
            onChange={() => {}}
            onRemove={() => {}}
          />,
        ),
      );
      const slot = container.querySelector<HTMLButtonElement>(".card-tile__add");
      await act(async () => slot?.click());
      expect(container.querySelector(".menu")).not.toBeNull();

      await act(async () => {
        container.dispatchEvent(new Event("scroll", { bubbles: false }));
        window.dispatchEvent(new Event("scroll"));
      });

      expect(container.querySelector(".menu")).not.toBeNull();
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  test("the add slot menu restores focus on Escape and a picked kind enrols and selects", async () => {
    const initial = cardListConfig([clockCard("clock", "Desk")]);
    let latest = initial;
    let selected: string | null = null;

    function Harness() {
      const [config, setConfig] = useState(initial);
      const [selectedCardId, setSelectedCardId] = useState<string | null>(null);
      latest = config;
      selected = selectedCardId;
      return (
        <CardList
          config={config}
          issues={[]}
          pomodoros={[]}
          selectedCardId={selectedCardId}
          onSelect={setSelectedCardId}
          onAdd={(kind) => {
            const result = addCard(config, kind);
            setConfig(result.config);
            setSelectedCardId(result.cardId);
          }}
          onChange={setConfig}
          onRemove={() => {}}
        />
      );
    }

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<Harness />));
      const slot = container.querySelector<HTMLButtonElement>(".card-tile__add");
      await act(async () => slot?.click());
      expect(slot?.getAttribute("aria-expanded")).toBe("true");
      expect(container.querySelector('[role="menu"]')).not.toBeNull();
      const menuText = container.querySelector('[role="menu"]')?.textContent;
      expect(menuText).toContain("Digital clock");
      expect(menuText).toContain("Pomodoro");
      expect(menuText).not.toContain("Weather");
      expect(menuText).not.toContain("Calendar");
      expect(menuText).not.toContain("RSS");

      const firstItem = container.querySelector<HTMLButtonElement>('[role="menuitem"]');
      await act(async () => {
        firstItem?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
        await new Promise((resolve) => setTimeout(resolve, 0));
      });
      expect(container.querySelector('[role="menu"]')).toBeNull();
      expect(document.activeElement === slot).toBe(true);

      await act(async () => slot?.click());
      slot?.focus();
      await act(async () => {
        slot?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
        await new Promise((resolve) => setTimeout(resolve, 0));
      });
      expect(container.querySelector('[role="menu"]')).toBeNull();
      expect(slot?.getAttribute("aria-expanded")).toBe("false");
      expect(document.activeElement === slot).toBe(true);

      await act(async () => slot?.click());
      const pomodoro = [...container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(
        (button) => button.textContent?.includes("Pomodoro"),
      );
      await act(async () => pomodoro?.click());
      expect(latest.cards.at(-1)?.kind).toBe("pomodoro");
      expect(latest.cards.at(-1)?.id).toBe("pomodoro");
      expect(selected).toBe("pomodoro");
      expect(container.querySelector('[role="menu"]')).toBeNull();
      expect(document.activeElement !== document.body).toBe(true);
      expect(document.activeElement?.classList.contains("card-tile__body")).toBe(true);
      // The tile is the countdown and the label now; the template is not printed.
      expect(document.activeElement?.textContent).toContain("Focus");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  test("tabbing focus outside the add slot closes its menu", async () => {
    const config = cardListConfig([clockCard("clock", "Desk")]);
    const container = document.createElement("div");
    const outside = document.createElement("button");
    outside.textContent = "Outside the menu";
    document.body.append(container, outside);
    const root = createRoot(container);

    try {
      await act(async () =>
        root.render(
          <CardList
            config={config}
            issues={[]}
            pomodoros={[]}
            selectedCardId={null}
            onSelect={() => {}}
            onAdd={() => {}}
            onChange={() => {}}
            onRemove={() => {}}
          />,
        ),
      );
      const slot = container.querySelector<HTMLButtonElement>(".card-tile__add");
      await act(async () => slot?.click());
      container.querySelector<HTMLButtonElement>('[role="menuitem"]')?.focus();
      await act(async () => outside.focus());

      expect(container.querySelector('[role="menu"]')).toBeNull();
      expect(slot?.getAttribute("aria-expanded")).toBe("false");
    } finally {
      await act(async () => root.unmount());
      container.remove();
      outside.remove();
    }
  });

  test("grid move buttons and Alt arrows update active-loop order", async () => {
    const config = cardListConfig([
      pomodoroCard("first", "Desk"),
      pomodoroCard("second", "Up next"),
    ]);
    let latest = config;
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    function Harness() {
      const [value, setValue] = useState(config);
      latest = value;
      return (
        <CardList
          config={value}
          issues={[]}
          pomodoros={[]}
          selectedCardId={null}
          onSelect={() => {}}
          onAdd={() => {}}
          onChange={setValue}
          onRemove={() => {}}
        />
      );
    }

    try {
      await act(async () => root.render(<Harness />));
      expect(
        container.querySelector<HTMLButtonElement>(
          'button[aria-label="Move Pomodoro — Desk earlier"]',
        )?.disabled,
      ).toBe(true);
      expect(
        container.querySelector<HTMLButtonElement>(
          'button[aria-label="Move Pomodoro — Up next later"]',
        )?.disabled,
      ).toBe(true);
      const earlier = container.querySelector<HTMLButtonElement>(
        'button[aria-label="Move Pomodoro — Up next earlier"]',
      );
      expect(earlier).not.toBeNull();
      await act(async () => earlier?.click());
      expect(latest.cards.map((card) => card.id)).toEqual(["second", "first"]);

      const later = container.querySelector<HTMLButtonElement>(
        'button[aria-label="Move Pomodoro — Up next later"]',
      );
      await act(async () => later?.click());
      expect(latest.cards.map((card) => card.id)).toEqual(["first", "second"]);

      let selectedTile = [
        ...container.querySelectorAll<HTMLButtonElement>(".card-tile .card-tile__body"),
      ].find((button) => button.textContent?.includes("Desk"));
      expect(selectedTile).not.toBeNull();
      await act(async () => {
        selectedTile?.dispatchEvent(
          new KeyboardEvent("keydown", { key: "ArrowRight", altKey: true, bubbles: true }),
        );
      });
      expect(latest.cards.map((card) => card.id)).toEqual(["second", "first"]);

      selectedTile = [
        ...container.querySelectorAll<HTMLButtonElement>(".card-tile .card-tile__body"),
      ].find((button) => button.textContent?.includes("Desk"));
      await act(async () => {
        selectedTile?.dispatchEvent(
          new KeyboardEvent("keydown", { key: "ArrowLeft", altKey: true, bubbles: true }),
        );
      });
      expect(latest.cards.map((card) => card.id)).toEqual(["first", "second"]);
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  test("keyboard reorder restores focus to the moved tile body", async () => {
    const config = cardListConfig([
      pomodoroCard("first", "Desk"),
      pomodoroCard("second", "Up next"),
    ]);
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    function Harness() {
      const [value, setValue] = useState(config);
      return (
        <CardList
          config={value}
          issues={[]}
          pomodoros={[]}
          selectedCardId={null}
          onSelect={() => {}}
          onAdd={() => {}}
          onChange={(next) => {
            (document.activeElement as HTMLElement | null)?.blur();
            setValue(next);
          }}
          onRemove={() => {}}
        />
      );
    }

    try {
      await act(async () => root.render(<Harness />));
      const firstTile = [...container.querySelectorAll<HTMLButtonElement>(".card-tile__body")].find(
        (button) => button.textContent?.includes("Desk"),
      );
      firstTile?.focus();
      await act(async () =>
        firstTile?.dispatchEvent(
          new KeyboardEvent("keydown", { key: "ArrowRight", altKey: true, bubbles: true }),
        ),
      );
      const movedTile = [...container.querySelectorAll<HTMLButtonElement>(".card-tile__body")].find(
        (button) => button.textContent?.includes("Desk"),
      );
      expect(movedTile?.textContent).toContain("Desk");
      expect(document.activeElement === movedTile).toBe(true);
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  test("dropping a loop tile on itself does not emit a draft change", async () => {
    const config = cardListConfig([
      pomodoroCard("first", "Desk"),
      pomodoroCard("second", "Up next"),
    ]);
    let changeCount = 0;
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    try {
      await act(async () =>
        root.render(
          <CardList
            config={config}
            issues={[]}
            pomodoros={[]}
            selectedCardId={null}
            onSelect={() => {}}
            onAdd={() => {}}
            onChange={() => {
              changeCount += 1;
            }}
            onRemove={() => {}}
          />,
        ),
      );
      const firstTile = [
        ...container.querySelectorAll<HTMLLIElement>('.card-tile[draggable="true"]'),
      ].find((tile) => tile.textContent?.includes("Desk"));
      const transfer = new DataTransfer();
      transfer.setData("text/plain", "first");
      const drop = new Event("drop", { bubbles: true }) as DragEvent;
      Object.defineProperty(drop, "dataTransfer", { value: transfer });
      await act(async () => firstTile?.dispatchEvent(drop));
      expect(changeCount).toBe(0);
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  test("editing an already-saved config does not resurface first-run guidance", async () => {
    let liveSnapshot = {
      ...(structuredClone(snapshot) as AppSnapshot),
      has_saved_config: true,
    };
    const saved: AppConfig[] = [];
    snapshotImpl = async () => liveSnapshot;
    validateImpl = async () => ({ valid: true, issues: [] });
    saveImpl = async (config) => {
      saved.push(config);
      liveSnapshot = { ...liveSnapshot, config };
      return { save: { generation: 2, warning: null } };
    };
    previewImpl = async () => ({ png_base64: "cHJldmlldw==", sample: false, state: null });

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
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
        const save = buttonWithText(container, "Save & apply");
        expect(save?.disabled).toBe(false);
      });
      await act(async () => buttonWithText(container, "Save & apply")?.click());
      await waitFor(() => expect(saved).toHaveLength(1));
      expect(saved[0].advance).toEqual({ kind: "manual" });
    } finally {
      await act(async () => root.unmount());
      container.remove();
      snapshotImpl = async () => snapshot;
      validateImpl = async () => ({ valid: true, issues: [] });
      saveImpl = async () => ({ save: { generation: 1, warning: null } });
    }
  });

  test("fresh default settings show first-run guidance until they have been saved", async () => {
    let liveSnapshot = {
      ...(structuredClone(snapshot) as AppSnapshot),
      has_saved_config: false,
    };
    snapshotImpl = async () => liveSnapshot;
    saveImpl = async (config) => {
      liveSnapshot = { ...liveSnapshot, config, has_saved_config: true };
      return { save: { generation: 1, warning: null } };
    };
    previewImpl = async () => ({ png_base64: "cHJldmlldw==", sample: false, state: null });

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<App />));
      await waitFor(() => expect(container.textContent).toContain("Make the display yours"));
      expect(container.textContent).toContain("Save your settings");

      await act(async () => buttonWithText(container, "Manual")?.click());
      await waitFor(() => expect(container.textContent).toContain("Unsaved changes"));
      expect(container.textContent).toContain("Make the display yours");
      await waitFor(() => {
        expect(buttonWithText(container, "Save & apply")?.disabled).toBe(false);
      });
      await act(async () => buttonWithText(container, "Save & apply")?.click());
      await waitFor(() => expect(container.textContent).not.toContain("Make the display yours"));

      await act(async () => buttonWithText(container, "Timed")?.click());
      await waitFor(() => expect(container.textContent).toContain("Unsaved changes"));
      expect(container.textContent).not.toContain("Make the display yours");
    } finally {
      await act(async () => root.unmount());
      container.remove();
      snapshotImpl = async () => snapshot;
      saveImpl = async () => ({ save: { generation: 1, warning: null } });
    }
  });

  test("the mounted app routes an unplugged persisted-networked save only to the server", async () => {
    let liveSnapshot: AppSnapshot = {
      ...(structuredClone(snapshot) as AppSnapshot),
      has_saved_config: true,
      device: {
        ...(structuredClone(snapshot.device) as AppSnapshot["device"]),
        tier: null,
      },
    };
    let localWrites = 0;
    let serverWrites = 0;
    snapshotImpl = async () => liveSnapshot;
    networkSettingsImpl = async () => ({
      server_url: "https://desk.example",
      device_id: "desk-1",
      tier: "networked",
    });
    saveImpl = async () => {
      localWrites += 1;
      return { save: { generation: 1, warning: null } };
    };
    serverSaveImpl = async (config) => {
      serverWrites += 1;
      liveSnapshot = { ...liveSnapshot, config };
      return { save: { generation: 2, warning: null } };
    };
    previewImpl = async () => ({ png_base64: "cHJldmlldw==", sample: false, state: null });

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<App />));
      await waitFor(() => expect(buttonWithText(container, "Save to server")).toBeDefined());
      await act(async () => buttonWithText(container, "Manual")?.click());
      await waitFor(() =>
        expect(buttonWithText(container, "Save to server")?.disabled).toBe(false),
      );
      await act(async () => buttonWithText(container, "Save to server")?.click());
      await waitFor(() => expect(serverWrites).toBe(1));
      expect(localWrites).toBe(0);
    } finally {
      await act(async () => root.unmount());
      container.remove();
      snapshotImpl = async () => snapshot;
      networkSettingsImpl = async () => ({
        server_url: "https://desk.example",
        device_id: "desk-1",
        tier: "local",
      });
      saveImpl = async () => ({ save: { generation: 1, warning: null } });
      serverSaveImpl = async () => ({ save: { generation: 1, warning: null } });
    }
  });

  test("the mounted app refuses saving beside the button until ownership loads", async () => {
    snapshotImpl = async () => ({
      ...(structuredClone(snapshot) as AppSnapshot),
      device: {
        ...(structuredClone(snapshot.device) as AppSnapshot["device"]),
        tier: null,
      },
    });
    networkSettingsImpl = () => new Promise<NetworkSettings>(() => {});
    previewImpl = async () => ({ png_base64: "cHJldmlldw==", sample: false, state: null });

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<App />));
      await waitFor(() => expect(buttonWithText(container, "Manual")).toBeDefined());
      const unavailable = buttonWithText(container, "Ownership unavailable");
      expect(unavailable?.disabled).toBe(true);
      expect(container.textContent).toContain(
        "Connect over USB to confirm ownership before saving.",
      );
      expect(buttonWithText(container, "Save & apply")).toBeUndefined();
    } finally {
      await act(async () => root.unmount());
      container.remove();
      snapshotImpl = async () => snapshot;
      networkSettingsImpl = async () => ({
        server_url: "https://desk.example",
        device_id: "desk-1",
        tier: "local",
      });
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
    snapshotImpl = async () => networkedSnapshot;
    networkSettingsImpl = async () => ({
      server_url: "https://desk.example",
      device_id: "desk-1",
      tier: "networked",
    });
    serverSaveImpl = async () => {
      throw new backendModule.DeskmateCommandError({
        category: "validation",
        message: "the server rejected this configuration with 7 validation issue(s)",
        issues: Array.from({ length: 7 }, (_, index) => ({
          path: `cards[${index}].title`,
          code: "empty" as const,
          message: `Server issue ${index + 1}.`,
        })),
      });
    };
    previewImpl = async () => ({ png_base64: "cHJldmlldw==", sample: false, state: null });

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<App />));
      await waitFor(() => expect(buttonWithText(container, "Manual")).toBeDefined());
      await act(async () => buttonWithText(container, "Manual")?.click());
      await waitFor(() =>
        expect(buttonWithText(container, "Save to server")?.disabled).toBe(false),
      );
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
      await act(async () => root.unmount());
      container.remove();
      snapshotImpl = async () => snapshot;
      networkSettingsImpl = async () => ({
        server_url: "https://desk.example",
        device_id: "desk-1",
        tier: "local",
      });
      serverSaveImpl = async () => ({ save: { generation: 1, warning: null } });
    }
  });

  test("validation-failed persistence shows the saved-settings banner and issue messages", async () => {
    const invalidSnapshot: AppSnapshot = {
      ...(structuredClone(snapshot) as AppSnapshot),
      persistence: {
        kind: "validation-failed",
        message: "Your saved settings failed validation and were not applied",
        issues: [
          {
            path: "playlists[0].entries[0].card_id",
            code: "missing-reference",
            message: "playlist entry references a card that does not exist",
          },
        ],
      },
    };
    snapshotImpl = async () => invalidSnapshot;
    previewImpl = async () => ({ png_base64: "cHJldmlldw==", sample: false, state: null });

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<App />));
      await waitFor(() =>
        expect(container.textContent).toContain(
          "Your saved settings failed validation and were not applied",
        ),
      );
      expect(container.textContent).toContain(
        "playlist entry references a card that does not exist",
      );
      expect(container.textContent).not.toContain("Using your last working settings");
    } finally {
      await act(async () => root.unmount());
      container.remove();
      snapshotImpl = async () => snapshot;
    }
  });

  test("a display speaking the server's own protocol raises nothing", async () => {
    // Compatibility compares the device with the server-reported version rather
    // than a client literal, so matching peers never produce a false warning.
    snapshotImpl = async () => ({
      ...(structuredClone(snapshot) as AppSnapshot),
      host_protocol_version: 2,
      device: { ...snapshot.device, protocol_version: 2 },
    });
    previewImpl = async () => ({ png_base64: "cHJldmlldw==", sample: false, state: null });

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<App />));
      // Wait for the window proper, then assert by absence. The save button's
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
      await act(async () => root.unmount());
      container.remove();
      snapshotImpl = async () => snapshot;
    }
  });

  test("a protocol the server cannot speak is stated in the work column and flags Settings", async () => {
    // A mismatch must remain visible without opening Settings. A device ahead of
    // the server is the realistic shape: firmware is flashed first.
    snapshotImpl = async () => ({
      ...(structuredClone(snapshot) as AppSnapshot),
      host_protocol_version: 2,
      device: { ...snapshot.device, protocol_version: 3 },
    });
    previewImpl = async () => ({ png_base64: "cHJldmlldw==", sample: false, state: null });

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
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
      await act(async () => root.unmount());
      container.remove();
      snapshotImpl = async () => snapshot;
    }
  });

  test("the global card banner does not blame the display for a host-side scene failure", async () => {
    snapshotImpl = async () => ({
      ...(structuredClone(snapshot) as AppSnapshot),
      card_errors: [
        {
          kind: "scene-refused",
          card_id: "clock",
          message: "the configured timezone is not recognized",
        },
      ],
    });
    previewImpl = async () => ({ png_base64: "cHJldmlldw==", sample: false, state: null });

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<App />));
      await waitFor(() =>
        expect(container.textContent).toContain("One card could not be rendered"),
      );
      expect(container.textContent).toContain("the configured timezone is not recognized");
      expect(container.textContent).not.toContain("The display refused one card update");
    } finally {
      await act(async () => root.unmount());
      container.remove();
      snapshotImpl = async () => snapshot;
    }
  });

  test("the global card banner names the display only for a typed data refusal", async () => {
    snapshotImpl = async () => ({
      ...(structuredClone(snapshot) as AppSnapshot),
      card_errors: [
        {
          kind: "data-refused",
          card_id: "clock",
          message: "the display refused this card's data (InvalidPayload)",
        },
      ],
    });
    previewImpl = async () => ({ png_base64: "cHJldmlldw==", sample: false, state: null });

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<App />));
      await waitFor(() =>
        expect(container.textContent).toContain("The display refused one card update"),
      );
      expect(container.textContent).not.toContain("One card could not be rendered");
    } finally {
      await act(async () => root.unmount());
      container.remove();
      snapshotImpl = async () => snapshot;
    }
  });

  test("the settings sheet shows the device link and unavailable ownership", () => {
    const html = renderNetworkPanel({
      tier: "networked",
      wifiState: "connected",
      ip: "192.168.1.42",
    });
    expect(html).toContain("/dev/cu.usbmodem2101");
    expect(ownershipLabel(null)).toBe("Ownership unavailable");
  });

  /// A live-mounted `DevicePreview` for one selected card, wired to the shared
  /// `previewImpl` mock above. `dataGeneration` defaults to 0 since most of these
  /// tests aren't exercising re-fetch-on-change.
  async function mountPreview(
    card: CardSettings,
    dataGeneration = 0,
  ): Promise<{ container: HTMLDivElement; root: Root }> {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    await renderPreviewInto(
      root,
      <DevicePreview
        cards={[card]}
        selectedWidgetId={card.id}
        orientation="landscape"
        dataGeneration={dataGeneration}
      />,
    );
    return { container, root };
  }

  test("renders the device's own pixels as an img when the preview request resolves", async () => {
    previewImpl = async (cardId) => {
      expect(cardId).toBe("upnext");
      return { png_base64: "Zmlyc3QtZnJhbWU=", sample: false, state: null };
    };
    const { container, root } = await mountPreview(clockCard("upnext"));
    await waitFor(() => {
      const img = container.querySelector("img");
      expect(img).not.toBeNull();
      expect(img?.getAttribute("src")).toBe("data:image/png;base64,Zmlyc3QtZnJhbWU=");
    });
    expect(container.querySelector(".stage__badge")).toBeNull();
    await act(async () => root.unmount());
  });

  test("shows the explicit unavailable state when the preview request rejects", async () => {
    previewImpl = async () => {
      throw new Error("simulator init failed");
    };
    const { container, root } = await mountPreview(clockCard("upnext"));
    await waitFor(() => {
      expect(container.textContent).toContain("Preview unavailable");
    });
    expect(container.querySelector("img")).toBeNull();
    await act(async () => root.unmount());
  });

  test("badges the frame as sample when the card has never published data", async () => {
    previewImpl = async () => ({ png_base64: "dW5jb25maWd1cmVk", sample: true, state: null });
    const { container, root } = await mountPreview(clockCard("upnext"));
    await waitFor(() => {
      expect(container.querySelector("img")).not.toBeNull();
      expect(container.textContent).toContain("No data yet");
    });
    await act(async () => root.unmount());
  });

  test("a frameless preview prints its state word, never the fault message", async () => {
    previewImpl = async () => ({
      png_base64: null,
      sample: false,
      state: "Waiting for the first refresh",
    });
    const { container, root } = await mountPreview(pictureCard());
    await waitFor(() => {
      expect(container.textContent).toContain("Waiting for the first refresh");
    });
    expect(container.textContent).not.toContain("Preview unavailable");
    expect(container.querySelector("img")).toBeNull();
    // The badge means "a real frame from sample data". There is no frame here.
    expect(container.querySelector(".stage__badge")).toBeNull();
    await act(async () => root.unmount());
  });

  test("re-requests the preview when dataGeneration bumps", async () => {
    let calls = 0;
    previewImpl = async () => {
      calls += 1;
      return { png_base64: `frame-${calls}`, sample: false, state: null };
    };
    const card = clockCard("upnext");
    const { container, root } = await mountPreview(card, 0);
    await waitFor(() => expect(calls).toBe(1));

    await renderPreviewInto(
      root,
      <DevicePreview
        cards={[card]}
        selectedWidgetId={card.id}
        orientation="landscape"
        dataGeneration={1}
      />,
    );
    await waitFor(() => expect(calls).toBe(2));
    await waitFor(() => {
      expect(container.querySelector("img")?.getAttribute("src")).toBe(
        "data:image/png;base64,frame-2",
      );
    });
    await act(async () => root.unmount());
  });

  test("keeps a stale frame on screen while a superseding request is in flight, then replaces it", async () => {
    // Two cards, so switching `selectedWidgetId` (not `dataGeneration`) is what
    // triggers the re-request here — the effect depends on both.
    let resolveSecond!: (frame: PreviewFrame) => void;
    let requestCount = 0;
    previewImpl = async (cardId) => {
      requestCount += 1;
      if (cardId === "first") {
        return { png_base64: "first-frame", sample: false, state: null };
      }
      return new Promise((resolve) => {
        resolveSecond = resolve;
      });
    };
    const cardA = clockCard("first");
    const cardB = clockCard("second");
    const { container, root } = await mountPreview(cardA);
    await waitFor(() => {
      expect(container.querySelector("img")?.getAttribute("src")).toBe(
        "data:image/png;base64,first-frame",
      );
    });

    await renderPreviewInto(
      root,
      <DevicePreview
        cards={[cardA, cardB]}
        selectedWidgetId="second"
        orientation="landscape"
        dataGeneration={0}
      />,
    );
    await waitFor(() => expect(requestCount).toBe(2));
    // The old frame is still the last thing painted — no blank flash, no stale claim
    // beyond what was already shown, since the component never clears `frame` itself.
    expect(container.querySelector("img")?.getAttribute("src")).toBe(
      "data:image/png;base64,first-frame",
    );

    resolveSecond({ png_base64: "second-frame", sample: false, state: null });
    await waitFor(() => {
      expect(container.querySelector("img")?.getAttribute("src")).toBe(
        "data:image/png;base64,second-frame",
      );
    });
    await act(async () => root.unmount());
  });

  // Regression test: `preview.rs`'s latest-wins coalescing replies to a superseded
  // job with `Err("superseded")` *before* the winning job even starts rendering, so
  // an older request's rejection can land in the browser after a newer request from
  // the *same* effect invocation has already succeeded. This only happens within one
  // effect invocation (a prop-driven re-render already fully guards the old promise
  // via `cancelled`, set synchronously during React's cleanup) — the live-templates
  // 1 Hz interval is the one in-component path that issues a second request before
  // the first has settled, so this test drives that interval deterministically
  // instead of waiting on a real 1 s timer: it captures the handler `DevicePreview`
  // passes to `window.setInterval` and invokes it itself.
  test("an older superseded rejection landing after a newer success does not flip the panel to unavailable", async () => {
    let capturedTick: (() => void) | undefined;
    const originalSetInterval = window.setInterval;
    const originalClearInterval = window.clearInterval;
    window.setInterval = ((handler: () => void) => {
      capturedTick = handler;
      return 0;
    }) as unknown as typeof window.setInterval;
    window.clearInterval = (() => {}) as unknown as typeof window.clearInterval;

    try {
      let rejectFirst!: (error: unknown) => void;
      let resolveSecond!: (frame: PreviewFrame) => void;
      let callCount = 0;
      previewImpl = () => {
        callCount += 1;
        if (callCount === 1) {
          return new Promise((_resolve, reject) => {
            rejectFirst = reject;
          });
        }
        return new Promise((resolve) => {
          resolveSecond = resolve;
        });
      };

      // `clock` is a live template, so `DevicePreview` registers the 1 Hz interval
      // this test drives by hand.
      const clock = clockCard("clock-preview");
      const { container, root } = await mountPreview(clock);
      await waitFor(() => expect(callCount).toBe(1));
      expect(capturedTick).not.toBeUndefined();

      // Fire the "interval tick" ourselves: the second, superseding request.
      await act(async () => {
        capturedTick?.();
        await new Promise((resolve) => setTimeout(resolve, 0));
      });
      await waitFor(() => expect(callCount).toBe(2));

      // The newer request succeeds first...
      await act(async () => {
        resolveSecond({ png_base64: "winning-frame", sample: false, state: null });
        await new Promise((resolve) => setTimeout(resolve, 0));
      });
      await waitFor(() => {
        expect(container.querySelector("img")?.getAttribute("src")).toBe(
          "data:image/png;base64,winning-frame",
        );
      });

      // ...then the older, now-superseded request's rejection lands. It must not
      // flip the panel to "Preview unavailable" over the already-correct frame.
      await act(async () => {
        rejectFirst(new Error("superseded"));
        await new Promise((resolve) => setTimeout(resolve, 20));
      });
      expect(container.textContent).not.toContain("Preview unavailable");
      expect(container.querySelector("img")?.getAttribute("src")).toBe(
        "data:image/png;base64,winning-frame",
      );

      await act(async () => root.unmount());
    } finally {
      window.setInterval = originalSetInterval;
      window.clearInterval = originalClearInterval;
    }
  });

  test("shows 'No cards configured' when there are no cards, never a stale or invented frame", () => {
    // No live effect needed here: with no cards, `DevicePreview` never calls the
    // preview backend at all, so a static render is enough to check the empty state.
    const html = renderToStaticMarkup(
      <DevicePreview
        cards={[]}
        selectedWidgetId={null}
        orientation="landscape-flipped"
        dataGeneration={0}
      />,
    );
    expect(html).toContain("No cards configured");
    expect(html).not.toContain("<img");
  });

  function loopConfig(): AppConfig {
    return {
      schema_version: snapshot.config.schema_version,
      preferences: { timezone: "UTC", autostart: false, paused: false, orientation: "landscape" },
      // Three cards the loop can actually tell apart. Three clocks would all
      // read "Clock" now, which is true of the product and useless in a test
      // about which entry is which.
      cards: [
        { ...pomodoroCard("first", "Desk"), dwell_seconds: 45 },
        pomodoroCard("second", "Up next"),
        pomodoroCard("third", "Focus"),
      ],
      image_sources: [],
      assets: [],
      advance: { kind: "timed", default_dwell_seconds: 20 },
      updater: { channel: "stable", checks: "notify" },
    };
  }

  function renderLoopRing(config: AppConfig): string {
    return renderToStaticMarkup(
      <LoopRing
        config={config}
        issues={[]}
        selectedCardId="first"
        onSelect={() => {}}
        onChange={() => {}}
      />,
    );
  }

  test("the loop ring shows the loop length and every card in it", () => {
    // 45 + 20 + 20: the first card overrides the 20 s default, the other two take it.
    // Since schema v10 every card is in the loop, so there is no card to leave out.
    const html = renderLoopRing(loopConfig());
    expect(html).toMatch(/1 min 25 s/);
    expect(html).toContain("Desk");
    expect(html).toContain("Up next");
    expect(html).toContain("Focus");
  });

  test("the loop ring hides timings and the play control under manual advance", () => {
    const config = loopConfig();
    const html = renderLoopRing({
      ...config,
      advance: { kind: "manual" },
    });
    expect(html).not.toMatch(/\d+s</);
    expect(html).not.toContain("Play the loop");
    expect(html).toContain("Desk");
  });

  test("the loop ring owns pacing and writes the active loop advance mode", async () => {
    const initial = loopConfig();
    let latest = initial;

    function Harness() {
      const [config, setConfig] = useState(initial);
      latest = config;
      return (
        <LoopRing
          config={config}
          issues={[
            {
              path: "advance.default_dwell_seconds",
              code: "out-of-range",
              message: "Default dwell is out of range.",
            },
          ]}
          selectedCardId="first"
          onSelect={() => {}}
          onChange={setConfig}
        />
      );
    }

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<Harness />));
      expect(container.textContent).toContain("The loop");
      expect(container.textContent).not.toContain("Workday");
      const dwell = container.querySelector<HTMLInputElement>(".loop__dwell input");
      expect(dwell?.getAttribute("aria-invalid")).toBe("true");
      expect(dwell?.getAttribute("aria-describedby")).toBe("loop-pacing-issues");
      expect(container.querySelector("#loop-pacing-issues")?.tagName).toBe("UL");
      expect(container.textContent).toContain("Default dwell is out of range.");
      const manual = buttonWithText(container, "Manual");
      expect(manual?.getAttribute("aria-pressed")).toBe("false");
      await act(async () => manual?.click());
      expect(latest.advance).toEqual({ kind: "manual" });
      expect(buttonWithText(container, "Manual")?.getAttribute("aria-pressed")).toBe("true");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  test("clicking the already-selected pacing mode does not emit a draft change", async () => {
    const config = loopConfig();
    let changeCount = 0;
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    try {
      await act(async () =>
        root.render(
          <LoopRing
            config={config}
            issues={[]}
            selectedCardId="first"
            onSelect={() => {}}
            onChange={() => {
              changeCount += 1;
            }}
          />,
        ),
      );
      await act(async () => buttonWithText(container, "Timed")?.click());
      expect(changeCount).toBe(0);
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  test("clearing the default dwell input keeps an empty edit without writing zero", async () => {
    const initial = loopConfig();
    let latest = initial;

    function Harness() {
      const [config, setConfig] = useState(initial);
      latest = config;
      return (
        <LoopRing
          config={config}
          issues={[]}
          selectedCardId="first"
          onSelect={() => {}}
          onChange={setConfig}
        />
      );
    }

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<Harness />));
      const dwell = container.querySelector<HTMLInputElement>(".loop__dwell input");
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(dwell, "");
        dwell?.dispatchEvent(new Event("input", { bubbles: true }));
      });
      expect(dwell?.value).toBe("");
      expect(latest.advance).toEqual({
        kind: "timed",
        default_dwell_seconds: 20,
      });
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  test("the loop legend only displays and selects; it has no reorder affordance", () => {
    const html = renderLoopRing(loopConfig());
    expect(html).not.toContain("draggable");
    expect(html).not.toContain("loop__moves");
    expect(html).not.toContain("Move Digital clock");
  });

  test("an issue on a path no card, playlist, or preference surface claims (e.g. a missing-capability issue) is not silently dropped", () => {
    const config = cardListConfig([clockCard("only-card")]);
    const capabilityIssue: ValidationIssue = {
      path: "device.capabilities",
      code: "requires-capability",
      message:
        "the connected firmware does not support extended templates. Update the firmware, or remove the cards and settings that need it.",
    };
    expect(unclaimedIssues([capabilityIssue], config)).toEqual([capabilityIssue]);
  });

  test("every issue is claimed by exactly one surface — the cards container, a card, the loop advance, a preference field, or the unclaimed fallback", () => {
    const config = cardListConfig([clockCard("first"), clockCard("second")]);
    const issues: ValidationIssue[] = [
      { path: "cards", code: "empty", message: "At least one card is required." },
      { path: "cards[0].title", code: "too-long", message: "Title is too long." },
      { path: "cards[1]", code: "invalid-composition", message: "This card is misconfigured." },
      { path: "preferences.timezone", code: "invalid-timezone", message: "Unknown timezone." },
      {
        path: "advance.default_dwell_seconds",
        code: "out-of-range",
        message: "Dwell time is out of range.",
      },
      {
        path: "device.capabilities",
        code: "requires-capability",
        message: "the connected firmware does not support extended templates.",
      },
    ];

    // Reconstructed independently from the same public helpers each real surface calls
    // (App scopes CardList/CardEditor/LoopRing/the timezone field this exact way —
    // see App.tsx), plus the fallback under test. This is the invariant that actually
    // guards against the class of bug this fix addresses: if a surface's claim and
    // `unclaimedIssues`'s notion of "claimed" ever drift apart, an issue either goes
    // missing from every surface (as `device.capabilities` originally did) or gets
    // double-rendered — either way this union stops matching `issues` one-to-one.
    const union = [
      ...cardsContainerIssues(issues),
      ...config.cards.flatMap((card) => issuesForCard(issues, config, card.id)),
      ...issuesForPath(issues, "playlists[0].entries"),
      ...issuesForPath(issues, "advance"),
      ...issuesForPath(issues, "preferences.timezone"),
      ...unclaimedIssues(issues, config),
    ];

    expect(union).toHaveLength(issues.length);
    expect(new Set(union)).toEqual(new Set(issues));
  });

  test("includes narrow-window and reduced-motion fallbacks", async () => {
    const css = await Bun.file(new URL("../src/styles.css", import.meta.url)).text();
    expect(css).toContain("@media (max-width: 430px)");
    expect(css).toContain("@media (prefers-reduced-motion: reduce)");
    expect(css).toContain("animation-duration: 0.001ms");
  });

  test("minting a picture source makes a server round trip with the source name", async () => {
    httpCalls.length = 0;

    // The translation is the point: the route says `id`, the window needs
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
});
