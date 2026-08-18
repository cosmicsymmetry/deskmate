import { describe, expect, mock, test } from "bun:test";
import { act, type ComponentProps, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";

import { App } from "../src/App";
import { CardEditor } from "../src/components/CardEditor";
import { CardList } from "../src/components/CardList";
import { DeviceHeader } from "../src/components/DeviceHeader";
import { DevicePreview } from "../src/components/DevicePreview";
import { Filmstrip } from "../src/components/Filmstrip";
import { NetworkPanel } from "../src/components/NetworkPanel";
import { PlaylistPanel } from "../src/components/PlaylistPanel";
import { ProviderStatus, formatProviderAge } from "../src/components/ProviderStatus";
import {
  cardsContainerIssues,
  issuesForCard,
  issuesForPath,
  unclaimedIssues,
} from "../src/lib/configDraft";
import * as tauriModule from "../src/lib/tauri";
import type {
  AppConfig,
  AppSnapshot,
  CardSettings,
  ConfigApplyResult,
  DraftValidation,
  NetworkSettings,
  PreviewFrame,
  ValidationIssue,
} from "../src/lib/types";
import { ipcContractFixtures } from "../src/lib/types.contract";
import { resolveDeviceTier, saveConfigForTier } from "../src/lib/useAppState";

const snapshot = ipcContractFixtures.snapshot;
const cards = snapshot.config.cards;

// `DevicePreview` calls `renderCardPreview` (typed IPC over `tauri.ts`) directly, so
// its behaviour tests mock that one export at the module boundary — the same
// boundary every other command is mocked at when a test needs to control an IPC
// result — while every other export of `tauri.ts` (used transitively by `App` and
// `useAppState`) stays real.
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

mock.module("../src/lib/tauri", () => ({
  ...tauriModule,
  renderCardPreview: (cardId: string) => previewImpl(cardId),
  getAppSnapshot: () => snapshotImpl(),
  listenToAppState: async () => () => {},
  validateConfigDraft: (config: AppConfig) => validateImpl(config),
  saveApplyConfig: (config: AppConfig) => saveImpl(config),
  saveServerConfig: (config: AppConfig) => serverSaveImpl(config),
  getNetworkSettings: () => networkSettingsImpl(),
  getAutostartStatus: async () => ({ enabled: false, preference_enabled: false }),
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
  };
}

function weatherCard(id: string, alert: CardSettings["alert"] = { kind: "none" }): CardSettings {
  return {
    kind: "weather",
    id,
    title: "Weather",
    location: "",
    units: "metric",
    template: { kind: "big-number-label" },
    tap_action: { kind: "none" },
    refresh: { kind: "interval", minutes: 30 },
    alert,
  };
}

function cardListConfig(
  cardList: CardSettings[],
  entries = cardList.map((card) => ({ card_id: card.id, dwell_seconds: null })),
): AppConfig {
  return {
    schema_version: 4,
    preferences: { timezone: "UTC", autostart: false, paused: false, orientation: "landscape" },
    cards: cardList,
    assets: [],
    playlists: [
      {
        id: "workday",
        name: "Workday",
        advance: { kind: "timed", default_dwell_seconds: 20 },
        entries,
      },
    ],
    active_playlist_id: "workday",
    updater: { channel: "stable", checks: "notify" },
  };
}

describe("settings accessibility and states", () => {
  test("renders a non-blocking loading state before the first backend snapshot", () => {
    const html = renderToStaticMarkup(<App />);
    expect(html).toContain("Opening your display settings");
    expect(html).toContain("background service keeps running");
  });

  function renderCardEditor(card: CardSettings, issues: ValidationIssue[] = []) {
    return renderToStaticMarkup(
      <CardEditor
        card={card}
        issues={issues}
        pomodoro={null}
        timerBusy={false}
        filePickerBusy={false}
        onChange={() => {}}
        onRemove={() => {}}
        onTimerAction={() => {}}
        onChooseCalendarFile={() => {}}
      />,
    );
  }

  function renderNetworkPanel({
    tier,
    wifiState,
    ip,
    ssid = "",
  }: {
    tier: "local" | "networked";
    wifiState: "down" | "connected";
    ip: string;
    ssid?: string;
  }) {
    // Deliberately include hostile extra properties at the runtime boundary. The
    // public prop type does not admit them, and the component must continue to ignore
    // them if a stale/malicious caller supplies a wider object anyway.
    const publicSettingsWithStoredSecrets = {
      serverUrl: "https://desk.example",
      deviceId: "desk-1",
      ssid,
      passphrase: "stored-wifi-secret",
      deviceToken: "stored-device-secret",
      adminToken: "stored-admin-secret",
    };
    return renderToStaticMarkup(
      <NetworkPanel
        device={{
          tier,
          wifiState,
          wifiRssi: wifiState === "connected" ? -54 : null,
          ip,
          lastNetworkError: null,
          otaState: "idle",
        }}
        settings={publicSettingsWithStoredSecrets}
        onPair={async () => {}}
        onUnpair={async () => {}}
        onFactoryReset={async () => {}}
      />,
    );
  }

  test("network panel shows the device as locally owned before provisioning", () => {
    const html = renderNetworkPanel({ tier: "local", wifiState: "down", ip: "" });
    expect(html).toContain("Owned by this Mac");
    expect(html).not.toContain("Owned by the server");
  });

  test("network panel says where settings are written once networked", () => {
    // The destination changing invisibly is the failure mode worth pinning:
    // the same edit goes to a different place depending on tier.
    const html = renderNetworkPanel({
      tier: "networked",
      wifiState: "connected",
      ip: "192.168.1.42",
    });
    expect(html).toContain("Owned by the server");
    expect(html).toContain("192.168.1.42");
    expect(html).toContain("Settings are saved to the server");
  });

  test("network panel renders public settings but never a stored secret", () => {
    const html = renderNetworkPanel({
      tier: "networked",
      wifiState: "connected",
      ip: "192.168.1.42",
      ssid: "home-network",
    });
    expect(html).toContain("home-network");
    expect(html).toContain("https://desk.example");
    expect(html).toContain("desk-1");
    expect(html).not.toContain("stored-wifi-secret");
    expect(html).not.toContain("stored-device-secret");
    expect(html).not.toContain("stored-admin-secret");
  });

  test("network panel accepts secrets once and clears them after pairing", async () => {
    let submitted: Parameters<ComponentProps<typeof NetworkPanel>["onPair"]>[0] | null = null;
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    const setInput = (labelText: string, value: string) => {
      const label = [...container.querySelectorAll("label")].find((candidate) =>
        candidate.querySelector("span")?.textContent?.includes(labelText),
      );
      const input = label?.querySelector("input");
      expect(input).not.toBeNull();
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, value);
      input?.dispatchEvent(new Event("input", { bubbles: true }));
    };
    const inputValue = (labelText: string) =>
      [...container.querySelectorAll("label")]
        .find((candidate) => candidate.querySelector("span")?.textContent?.includes(labelText))
        ?.querySelector("input")?.value;

    try {
      await act(async () =>
        root.render(
          <NetworkPanel
            device={{
              tier: "local",
              wifiState: "down",
              wifiRssi: null,
              ip: "",
              lastNetworkError: null,
              otaState: "idle",
            }}
            settings={{
              serverUrl: "https://desk.example",
              deviceId: "desk-1",
              ssid: "home-network",
            }}
            onPair={async (input) => {
              submitted = input;
            }}
            onUnpair={async () => {}}
            onFactoryReset={async () => {}}
            onSaveServerAccess={async () => {}}
          />,
        ),
      );
      await act(async () => {
        setInput("WiFi passphrase", "wifi-secret-92");
        setInput("Device token", "device-secret-17");
        setInput("Admin token", "admin-secret-46");
      });
      const pair = buttonWithText(container, "Pair with server");
      expect(pair?.disabled).toBe(false);
      await act(async () => pair?.click());

      expect(submitted).toMatchObject({
        ssid: "home-network",
        passphrase: "wifi-secret-92",
        server_url: "https://desk.example",
        device_id: "desk-1",
        device_token: "device-secret-17",
        admin_token: "admin-secret-46",
        tier: "networked",
      });
      expect(inputValue("WiFi passphrase")).toBe("");
      expect(inputValue("Device token")).toBe("");
      expect(inputValue("Admin token")).toBe("");
      expect(inputValue("WiFi network")).toBe("home-network");
      expect(inputValue("Server URL")).toBe("https://desk.example");
      expect(inputValue("Device ID")).toBe("desk-1");
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

  test("names the clock card's title field rather than calling it a heading", () => {
    // The clock faces draw no title chip, so the field only names the card in
    // the library. Weather still renders its chip, so "Heading" stays right
    // there — the assertion is that the two differ, not just that one changed.
    const clockHtml = renderCardEditor(clockCard("clock-1", "Desk"));
    expect(clockHtml).toContain("<span>Name</span>");
    expect(clockHtml).not.toContain("<span>Heading</span>");

    const weatherHtml = renderCardEditor(weatherCard("weather-1"));
    expect(weatherHtml).toContain("<span>Heading</span>");
  });

  test("explains the clean canvas and never shows the wire id", () => {
    // A distinctive id with no overlap with any visible label (unlike the
    // fixture's plain "clock", which is also a substring of the visible
    // "Digital clock" kind name and would make this assertion meaningless).
    const clock = clockCard("internal-uuid-0001", "Desk");
    const html = renderCardEditor(clock);
    expect(html).toContain("Clean 448 × 368 canvas");
    expect(html).toContain("Show seconds");
    // IDs are wire identifiers, not something a person should see or edit.
    expect(html).not.toContain("Widget ID");
    expect(html).not.toContain("internal-uuid-0001");
  });

  test("offers a bounded native file chooser for local calendars", () => {
    const calendar = cards.find((card) => card.kind === "calendar");
    if (calendar?.kind !== "calendar") {
      throw new Error("contract fixture is missing its calendar widget");
    }
    const html = renderCardEditor({ ...calendar, source: { kind: "file", value: "" } });
    expect(html).toContain("Choose file…");
    expect(html).toContain("up to 1 MB");
  });

  test("weather cards offer no alert controls", () => {
    const html = renderCardEditor(weatherCard("weather-1"));
    // The other four kinds have no trigger that could ever fire, so a
    // disabled alert control there would be noise — nothing renders at all.
    expect(html).not.toContain("alert-fieldset");
    expect(html).not.toMatch(/<legend>Alert<\/legend>/);
  });

  test("pomodoro and calendar cards do offer alert controls", () => {
    const pomodoro = cards.find((card) => card.kind === "pomodoro");
    const calendar = cards.find((card) => card.kind === "calendar");
    if (!pomodoro || !calendar) {
      throw new Error("contract fixture is missing pomodoro or calendar widgets");
    }
    expect(renderCardEditor(pomodoro)).toContain("Take over the screen when the timer ends");
    expect(renderCardEditor(calendar)).toContain("Take over the screen before an event");
  });

  test("states the card's tap gesture and the shared alert-dismiss behaviour", () => {
    const clock = cards.find((card) => card.kind === "clock");
    if (!clock) {
      throw new Error("contract fixture is missing its clock widget");
    }
    const html = renderCardEditor(clock);
    expect(html).toContain("Tapping this card does nothing.");
    expect(html).toContain("a tap dismisses it");
  });

  test("the library renders every card with alert and unused badges, never wire ids", () => {
    const config = cardListConfig(
      [
        clockCard("internal-uuid-0001", "Desk"),
        clockCard("internal-uuid-0002", "Focus", {
          kind: "on-timer-finish",
          hold: { kind: "until-dismissed" },
        }),
        clockCard("internal-uuid-0003", "Spare"),
      ],
      [
        { card_id: "internal-uuid-0001", dwell_seconds: null },
        { card_id: "internal-uuid-0002", dwell_seconds: null },
      ],
    );
    const html = renderToStaticMarkup(
      <CardList
        config={config}
        issues={[]}
        selectedCardId="first-clock-id"
        onSelect={() => {}}
        onAdd={() => {}}
        onRemove={() => {}}
      />,
    );
    expect(html).toContain('aria-label="Card library"');
    expect(html).toContain("Desk");
    expect(html).toContain("Focus");
    expect(html).toContain("Spare");
    expect(html).toContain(">alerts<");
    expect(html).toContain(">unused<");
    // Internal identifiers never reach the user — only their titles do.
    expect(html).not.toContain("internal-uuid-0001");
    expect(html).not.toContain("internal-uuid-0002");
    expect(html).not.toContain("internal-uuid-0003");
  });

  test("adding is disabled at the eight-card contract limit", () => {
    const config = cardListConfig(
      Array.from({ length: 8 }, (_, index) => clockCard(`card-${index}`)),
    );
    const html = renderToStaticMarkup(
      <CardList
        config={config}
        issues={[]}
        selectedCardId={null}
        onSelect={() => {}}
        onAdd={() => {}}
        onRemove={() => {}}
      />,
    );
    const addButtons = html.match(/<button class="add-card"[^>]*>[\s\S]*?<\/button>/g) ?? [];
    expect(addButtons.length).toBe(6);
    for (const button of addButtons) {
      expect(button).toContain('disabled=""');
      expect(button).toContain(">Add ");
    }
  });

  test("playlist entry reorder updates the ordered draft", async () => {
    const config = cardListConfig([clockCard("first", "Desk"), clockCard("second", "Up next")]);
    let latest = config;
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    function Harness() {
      const [value, setValue] = useState(config);
      latest = value;
      return (
        <PlaylistPanel config={value} issues={[]} onChange={setValue} onSelectCard={() => {}} />
      );
    }

    await act(async () => root.render(<Harness />));
    const moveUp = container.querySelector<HTMLButtonElement>(
      'button[aria-label="Move Up next up"]',
    );
    expect(moveUp).not.toBeNull();
    await act(async () => moveUp?.click());
    expect(latest.playlists[0].entries.map((entry) => entry.card_id)).toEqual(["second", "first"]);
    await act(async () => root.unmount());
    container.remove();
  });

  test("cards already in a playlist are disabled in Add from library", () => {
    const config = cardListConfig([clockCard("first", "Desk"), clockCard("second", "Up next")]);
    const html = renderToStaticMarkup(
      <PlaylistPanel config={config} issues={[]} onChange={() => {}} onSelectCard={() => {}} />,
    );
    const options = html.match(/<option[^>]*>[^<]*already added<\/option>/g) ?? [];
    expect(options).toHaveLength(2);
    expect(options.every((option) => option.includes('disabled=""'))).toBe(true);
    expect(html).toContain("Every library card is already in this playlist");
  });

  test("card and playlist-entry validation paths render on their corresponding rows", () => {
    const config = cardListConfig([clockCard("first", "Desk"), clockCard("second", "Up next")]);
    const issues: ValidationIssue[] = [
      { path: "cards[1].title", code: "too-long", message: "Card title is too long." },
      {
        path: "playlists[0].entries[1].dwell_seconds",
        code: "out-of-range",
        message: "Entry dwell is out of range.",
      },
    ];
    const library = renderToStaticMarkup(
      <CardList
        config={config}
        issues={issues}
        selectedCardId={null}
        onSelect={() => {}}
        onAdd={() => {}}
        onRemove={() => {}}
      />,
    );
    const playlists = renderToStaticMarkup(
      <PlaylistPanel config={config} issues={issues} onChange={() => {}} onSelectCard={() => {}} />,
    );
    expect(library).toContain("Card title is too long.");
    expect(playlists).toContain("Entry dwell is out of range.");
    expect(playlists).toContain('aria-invalid="true"');
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
    previewImpl = async () => ({ png_base64: "cHJldmlldw==", sample: false });

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<App />));
      await waitFor(() => expect(container.textContent).toContain("Workday"));
      expect(container.textContent).not.toContain("A quick first setup");

      const manualTab = buttonWithText(container, "○Manual");
      expect(manualTab).not.toBeUndefined();
      await act(async () => manualTab?.click());
      const makeActive = buttonWithText(container, "Make active");
      expect(makeActive).not.toBeUndefined();
      await act(async () => makeActive?.click());

      await waitFor(() => expect(container.textContent).toContain("Unsaved changes"));
      expect(container.textContent).not.toContain("A quick first setup");
      await waitFor(() => expect(container.textContent).toContain("●Manual◀ active"));
      await waitFor(() => {
        const save = buttonWithText(container, "Save & apply");
        expect(save?.disabled).toBe(false);
      });
      await act(async () => buttonWithText(container, "Save & apply")?.click());
      await waitFor(() => expect(saved).toHaveLength(1));
      expect(saved[0].active_playlist_id).toBe("manual");
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
    previewImpl = async () => ({ png_base64: "cHJldmlldw==", sample: false });

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<App />));
      await waitFor(() => expect(container.textContent).toContain("A quick first setup"));
      expect(container.textContent).toContain("Save your settings");

      await act(async () => buttonWithText(container, "○Manual")?.click());
      await act(async () => buttonWithText(container, "Make active")?.click());
      await waitFor(() => expect(container.textContent).toContain("Unsaved changes"));
      expect(container.textContent).toContain("A quick first setup");
      await waitFor(() => {
        expect(buttonWithText(container, "Save & apply")?.disabled).toBe(false);
      });
      await act(async () => buttonWithText(container, "Save & apply")?.click());
      await waitFor(() => expect(container.textContent).not.toContain("A quick first setup"));

      await act(async () => buttonWithText(container, "○Workday")?.click());
      await act(async () => buttonWithText(container, "Make active")?.click());
      await waitFor(() => expect(container.textContent).toContain("Unsaved changes"));
      expect(container.textContent).not.toContain("A quick first setup");
    } finally {
      await act(async () => root.unmount());
      container.remove();
      snapshotImpl = async () => snapshot;
      saveImpl = async () => ({ save: { generation: 1, warning: null } });
    }
  });

  test("server validation rejections show every issue beside the save action", async () => {
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
      throw new tauriModule.DeskmateCommandError({
        category: "validation",
        message: "the server rejected this configuration with 2 validation issue(s)",
        issues: [
          { path: "cards[0].title", code: "empty", message: "Choose a card title." },
          {
            path: "active_playlist_id",
            code: "missing-reference",
            message: "Choose an active playlist.",
          },
        ],
      });
    };
    previewImpl = async () => ({ png_base64: "cHJldmlldw==", sample: false });

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<App />));
      await waitFor(() => expect(container.textContent).toContain("Workday"));
      await act(async () => buttonWithText(container, "○Manual")?.click());
      await act(async () => buttonWithText(container, "Make active")?.click());
      await waitFor(() =>
        expect(buttonWithText(container, "Save to server")?.disabled).toBe(false),
      );
      await act(async () => buttonWithText(container, "Save to server")?.click());

      await waitFor(() =>
        expect(container.textContent).toContain(
          "the server rejected this configuration with 2 validation issue(s)",
        ),
      );
      expect(container.textContent).toContain("Choose a card title.");
      expect(container.textContent).toContain("Choose an active playlist.");
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
    previewImpl = async () => ({ png_base64: "cHJldmlldw==", sample: false });

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

  test("renders disconnected, protocol mismatch, stale, and last-good copy", () => {
    const mismatch = {
      ...snapshot,
      device: { ...snapshot.device, protocol_version: 2 as number },
    };
    const header = renderToStaticMarkup(
      <DeviceHeader
        snapshot={mismatch}
        commandError={null}
        busyAction={null}
        onTogglePause={() => {}}
      />,
    );
    expect(header).toContain("Display not connected");
    expect(header).toContain("supports protocol 1");

    const providers = renderToStaticMarkup(
      <ProviderStatus
        providers={snapshot.providers}
        cards={cards}
        refreshingId={null}
        onRefresh={() => {}}
      />,
    );
    expect(providers).toContain("Showing the last successful data");
    expect(providers).toContain("Refresh now");
  });

  function calendarCard(id: string, title = "Up next"): CardSettings {
    return {
      kind: "calendar",
      id,
      title,
      source: { kind: "url", value: "https://example.com/cal.ics" },
      template: { kind: "row-list" },
      tap_action: { kind: "none" },
      refresh: { kind: "interval", minutes: 15 },
      alert: { kind: "none" },
    };
  }

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

  test("renders the device's own pixels as an img with a data URL once the IPC resolves", async () => {
    previewImpl = async (cardId) => {
      expect(cardId).toBe("upnext");
      return { png_base64: "Zmlyc3QtZnJhbWU=", sample: false };
    };
    const { container, root } = await mountPreview(calendarCard("upnext"));
    await waitFor(() => {
      const img = container.querySelector("img");
      expect(img).not.toBeNull();
      expect(img?.getAttribute("src")).toBe("data:image/png;base64,Zmlyc3QtZnJhbWU=");
    });
    expect(container.querySelector(".preview-sample-badge")).toBeNull();
    await act(async () => root.unmount());
  });

  test("shows the explicit unavailable state when the IPC rejects, never a stale or invented frame", async () => {
    previewImpl = async () => {
      throw new Error("simulator init failed");
    };
    const { container, root } = await mountPreview(calendarCard("upnext"));
    await waitFor(() => {
      expect(container.textContent).toContain("Preview unavailable");
    });
    expect(container.querySelector("img")).toBeNull();
    await act(async () => root.unmount());
  });

  test("badges the frame as sample when the card has never published data", async () => {
    previewImpl = async () => ({ png_base64: "dW5jb25maWd1cmVk", sample: true });
    const { container, root } = await mountPreview(calendarCard("upnext"));
    await waitFor(() => {
      expect(container.querySelector("img")).not.toBeNull();
      expect(container.textContent).toContain("No data yet");
    });
    await act(async () => root.unmount());
  });

  test("re-requests the preview when dataGeneration bumps", async () => {
    let calls = 0;
    previewImpl = async () => {
      calls += 1;
      return { png_base64: `frame-${calls}`, sample: false };
    };
    const card = calendarCard("upnext");
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
        return { png_base64: "first-frame", sample: false };
      }
      return new Promise((resolve) => {
        resolveSecond = resolve;
      });
    };
    const cardA = calendarCard("first");
    const cardB = calendarCard("second");
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

    resolveSecond({ png_base64: "second-frame", sample: false });
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
        resolveSecond({ png_base64: "winning-frame", sample: false });
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
    // preview IPC at all, so a static render is enough to check the empty state.
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

  function filmstripConfig(): AppConfig {
    return {
      schema_version: 4,
      preferences: { timezone: "UTC", autostart: false, paused: false, orientation: "landscape" },
      cards: [
        calendarCard("first", "Desk"),
        calendarCard("second", "Up next"),
        calendarCard("third", "Focus"),
      ],
      assets: [],
      playlists: [
        {
          id: "workday",
          name: "Workday",
          advance: { kind: "timed", default_dwell_seconds: 20 },
          entries: [
            { card_id: "first", dwell_seconds: 45 },
            { card_id: "second", dwell_seconds: null },
          ],
        },
      ],
      active_playlist_id: "workday",
      updater: { channel: "stable", checks: "notify" },
    };
  }

  function renderFilmstrip(config: AppConfig): string {
    return renderToStaticMarkup(
      <Filmstrip config={config} selectedCardId="first" onSelect={() => {}} onReorder={() => {}} />,
    );
  }

  test("the filmstrip shows the loop length and only in-rotation cards", () => {
    const html = renderFilmstrip(filmstripConfig());
    expect(html).toMatch(/1 min 5 s/);
    expect(html).toContain("Desk");
    expect(html).toContain("Up next");
    expect(html).not.toContain("Focus");
  });

  test("the filmstrip hides timings and the play control under manual advance", () => {
    const config = filmstripConfig();
    const html = renderFilmstrip({
      ...config,
      playlists: [{ ...config.playlists[0], advance: { kind: "manual" } }],
    });
    expect(html).not.toMatch(/\d+s</);
    expect(html).not.toContain(">Play<");
    expect(html).toContain("Desk");
  });

  test("filmstrip keyboard reorder maps visible segments back to playlist entry indexes", async () => {
    const initial = filmstripConfig();
    initial.playlists[0].entries = [
      { card_id: "first", dwell_seconds: 45 },
      { card_id: "missing-card", dwell_seconds: null },
      { card_id: "second", dwell_seconds: null },
    ];
    let latest = initial;

    function Harness() {
      const [config, setConfig] = useState(initial);
      return (
        <Filmstrip
          config={config}
          selectedCardId="first"
          onSelect={() => {}}
          onReorder={(next) => {
            latest = next;
            setConfig(next);
          }}
        />
      );
    }

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<Harness />));
      const firstSegment = container.querySelector<HTMLButtonElement>(".filmstrip-segment__button");
      expect(firstSegment).not.toBeNull();
      await act(async () => {
        firstSegment?.dispatchEvent(
          new KeyboardEvent("keydown", { key: "ArrowDown", altKey: true, bubbles: true }),
        );
      });
      expect(latest.playlists[0].entries.map((entry) => entry.card_id)).toEqual([
        "missing-card",
        "second",
        "first",
      ]);
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  test("formats provider staleness without exposing raw timestamps", () => {
    expect(formatProviderAge(null)).toBe("No successful refresh yet");
    expect(formatProviderAge(30)).toBe("Updated just now");
    expect(formatProviderAge(120)).toBe("Updated 2 min ago");
    expect(formatProviderAge(7200)).toBe("Updated 2 hr ago");
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

  test("every issue is claimed by exactly one surface — the cards container, a card, a playlist, a preference field, or the unclaimed fallback", () => {
    const config = cardListConfig([clockCard("first"), clockCard("second")]);
    const issues: ValidationIssue[] = [
      { path: "cards", code: "empty", message: "At least one card is required." },
      { path: "cards[0].title", code: "too-long", message: "Title is too long." },
      { path: "cards[1]", code: "invalid-composition", message: "This card is misconfigured." },
      { path: "preferences.timezone", code: "invalid-timezone", message: "Unknown timezone." },
      {
        path: "playlists[0].advance.default_dwell_seconds",
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
    // (App scopes CardList/CardEditor/PlaylistPanel/the timezone field this exact way —
    // see App.tsx), plus the fallback under test. This is the invariant that actually
    // guards against the class of bug this fix addresses: if a surface's claim and
    // `unclaimedIssues`'s notion of "claimed" ever drift apart, an issue either goes
    // missing from every surface (as `device.capabilities` originally did) or gets
    // double-rendered — either way this union stops matching `issues` one-to-one.
    const union = [
      ...cardsContainerIssues(issues),
      ...config.cards.flatMap((card) => issuesForCard(issues, config, card.id)),
      ...issuesForPath(issues, "playlists"),
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
});
