import { describe, expect, mock, test } from "bun:test";
import { act, type ComponentProps, useState } from "react";
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
import { formatProviderAge, providerTrouble } from "../src/lib/providers";
import * as tauriModule from "../src/lib/tauri";
// The two server reads are the only wrappers this suite proves end to end, so it
// keeps a reference to the REAL ones before `mock.module` below replaces the
// module's namespace in place, and mocks the Tauri bridge underneath them instead.
// A second test file cannot do this: `mock.module` is process-global with no
// guaranteed file order, so whichever file ran first would win.
const coreInvocations: { command: string; args?: Record<string, unknown> }[] = [];
mock.module("@tauri-apps/api/core", () => ({
  invoke: async (command: string, args?: Record<string, unknown>) => {
    coreInvocations.push({ command, args });
    return command === "get_server_plugins" ? { plugins: [], load_failures: [] } : [];
  },
}));
const realGetServerPlugins = tauriModule.getServerPlugins;
const realGetServerCardState = tauriModule.getServerCardState;
import type {
  AppConfig,
  AppSnapshot,
  CardError,
  CardSettings,
  ConfigApplyResult,
  DraftValidation,
  NetworkSettings,
  PluginCatalog,
  PreviewFrame,
  ProvisionDeviceInput,
  ServerCardState,
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
let serverPluginsImpl: () => Promise<PluginCatalog> = async () => ({
  plugins: [],
  load_failures: [],
});
let serverCardStateImpl: () => Promise<ServerCardState[]> = async () => [];
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
let provisionImpl: (input: ProvisionDeviceInput) => Promise<NetworkSettings> = async (input) => ({
  server_url: input.server_url,
  device_id: input.device_id,
  tier: input.tier,
});
let setServerEndpointImpl: (
  serverUrl: string,
  deviceId: string,
  adminToken: string,
) => Promise<NetworkSettings> = async (serverUrl, deviceId) => ({
  server_url: serverUrl,
  device_id: deviceId,
  tier: "networked",
});
let useLocalOwnershipImpl: () => Promise<NetworkSettings> = async () => ({
  server_url: "https://desk.example",
  device_id: "desk-1",
  tier: "local",
});

mock.module("../src/lib/tauri", () => ({
  ...tauriModule,
  renderCardPreview: (cardId: string) => previewImpl(cardId),
  getServerPlugins: () => serverPluginsImpl(),
  getServerCardState: () => serverCardStateImpl(),
  getAppSnapshot: () => snapshotImpl(),
  listenToAppState: async () => () => {},
  validateConfigDraft: (config: AppConfig) => validateImpl(config),
  saveApplyConfig: (config: AppConfig) => saveImpl(config),
  saveServerConfig: (config: AppConfig) => serverSaveImpl(config),
  getNetworkSettings: () => networkSettingsImpl(),
  provisionDevice: (input: ProvisionDeviceInput) => provisionImpl(input),
  setServerEndpoint: (serverUrl: string, deviceId: string, adminToken: string) =>
    setServerEndpointImpl(serverUrl, deviceId, adminToken),
  chooseLocalOwnership: () => useLocalOwnershipImpl(),
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

function pomodoroCard(id: string, label = `Card ${id}`): CardSettings {
  return {
    kind: "pomodoro",
    id,
    label,
    duration_seconds: 1500,
    template: { kind: "progress-ring" },
    tap_action: { kind: "start-pause" },
    refresh: { kind: "device-local" },
    alert: { kind: "on-timer-finish", hold: { kind: "until-dismissed" } },
  };
}

function pluginCard(id = "internal-plugin-card"): CardSettings {
  return {
    kind: "plugin",
    id,
    title: "Office air",
    plugin_id: "com.example.air-quality",
    tap_action: { kind: "none" },
    refresh: { kind: "interval", minutes: 15 },
    alert: { kind: "none" },
  };
}

function cardListConfig(
  cardList: CardSettings[],
  entries = cardList.map((card) => ({ card_id: card.id, dwell_seconds: null })),
): AppConfig {
  return {
    schema_version: snapshot.config.schema_version,
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
    expect(html).toContain("Waking the display");
    expect(html).toContain("background service keeps running");
  });

  function renderCardEditor(
    card: CardSettings,
    issues: ValidationIssue[] = [],
    cardError: CardError | null = null,
    catalog: PluginCatalog | null = null,
    ownershipTier: "local" | "networked" = "networked",
  ) {
    return renderToStaticMarkup(
      <CardEditor
        card={card}
        config={cardListConfig([card])}
        issues={issues}
        entryIssues={[]}
        cardError={cardError}
        pomodoro={null}
        provider={null}
        timerBusy={false}
        filePickerBusy={false}
        providerRefreshing={false}
        catalog={catalog}
        ownershipTier={ownershipTier}
        onChange={() => {}}
        onConfigChange={() => {}}
        onRemove={() => {}}
        onTimerAction={() => {}}
        onChooseCalendarFile={() => {}}
        onRefreshProvider={() => {}}
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
        onPair={async () => {}}
        onUnpair={async () => {}}
        onFactoryReset={async () => {}}
      />,
    );
  }

  test("network panel shows the device as locally owned before provisioning", () => {
    const html = renderNetworkPanel({ tier: "local", wifiState: "down", ip: "" });
    expect(ownershipLabel("local")).toBe("Owned by this Mac");
    expect(html).toContain("saved on this Mac and sent to the display over USB");
    expect(html).not.toContain("saved to the server");
    expect(html).toContain("Server base URL");
    expect(html).toContain("secure WebSocket device link");
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
        setInput("WiFi network", "home-network");
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
      expect(inputValue("Server base URL")).toBe("https://desk.example");
      expect(inputValue("Device ID")).toBe("desk-1");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  test("the mounted pairing flow provisions before persisting server access", async () => {
    const order: string[] = [];
    let serverAccess: [string, string, string] | null = null;
    snapshotImpl = async () => snapshot;
    networkSettingsImpl = async () => ({
      server_url: "https://desk.example",
      device_id: "desk-1",
      tier: "local",
    });
    provisionImpl = async (input) => {
      order.push("provision");
      return {
        server_url: input.server_url,
        device_id: input.device_id,
        tier: input.tier,
      };
    };
    setServerEndpointImpl = async (serverUrl, deviceId, adminToken) => {
      order.push("server-access");
      serverAccess = [serverUrl, deviceId, adminToken];
      return { server_url: serverUrl, device_id: deviceId, tier: "networked" };
    };
    previewImpl = async () => ({ png_base64: "cHJldmlldw==", sample: false, state: null });

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

    try {
      await act(async () => root.render(<App />));
      await waitFor(() => expect(container.textContent).toContain("Pair with server"));
      await act(async () => {
        setInput("WiFi network", "home-network");
        setInput("WiFi passphrase", "wifi-secret");
        setInput("Device token", "device-secret");
        setInput("Admin token", "admin-secret");
      });
      const pair = buttonWithText(container, "Pair with server");
      expect(pair?.disabled).toBe(false);
      await act(async () => pair?.click());
      await waitFor(() => expect(order).toHaveLength(2));

      expect(order).toEqual(["provision", "server-access"]);
      expect(serverAccess).toEqual(["https://desk.example", "desk-1", "admin-secret"]);
    } finally {
      await act(async () => root.unmount());
      container.remove();
      provisionImpl = async (input) => ({
        server_url: input.server_url,
        device_id: input.device_id,
        tier: input.tier,
      });
      setServerEndpointImpl = async (serverUrl, deviceId) => ({
        server_url: serverUrl,
        device_id: deviceId,
        tier: "networked",
      });
    }
  });

  test("an unplugged incomplete pairing can explicitly return this Mac to local routing", async () => {
    snapshotImpl = async () => ({
      ...(structuredClone(snapshot) as AppSnapshot),
      device: {
        ...(structuredClone(snapshot.device) as AppSnapshot["device"]),
        tier: null,
      },
    });
    networkSettingsImpl = async () => ({
      server_url: "https://desk.example",
      device_id: "desk-1",
      tier: null,
    });
    let overrideCalls = 0;
    useLocalOwnershipImpl = async () => {
      overrideCalls += 1;
      return {
        server_url: "https://desk.example",
        device_id: "desk-1",
        tier: "local",
      };
    };
    previewImpl = async () => ({ png_base64: "cHJldmlldw==", sample: false, state: null });

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<App />));
      await waitFor(() => expect(buttonWithText(container, "Use local on this Mac")).toBeDefined());
      expect(container.textContent).toContain("changes only the Mac's routing");
      await act(async () => buttonWithText(container, "Use local on this Mac")?.click());
      await waitFor(() => expect(overrideCalls).toBe(1));
      await waitFor(() => expect(buttonWithText(container, "Save & apply")).toBeDefined());
      expect(buttonWithText(container, "Use local on this Mac")).toBeUndefined();
    } finally {
      await act(async () => root.unmount());
      container.remove();
      snapshotImpl = async () => snapshot;
      networkSettingsImpl = async () => ({
        server_url: "https://desk.example",
        device_id: "desk-1",
        tier: "local",
      });
      useLocalOwnershipImpl = async () => ({
        server_url: "https://desk.example",
        device_id: "desk-1",
        tier: "local",
      });
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

  test("an unreachable server costs the window one notice and no plugin state", async () => {
    snapshotImpl = async () => ({
      ...snapshot,
      device: { ...snapshot.device, tier: "networked" },
      config: {
        ...snapshot.config,
        cards: [pluginCard("air")],
        playlists: [
          {
            id: "workday",
            name: "Workday",
            advance: { kind: "timed" as const, default_dwell_seconds: 20 },
            entries: [{ card_id: "air", dwell_seconds: null }],
          },
        ],
        active_playlist_id: "workday",
      },
      providers: [],
      pomodoros: [],
      card_data: [],
      card_errors: [],
    });
    networkSettingsImpl = async () => ({
      server_url: "https://desk.example",
      device_id: "desk-1",
      tier: "networked",
    });
    previewImpl = async () => ({ png_base64: "cHJldmlldw==", sample: false, state: null });
    let catalogCalls = 0;
    serverPluginsImpl = async () => {
      catalogCalls += 1;
      if (catalogCalls === 1) {
        throw { category: "runtime-unavailable", message: "connection refused" };
      }
      return {
        plugins: [
          {
            id: "com.example.air-quality",
            name: "aqi",
            version: "1.0.0",
            node_count: 7,
            assets: [],
            display_name: "Air quality",
            description: "EPA index for a location",
            manifest_version: 2,
            template: "display-list" as const,
            refresh_minutes: 15,
          },
        ],
        load_failures: [],
      };
    };
    serverCardStateImpl = async () => [];

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await renderPreviewInto(root, <App />);
      await waitFor(() => {
        expect(container.textContent).toContain("Couldn't reach the server for plugin data.");
      });
      // A successful card-state poll must not clear a failed catalog read: the two
      // failures are tracked apart even though they share one sentence.
      expect(container.textContent).toContain("com.example.air-quality");

      const retry = buttonWithText(container, "Try again");
      expect(retry).toBeDefined();
      await act(async () => retry?.click());
      await waitFor(() => {
        expect(container.textContent).not.toContain("Couldn't reach the server for plugin data.");
        expect(container.textContent).toContain("Air quality");
      });
      expect(catalogCalls).toBe(2);
    } finally {
      await act(async () => root.unmount());
      container.remove();
      serverPluginsImpl = async () => ({ plugins: [], load_failures: [] });
      serverCardStateImpl = async () => [];
      // Also restore what this test overrode beyond the two server impls: leaving
      // any of these mutated would make a later mounted-`App` test observe a
      // networked tier or this test's plugin-only config, which is exactly the
      // cross-test pollution this file's convention exists to avoid.
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

  test("a card is called the same thing everywhere, and that is its template", () => {
    // Owner's rule: a card should say what it *is*. "Outside" and "Desk" teach a
    // first-time reader nothing, so every surface that identifies a card leads with
    // its template and carries the owner's own title as a quiet second line.
    const config = cardListConfig(
      [clockCard("internal-uuid-0001", "Desk")],
      [{ card_id: "internal-uuid-0001", dwell_seconds: 45 }],
    );
    const library = renderToStaticMarkup(
      <CardList
        config={config}
        issues={[]}
        cardData={[]}
        pomodoros={[]}
        providers={[]}
        pluginKinds={[]}
        catalog={null}
        serverCardState={[]}
        ownershipTier="networked"
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

    expect(library).toContain('class="tile-label">Digital clock<');
    expect(library).toContain('class="card-tile__name">Desk<');
    expect(library).toContain('aria-label="Move Digital clock — Desk earlier"');
    expect(library).toContain('aria-label="Move Digital clock — Desk later"');
    expect(library).toContain('aria-label="Remove Digital clock — Desk');
    expect(loop).toContain('class="loop__entry-name">Digital clock<');
    expect(loop).toContain('class="loop__entry-title">Desk<');
    expect(editor).toContain('id="editor-heading">Digital clock<');
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
        cardData={[]}
        pomodoros={[]}
        providers={[]}
        pluginKinds={[]}
        catalog={null}
        serverCardState={[]}
        ownershipTier="networked"
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

  test("a plugin tile is named, valued and flagged like any other complication", () => {
    const plugin = pluginCard();
    const catalog: PluginCatalog = {
      plugins: [
        {
          id: "com.example.air-quality",
          name: "aqi",
          version: "1.0.0",
          node_count: 7,
          assets: [],
          display_name: "Air quality",
          description: "EPA index for a location",
          manifest_version: 2,
          template: "display-list",
          refresh_minutes: 15,
        },
      ],
      load_failures: [],
    };
    const render = (
      cardCatalog: PluginCatalog | null,
      serverCardState: ServerCardState[],
      tier: "local" | "networked",
    ) =>
      renderToStaticMarkup(
        <CardList
          config={cardListConfig([plugin])}
          issues={[]}
          cardData={[]}
          pomodoros={[]}
          providers={[]}
          pluginKinds={[]}
          catalog={cardCatalog}
          serverCardState={serverCardState}
          ownershipTier={tier}
          selectedCardId={plugin.id}
          onSelect={() => {}}
          onAdd={() => {}}
          onChange={() => {}}
          onRemove={() => {}}
        />,
      );

    const live = render(
      catalog,
      [{ card_id: plugin.id, provider: { kind: "fresh" }, hero: "42", errors: [] }],
      "networked",
    );
    expect(live).toContain('class="tile-label">Air quality<');
    expect(live).toContain('<strong class="card-tile__value numeral">42</strong>');
    expect(live).toContain('class="card-tile__name">Office air<');
    expect(live).toContain('aria-label="Remove Air quality — Office air');
    // Nothing on the tile says "plugin", and the wire id is still never shown.
    expect(live).not.toContain(">Plugin<");
    expect(live).not.toContain(plugin.id);

    // No headline yet reads like a weather card with no data, not like a fault.
    expect(render(catalog, [], "networked")).toContain(
      '<strong class="card-tile__value numeral">—</strong>',
    );
    // The two flags, each beside the bare id it explains.
    expect(render({ plugins: [], load_failures: [] }, [], "networked")).toContain(
      '<span class="flag">not on the server</span>',
    );
    expect(render(catalog, [], "local")).toContain('<span class="flag">needs the server</span>');
  });

  test("the add menu lists server plugins by display name, with description fallbacks", async () => {
    let added: { pluginId: string; refreshMinutes: number } | null = null;
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () =>
        root.render(
          <CardList
            config={cardListConfig([clockCard("clock", "Desk")])}
            issues={[]}
            cardData={[]}
            pomodoros={[]}
            providers={[]}
            pluginKinds={[
              {
                id: "aqi",
                version: "1.0.0",
                displayName: "Air quality",
                description: "EPA index for a location",
                onAdd: () => {
                  added = { pluginId: "aqi", refreshMinutes: 15 };
                },
              },
              {
                id: "agenda",
                version: "2.1.0",
                displayName: null,
                description: null,
                onAdd: () => {},
              },
            ]}
            catalog={null}
            serverCardState={[]}
            ownershipTier="networked"
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
      expect(container.textContent).toContain("Plugins on the server");
      expect(container.textContent).toContain("Air quality");
      expect(container.textContent).toContain("EPA index for a location");
      // Both fallbacks: the id names it, "Plugin · <version>" describes it.
      expect(container.textContent).toContain("agenda");
      expect(container.textContent).toContain("Plugin · 2.1.0");

      const row = [...container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(
        (button) => button.textContent?.includes("Air quality"),
      );
      await act(async () => row?.click());
      expect(added).toEqual({ pluginId: "aqi", refreshMinutes: 15 });
      expect(container.querySelector('[role="menu"]')).toBeNull();
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  test("the editor names the plugin, owns its issue, and says when it cannot change it", () => {
    const plugin = pluginCard();
    const catalog: PluginCatalog = {
      plugins: [
        {
          id: "com.example.air-quality",
          name: "aqi",
          version: "1.0.0",
          node_count: 7,
          assets: [],
          display_name: "Air quality",
          description: "EPA index for a location",
          manifest_version: 2,
          template: "display-list",
          refresh_minutes: 15,
        },
      ],
      load_failures: [],
    };

    const editable = renderCardEditor(plugin, [], null, catalog);
    expect(editable).toContain('id="editor-heading">Air quality<');
    expect(editable).toContain("<span>Plugin</span>");
    expect(editable).toContain("Air quality · 1.0.0");
    expect(editable).toContain("com.example.air-quality");
    expect(editable).toContain("<span>Name</span>");
    expect(editable).not.toMatch(/<select[^>]*disabled/);

    // A validation issue on the id now attaches to the control that can fix it.
    const flagged = renderCardEditor(
      plugin,
      [
        {
          path: "cards[0].plugin_id",
          code: "missing-reference",
          message: "That plugin is not installed on the server.",
        },
      ],
      null,
      catalog,
    );
    expect(flagged).toContain('aria-invalid="true"');
    expect(flagged).toContain("That plugin is not installed on the server.");

    // An id the catalog lacks stays selected, and says so rather than resetting.
    const unknown = renderCardEditor(
      { ...plugin, plugin_id: "com.example.gone" },
      [],
      null,
      catalog,
    );
    expect(unknown).toContain("Not installed on the server");

    // No catalog: read-only, with the reason the tier makes true.
    const localHtml = renderCardEditor(plugin, [], null, null, "local");
    expect(localHtml).toMatch(/<select[^>]*disabled/);
    expect(localHtml).toContain("Needs the server to render");
    expect(renderCardEditor(plugin, [], null, null, "networked")).toContain(
      "The plugin list comes from the server",
    );
  });

  test("row-list tiles count only populated title fields from real provider snapshots", () => {
    const calendar = calendarCard("calendar");
    const html = renderToStaticMarkup(
      <CardList
        config={cardListConfig([calendar])}
        issues={[]}
        cardData={[
          {
            card_id: calendar.id,
            fields: [
              { key: "row0_title", value: { kind: "text", value: "Standup" } },
              { key: "row0_time", value: { kind: "text", value: "09:30" } },
              { key: "row1_title", value: { kind: "text", value: "Design review" } },
              { key: "row1_time", value: { kind: "text", value: "13:00" } },
              { key: "row2_title", value: { kind: "text", value: "" } },
              { key: "row2_time", value: { kind: "text", value: "" } },
              { key: "row3_title", value: { kind: "text", value: "   " } },
              { key: "row3_time", value: { kind: "text", value: "" } },
              { key: "row4_title", value: { kind: "text", value: "" } },
              { key: "row4_time", value: { kind: "text", value: "" } },
            ],
          },
        ]}
        pomodoros={[]}
        providers={[]}
        pluginKinds={[]}
        catalog={null}
        serverCardState={[]}
        ownershipTier="networked"
        selectedCardId={calendar.id}
        onSelect={() => {}}
        onAdd={() => {}}
        onChange={() => {}}
        onRemove={() => {}}
      />,
    );
    expect(html).toContain('<strong class="card-tile__value numeral">2</strong>');
  });

  test("never shows the wire id", () => {
    // A distinctive id with no overlap with any visible label (unlike the
    // fixture's plain "clock", which is also a substring of the visible
    // "Digital clock" kind name and would make this assertion meaningless).
    const clock = clockCard("internal-uuid-0001", "Desk");
    const html = renderCardEditor(clock);
    expect(html).toContain("Show seconds");
    // The canvas dimensions were a caption that never changed and never told the
    // reader anything they could act on. It is gone; keep it gone.
    expect(html).not.toContain("448");
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
    expect(html).toContain("Swiping the screen moves through the loop.");
    expect(html).toContain("a tap dismisses it");
  });

  test("the selected card's timed-loop dwell field writes its active entry", async () => {
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
          issues={[]}
          entryIssues={[
            {
              path: "playlists[0].entries[0].dwell_seconds",
              code: "out-of-range",
              message: "Entry dwell is out of range.",
            },
          ]}
          cardError={null}
          pomodoro={null}
          provider={null}
          timerBusy={false}
          filePickerBusy={false}
          providerRefreshing={false}
          catalog={null}
          ownershipTier="networked"
          onChange={() => {}}
          onConfigChange={setConfig}
          onRemove={() => {}}
          onTimerAction={() => {}}
          onChooseCalendarFile={() => {}}
          onRefreshProvider={() => {}}
        />
      );
    }

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<Harness />));
      const dwell = container.querySelector<HTMLInputElement>(
        'input[aria-label="Stays on the panel for Digital clock — Desk"]',
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
      expect(latest.playlists[0].entries[0].dwell_seconds).toBe(45);
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  test("the dwell field is shown only when an enrolled card can use it or must repair it", () => {
    const card = clockCard("clock-1", "Desk");
    const issue: ValidationIssue = {
      path: "playlists[0].entries[0].dwell_seconds",
      code: "out-of-range",
      message: "Entry dwell is out of range.",
    };
    const renderDwell = (config: AppConfig, entryIssues: ValidationIssue[] = []) =>
      renderToStaticMarkup(
        <CardEditor
          card={card}
          config={config}
          issues={[]}
          entryIssues={entryIssues}
          cardError={null}
          pomodoro={null}
          provider={null}
          timerBusy={false}
          filePickerBusy={false}
          providerRefreshing={false}
          catalog={null}
          ownershipTier="networked"
          onChange={() => {}}
          onConfigChange={() => {}}
          onRemove={() => {}}
          onTimerAction={() => {}}
          onChooseCalendarFile={() => {}}
          onRefreshProvider={() => {}}
        />,
      );

    const outside = renderDwell(cardListConfig([card], []));
    expect(outside).not.toContain("Stays on the panel for");

    const manualNullConfig = cardListConfig([card]);
    manualNullConfig.playlists[0].advance = { kind: "manual" };
    const manualNull = renderDwell(manualNullConfig);
    expect(manualNull).not.toContain("Stays on the panel for");

    const manualInvalidConfig = structuredClone(manualNullConfig);
    manualInvalidConfig.playlists[0].entries[0].dwell_seconds = 2;
    const manualInvalid = renderDwell(manualInvalidConfig, [issue]);
    expect(manualInvalid).toContain("Stays on the panel for");
    expect(manualInvalid).toContain('aria-invalid="true"');
    expect(manualInvalid).toContain("Dwell applies when the loop is timed.");

    const timed = renderDwell(cardListConfig([card]));
    expect(timed).toContain("Stays on the panel for");
  });

  test("the grid renders loop entries first, then outsiders with consequence flags", () => {
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
        cardData={[]}
        pomodoros={[]}
        providers={[]}
        pluginKinds={[]}
        catalog={null}
        serverCardState={[]}
        ownershipTier="networked"
        selectedCardId="first-clock-id"
        onSelect={() => {}}
        onAdd={() => {}}
        onChange={() => {}}
        onRemove={() => {}}
      />,
    );
    expect(html).toContain('aria-label="Cards, loop order first"');
    expect(html).toContain("Desk");
    expect(html).toContain("Focus");
    expect(html).toContain("Spare");
    expect(html).toContain(">alerts<");
    expect(html).toContain(">not in loop<");
    expect(html).toContain(">Add to loop<");
    expect(html.indexOf("Desk")).toBeLessThan(html.indexOf("Focus"));
    expect(html.indexOf("Focus")).toBeLessThan(html.indexOf("Spare"));
    expect(html).not.toContain(">unused<");
    // Internal identifiers never reach the user — only their titles do.
    expect(html).not.toContain("internal-uuid-0001");
    expect(html).not.toContain("internal-uuid-0002");
    expect(html).not.toContain("internal-uuid-0003");
  });

  test("the grid renders active entries container issues and scopes resolved entry issues", () => {
    const config = cardListConfig([clockCard("first", "Desk"), clockCard("second", "Up next")]);
    const containerIssue: ValidationIssue = {
      path: "playlists[0].entries",
      code: "out-of-range",
      message: "The loop entries need attention.",
    };
    const secondDwellIssue: ValidationIssue = {
      path: "playlists[0].entries[1].dwell_seconds",
      code: "out-of-range",
      message: "The second dwell is out of range.",
    };
    const container = document.createElement("div");
    container.innerHTML = renderToStaticMarkup(
      <CardList
        config={config}
        issues={[containerIssue, secondDwellIssue]}
        cardData={[]}
        pomodoros={[]}
        providers={[]}
        pluginKinds={[]}
        catalog={null}
        serverCardState={[]}
        ownershipTier="networked"
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
        cardData={[]}
        pomodoros={[]}
        providers={[]}
        pluginKinds={[]}
        catalog={null}
        serverCardState={[]}
        ownershipTier="networked"
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

  test("the add slot distinguishes a full loop from a full card library", () => {
    const cards = Array.from({ length: 7 }, (_, index) => clockCard(`card-${index}`));
    const config = cardListConfig(cards, [
      ...cards.map((card) => ({ card_id: card.id, dwell_seconds: null })),
      { card_id: "missing-card", dwell_seconds: null },
    ]);
    const html = renderToStaticMarkup(
      <CardList
        config={config}
        issues={[]}
        cardData={[]}
        pomodoros={[]}
        providers={[]}
        pluginKinds={[]}
        catalog={null}
        serverCardState={[]}
        ownershipTier="networked"
        selectedCardId={null}
        onSelect={() => {}}
        onAdd={() => {}}
        onChange={() => {}}
        onRemove={() => {}}
      />,
    );
    expect(html).toContain('aria-describedby="add-card-capacity"');
    expect(html).toContain("The limit is 8 in the loop.");

    const outsider = clockCard("outside", "Spare");
    const fullLoopWithOutsider = { ...config, cards: [...config.cards, outsider] };
    const container = document.createElement("div");
    container.innerHTML = renderToStaticMarkup(
      <CardList
        config={fullLoopWithOutsider}
        issues={[]}
        cardData={[]}
        pomodoros={[]}
        providers={[]}
        pluginKinds={[]}
        catalog={null}
        serverCardState={[]}
        ownershipTier="networked"
        selectedCardId={null}
        onSelect={() => {}}
        onAdd={() => {}}
        onChange={() => {}}
        onRemove={() => {}}
      />,
    );
    expect(
      container.querySelector<HTMLButtonElement>(
        'button[aria-label="Add Digital clock — Spare to the loop"]',
      )?.disabled,
    ).toBe(true);
  });

  test("the add menu aligns to the slot end only when its measured width would overflow", async () => {
    const config = cardListConfig([clockCard("clock", "Desk")]);
    const container = document.createElement("div");
    container.className = "face__work";
    Object.defineProperty(container, "clientWidth", { configurable: true, value: 1000 });
    document.body.appendChild(container);
    const root = createRoot(container);

    try {
      await act(async () =>
        root.render(
          <CardList
            config={config}
            issues={[]}
            cardData={[]}
            pomodoros={[]}
            providers={[]}
            pluginKinds={[]}
            catalog={null}
            serverCardState={[]}
            ownershipTier="networked"
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

      slotRoot.getBoundingClientRect = () => ({ right: 800 }) as DOMRect;
      await act(async () => slot?.click());
      expect(container.querySelector(".menu")?.classList.contains("menu--end")).toBe(true);

      await act(async () => slot?.click());
      slotRoot.getBoundingClientRect = () => ({ right: 100 }) as DOMRect;
      await act(async () => slot?.click());
      expect(container.querySelector(".menu")?.classList.contains("menu--end")).toBe(false);
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
          cardData={[]}
          pomodoros={[]}
          providers={[]}
          pluginKinds={[]}
          catalog={null}
          serverCardState={[]}
          ownershipTier="networked"
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
      expect(container.textContent).not.toContain("Plugins on the server");

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
      const weather = [...container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(
        (button) => button.textContent?.includes("Weather"),
      );
      await act(async () => weather?.click());
      expect(latest.cards.at(-1)?.kind).toBe("weather");
      expect(latest.playlists[0].entries.at(-1)?.card_id).toBe("weather");
      expect(selected).toBe("weather");
      expect(container.querySelector('[role="menu"]')).toBeNull();
      expect(document.activeElement !== document.body).toBe(true);
      expect(document.activeElement?.classList.contains("card-tile__body")).toBe(true);
      expect(document.activeElement?.textContent).toContain("Weather");
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
            cardData={[]}
            pomodoros={[]}
            providers={[]}
            pluginKinds={[]}
            catalog={null}
            serverCardState={[]}
            ownershipTier="networked"
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

  test("menu arrow wrapping ignores stale refs after the option list shrinks", async () => {
    const config = cardListConfig([clockCard("clock", "Desk")]);
    let hidePlugin = () => {};
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    function Harness() {
      const [showPlugin, setShowPlugin] = useState(true);
      hidePlugin = () => setShowPlugin(false);
      return (
        <CardList
          config={config}
          issues={[]}
          cardData={[]}
          pomodoros={[]}
          providers={[]}
          pluginKinds={
            showPlugin
              ? [
                  {
                    id: "weather-plus",
                    version: "1",
                    displayName: null,
                    description: "Plugin weather",
                    onAdd: () => {},
                  },
                ]
              : []
          }
          catalog={null}
          serverCardState={[]}
          ownershipTier="networked"
          selectedCardId={null}
          onSelect={() => {}}
          onAdd={() => {}}
          onChange={() => {}}
          onRemove={() => {}}
        />
      );
    }

    try {
      await act(async () => root.render(<Harness />));
      const slot = container.querySelector<HTMLButtonElement>(".card-tile__add");
      await act(async () => slot?.click());
      expect(container.querySelectorAll('[role="menuitem"]')).toHaveLength(7);
      await act(async () => slot?.click());
      await act(async () => hidePlugin());
      await act(async () => slot?.click());

      const currentItems = container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]');
      expect(currentItems).toHaveLength(6);
      const firstItem = currentItems[0];
      const lastItem = currentItems[currentItems.length - 1];
      await act(async () =>
        firstItem.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true })),
      );
      expect(lastItem.textContent).toContain("RSS feed");
      expect(document.activeElement === lastItem).toBe(true);
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  test("grid move buttons and Alt arrows update active-loop order", async () => {
    const config = cardListConfig([clockCard("first", "Desk"), clockCard("second", "Up next")]);
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
          cardData={[]}
          pomodoros={[]}
          providers={[]}
          pluginKinds={[]}
          catalog={null}
          serverCardState={[]}
          ownershipTier="networked"
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
          'button[aria-label="Move Digital clock — Desk earlier"]',
        )?.disabled,
      ).toBe(true);
      expect(
        container.querySelector<HTMLButtonElement>(
          'button[aria-label="Move Digital clock — Up next later"]',
        )?.disabled,
      ).toBe(true);
      const earlier = container.querySelector<HTMLButtonElement>(
        'button[aria-label="Move Digital clock — Up next earlier"]',
      );
      expect(earlier).not.toBeNull();
      await act(async () => earlier?.click());
      expect(latest.playlists[0].entries.map((entry) => entry.card_id)).toEqual([
        "second",
        "first",
      ]);

      const later = container.querySelector<HTMLButtonElement>(
        'button[aria-label="Move Digital clock — Up next later"]',
      );
      await act(async () => later?.click());
      expect(latest.playlists[0].entries.map((entry) => entry.card_id)).toEqual([
        "first",
        "second",
      ]);

      let selectedTile = [
        ...container.querySelectorAll<HTMLButtonElement>(".card-tile .card-tile__body"),
      ].find((button) => button.textContent?.includes("Desk"));
      expect(selectedTile).not.toBeNull();
      await act(async () => {
        selectedTile?.dispatchEvent(
          new KeyboardEvent("keydown", { key: "ArrowRight", altKey: true, bubbles: true }),
        );
      });
      expect(latest.playlists[0].entries.map((entry) => entry.card_id)).toEqual([
        "second",
        "first",
      ]);

      selectedTile = [
        ...container.querySelectorAll<HTMLButtonElement>(".card-tile .card-tile__body"),
      ].find((button) => button.textContent?.includes("Desk"));
      await act(async () => {
        selectedTile?.dispatchEvent(
          new KeyboardEvent("keydown", { key: "ArrowLeft", altKey: true, bubbles: true }),
        );
      });
      expect(latest.playlists[0].entries.map((entry) => entry.card_id)).toEqual([
        "first",
        "second",
      ]);
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  test("duplicate loop entries use their rendered index for move boundaries", () => {
    const card = clockCard("duplicate", "Desk");
    const config = cardListConfig(
      [card],
      [
        { card_id: card.id, dwell_seconds: null },
        { card_id: card.id, dwell_seconds: 45 },
      ],
    );
    const container = document.createElement("div");
    container.innerHTML = renderToStaticMarkup(
      <CardList
        config={config}
        issues={[]}
        cardData={[]}
        pomodoros={[]}
        providers={[]}
        pluginKinds={[]}
        catalog={null}
        serverCardState={[]}
        ownershipTier="networked"
        selectedCardId={null}
        onSelect={() => {}}
        onAdd={() => {}}
        onChange={() => {}}
        onRemove={() => {}}
      />,
    );

    const earlier = container.querySelectorAll<HTMLButtonElement>(
      'button[aria-label="Move Digital clock — Desk earlier"]',
    );
    const later = container.querySelectorAll<HTMLButtonElement>(
      'button[aria-label="Move Digital clock — Desk later"]',
    );
    expect(earlier).toHaveLength(2);
    expect(later).toHaveLength(2);
    expect(earlier[0].disabled).toBe(true);
    expect(later[0].disabled).toBe(false);
    expect(earlier[1].disabled).toBe(false);
    expect(later[1].disabled).toBe(true);
  });

  test("keyboard reorder restores focus to the moved tile body", async () => {
    const config = cardListConfig([clockCard("first", "Desk"), clockCard("second", "Up next")]);
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    function Harness() {
      const [value, setValue] = useState(config);
      return (
        <CardList
          config={value}
          issues={[]}
          cardData={[]}
          pomodoros={[]}
          providers={[]}
          pluginKinds={[]}
          catalog={null}
          serverCardState={[]}
          ownershipTier="networked"
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
    const config = cardListConfig([clockCard("first", "Desk"), clockCard("second", "Up next")]);
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
            cardData={[]}
            pomodoros={[]}
            providers={[]}
            pluginKinds={[]}
            catalog={null}
            serverCardState={[]}
            ownershipTier="networked"
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

  test("an outside tile says why it is outside and enrols directly into the loop", async () => {
    const outside = pomodoroCard("outside", "Spare");
    const initial = cardListConfig(
      [clockCard("inside", "Desk"), outside],
      [{ card_id: "inside", dwell_seconds: null }],
    );
    let latest = initial;

    function Harness() {
      const [config, setConfig] = useState(initial);
      latest = config;
      return (
        <CardList
          config={config}
          issues={[]}
          cardData={[]}
          pomodoros={[]}
          providers={[]}
          pluginKinds={[]}
          catalog={null}
          serverCardState={[]}
          ownershipTier="networked"
          selectedCardId={null}
          onSelect={() => {}}
          onAdd={() => {}}
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
      expect(container.textContent).toContain("not in loop");
      expect(container.textContent).toContain("alerts");
      const join = container.querySelector<HTMLButtonElement>(
        'button[aria-label="Add Pomodoro — Spare to the loop"]',
      );
      await act(async () => join?.click());
      expect(latest.playlists[0].entries.at(-1)?.card_id).toBe("outside");
      expect(container.textContent).not.toContain("not in loop");
      expect(document.activeElement !== document.body).toBe(true);
      expect(document.activeElement?.classList.contains("card-tile__body")).toBe(true);
      expect(document.activeElement?.textContent).toContain("Spare");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  test("a missing-card entry remains visible with all of its entry issues and can be removed", async () => {
    const initial = cardListConfig(
      [clockCard("first", "Desk")],
      [{ card_id: "missing-card", dwell_seconds: null }],
    );
    const issues: ValidationIssue[] = [
      {
        path: "playlists[0].entries[0].card_id",
        code: "missing-reference",
        message: "This loop entry references a missing card.",
      },
      {
        path: "playlists[0].entries[0].dwell_seconds",
        code: "out-of-range",
        message: "Dwell time is out of range.",
      },
    ];
    let latest = initial;

    function Harness() {
      const [config, setConfig] = useState(initial);
      latest = config;
      return (
        <CardList
          config={config}
          issues={issues}
          cardData={[]}
          pomodoros={[]}
          providers={[]}
          pluginKinds={[]}
          catalog={null}
          serverCardState={[]}
          ownershipTier="networked"
          selectedCardId={null}
          onSelect={() => {}}
          onAdd={() => {}}
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
      expect(container.textContent).toContain("Missing card");
      expect(container.textContent).toContain("This loop entry references a missing card.");
      expect(container.textContent).toContain("Dwell time is out of range.");
      expect(container.querySelector(".card-tile.has-issue")).not.toBeNull();
      await act(async () =>
        container
          .querySelector<HTMLButtonElement>('button[aria-label="Remove Missing card"]')
          ?.click(),
      );
      expect(latest.playlists[0].entries).toEqual([]);
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
      expect(saved[0].active_playlist_id).toBe(snapshot.config.active_playlist_id);
      expect(saved[0].playlists[0].advance).toEqual({ kind: "manual" });
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
      throw new tauriModule.DeskmateCommandError({
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

  test("a protocol the app cannot speak is stated in the work column, not a status bar", async () => {
    // The header that used to carry this was removed; the sentence has to survive
    // the move or a mismatched display fails silently.
    snapshotImpl = async () => ({
      ...(structuredClone(snapshot) as AppSnapshot),
      device: { ...snapshot.device, protocol_version: 2 },
    });
    previewImpl = async () => ({ png_base64: "cHJldmlldw==", sample: false, state: null });

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<App />));
      await waitFor(() => expect(container.textContent).toContain("this app speaks protocol 1"));
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
          card_id: "calendar",
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
          card_id: "calendar",
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

  test("a feed in trouble is reported on its own card, not in a list of every feed", () => {
    const troubled = snapshot.providers.find((provider) => provider.state.kind === "stale");
    if (!troubled) {
      throw new Error("fixture no longer carries a provider in trouble");
    }
    expect(providerTrouble(troubled)).toContain("Showing the last good data");
    // A healthy provider is not news and says nothing at all.
    expect(providerTrouble({ ...troubled, state: { kind: "fresh" } })).toBeNull();

    const editor = renderToStaticMarkup(
      <CardEditor
        card={cards[0]}
        config={cardListConfig([cards[0]])}
        issues={[]}
        entryIssues={[]}
        cardError={null}
        pomodoro={null}
        provider={troubled}
        timerBusy={false}
        filePickerBusy={false}
        providerRefreshing={false}
        catalog={null}
        ownershipTier="networked"
        onChange={() => {}}
        onConfigChange={() => {}}
        onRemove={() => {}}
        onTimerAction={() => {}}
        onChooseCalendarFile={() => {}}
        onRefreshProvider={() => {}}
      />,
    );
    expect(editor).toContain("Showing the last good data");
    expect(editor).toContain("Refresh");
  });

  test("a live SVG refusal is visible in the affected card editor, not mislabeled globally or stale", () => {
    const refusal =
      "this card needs the display to keep time:HH:mm moving between pushes, but the card is an SVG template, which no device draws natively; a server-rendered image of it would freeze, so it is refused instead — update the display’s firmware to show this card";
    const editor = renderCardEditor(clockCard("clock-1", "Desk"), [], {
      kind: "scene-refused",
      card_id: "clock-1",
      message: refusal,
    });

    expect(editor).toContain('class="panel"');
    expect(editor).toContain("This card could not be rendered.");
    expect(editor).toContain(refusal);
    expect(editor).toContain("time:HH:mm");
    expect(editor).toContain("would freeze");
    expect(editor).toContain("Adjust the card and save to try again.");
    expect(editor).toContain('role="alert"');
    expect(editor).not.toContain("stale");
    expect(editor).not.toContain("last working settings");
    expect(editor).not.toContain("Transport error");
  });

  test("the settings sheet carries the link and ownership facts the header used to", () => {
    const html = renderNetworkPanel({
      tier: "networked",
      wifiState: "connected",
      ip: "192.168.1.42",
    });
    expect(html).toContain("/dev/cu.usbmodem2101");
    expect(ownershipLabel(null)).toBe("Ownership unavailable");
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
      return { png_base64: "Zmlyc3QtZnJhbWU=", sample: false, state: null };
    };
    const { container, root } = await mountPreview(calendarCard("upnext"));
    await waitFor(() => {
      const img = container.querySelector("img");
      expect(img).not.toBeNull();
      expect(img?.getAttribute("src")).toBe("data:image/png;base64,Zmlyc3QtZnJhbWU=");
    });
    expect(container.querySelector(".stage__badge")).toBeNull();
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
    previewImpl = async () => ({ png_base64: "dW5jb25maWd1cmVk", sample: true, state: null });
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
      return { png_base64: `frame-${calls}`, sample: false, state: null };
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
        return { png_base64: "first-frame", sample: false, state: null };
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
      schema_version: snapshot.config.schema_version,
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

  test("the loop ring shows the loop length and only in-rotation cards", () => {
    const html = renderLoopRing(filmstripConfig());
    expect(html).toMatch(/1 min 5 s/);
    expect(html).toContain("Desk");
    expect(html).toContain("Up next");
    expect(html).not.toContain("Focus");
  });

  test("the loop ring hides timings and the play control under manual advance", () => {
    const config = filmstripConfig();
    const html = renderLoopRing({
      ...config,
      playlists: [{ ...config.playlists[0], advance: { kind: "manual" } }],
    });
    expect(html).not.toMatch(/\d+s</);
    expect(html).not.toContain("Play the loop");
    expect(html).toContain("Desk");
  });

  test("the loop ring owns pacing and writes the active loop advance mode", async () => {
    const initial = filmstripConfig();
    let latest = initial;

    function Harness() {
      const [config, setConfig] = useState(initial);
      latest = config;
      return (
        <LoopRing
          config={config}
          issues={[
            {
              path: "playlists[0].advance.default_dwell_seconds",
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
      expect(latest.playlists[0].advance).toEqual({ kind: "manual" });
      expect(buttonWithText(container, "Manual")?.getAttribute("aria-pressed")).toBe("true");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  test("clicking the already-selected pacing mode does not emit a draft change", async () => {
    const config = filmstripConfig();
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
    const initial = filmstripConfig();
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
      expect(latest.playlists[0].advance).toEqual({
        kind: "timed",
        default_dwell_seconds: 20,
      });
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  test("the loop legend only displays and selects; it has no reorder affordance", () => {
    const initial = filmstripConfig();
    initial.playlists[0].entries = [
      { card_id: "first", dwell_seconds: 45 },
      { card_id: "missing-card", dwell_seconds: null },
      { card_id: "second", dwell_seconds: null },
    ];
    const html = renderLoopRing(initial);
    expect(html).not.toContain("draggable");
    expect(html).not.toContain("loop__moves");
    expect(html).not.toContain("Move ICS calendar");
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
      ...issuesForPath(issues, "playlists[0].advance"),
      ...issuesForPath(issues, "preferences.timezone"),
      ...unclaimedIssues(issues, config),
    ];

    expect(union).toHaveLength(issues.length);
    expect(new Set(union)).toEqual(new Set(issues));
  });

  test("active playlist index one claims and renders only its entries and advance issues", () => {
    const base = cardListConfig([clockCard("clock", "Desk")]);
    const active = base.playlists[0];
    const config: AppConfig = {
      ...base,
      playlists: [
        { id: "inactive", name: "Inactive", advance: { kind: "manual" }, entries: [] },
        active,
      ],
      active_playlist_id: active.id,
    };
    const issues: ValidationIssue[] = [
      { path: "playlists[1].entries", code: "entries", message: "Active entries issue." },
      {
        path: "playlists[1].entries[0].dwell_seconds",
        code: "dwell",
        message: "Active dwell issue.",
      },
      { path: "playlists[1].advance", code: "advance", message: "Active advance issue." },
      {
        path: "playlists[1].advance.default_dwell_seconds",
        code: "default-dwell",
        message: "Active default dwell issue.",
      },
      { path: "playlists", code: "root", message: "Playlists root issue." },
      { path: "playlists[0].id", code: "inactive-id", message: "Inactive id issue." },
      {
        path: "playlists[0].entries",
        code: "inactive-entries",
        message: "Inactive entries issue.",
      },
      {
        path: "active_playlist_id",
        code: "active-id",
        message: "Active playlist id issue.",
      },
    ];

    const grid = renderToStaticMarkup(
      <CardList
        config={config}
        issues={issues}
        cardData={[]}
        pomodoros={[]}
        providers={[]}
        pluginKinds={[]}
        catalog={null}
        serverCardState={[]}
        ownershipTier="networked"
        selectedCardId={null}
        onSelect={() => {}}
        onAdd={() => {}}
        onChange={() => {}}
        onRemove={() => {}}
      />,
    );
    const ring = renderToStaticMarkup(
      <LoopRing
        config={config}
        issues={issues}
        selectedCardId="clock"
        onSelect={() => {}}
        onChange={() => {}}
      />,
    );

    expect(grid).toContain("Active entries issue.");
    expect(grid).toContain("Active dwell issue.");
    expect(ring).toContain("Active advance issue.");
    expect(ring).toContain("Active default dwell issue.");
    expect(unclaimedIssues(issues, config).map((issue) => issue.path)).toEqual([
      "playlists",
      "playlists[0].id",
      "playlists[0].entries",
      "active_playlist_id",
    ]);
  });

  test("includes narrow-window and reduced-motion fallbacks", async () => {
    const css = await Bun.file(new URL("../src/styles.css", import.meta.url)).text();
    expect(css).toContain("@media (max-width: 430px)");
    expect(css).toContain("@media (prefers-reduced-motion: reduce)");
    expect(css).toContain("animation-duration: 0.001ms");
  });

  test("each server read names its backend command exactly, and passes no arguments", async () => {
    coreInvocations.length = 0;
    expect(await realGetServerPlugins()).toEqual({ plugins: [], load_failures: [] });
    expect(await realGetServerCardState()).toEqual([]);
    expect(coreInvocations).toEqual([
      { command: "get_server_plugins", args: undefined },
      { command: "get_server_card_state", args: undefined },
    ]);
  });
});
