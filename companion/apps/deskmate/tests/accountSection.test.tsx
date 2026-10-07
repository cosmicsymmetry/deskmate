import { expect, test } from "bun:test";
import { act } from "react";

import { AccountSection, type AccountSectionApi } from "../src/components/AccountSection";
import {
  PanelSetup,
  type PanelSetupDependencies,
  type SetupController,
} from "../src/components/PanelSetup";
import type { Account, Instance, PanelRow } from "../src/lib/account";
import { MockPanelPort } from "../src/dev/mockBackend";
import { PanelLink } from "../src/lib/serial/port";
import type { SetupStep } from "../src/lib/serial/setup";

import { buttonWithText, installDomLifecycle, waitFor } from "./support/dom";

const { mount } = installDomLifecycle();

const instance: Instance = {
  setup_required: false,
  google_enabled: true,
  email_delivery: "email",
  signups_open: true,
  edition: "self-hosted",
};

const owner: Account = {
  id: "account-owner",
  email: "owner@example.com",
  email_verified: true,
  is_instance_owner: true,
};

const activePanel: PanelRow = {
  id: "desk-0001",
  connected: true,
  has_saved_config: true,
  configured_at: 1_795_000_000,
  state: "active",
};

function accountApi(overrides: Partial<AccountSectionApi> = {}): AccountSectionApi {
  return {
    getAccount: async () => owner,
    signOut: async () => {},
    signOutEverywhere: async () => {},
    deleteAccount: async () => {},
    setSignupsOpen: async (open) => open,
    listPanels: async () => [activePanel],
    removePanel: async () => {},
    ...overrides,
  };
}

async function changeInput(input: HTMLInputElement | null, value: string) {
  expect(input).not.toBeNull();
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, value);
    input?.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

test("unsupported browsers replace Add a panel with the desktop Chromium requirement", async () => {
  const { container, cleanup } = await mount(
    <AccountSection
      instance={instance}
      api={accountApi()}
      serialSupported={() => false}
      open
      onSessionEnded={() => {}}
      onPanelsChanged={() => {}}
    />,
  );
  try {
    await waitFor(() => expect(container.textContent).toContain(owner.email));
    expect(container.textContent).toContain(
      "Setting up a panel needs Chrome or Edge on a computer.",
    );
    expect(buttonWithText(container, "Add a panel")).toBeUndefined();
  } finally {
    await cleanup();
  }
});

test("PanelSetup renders every setup state and keeps the Wi-Fi form for a retry", async () => {
  let emit: ((step: SetupStep) => void) | undefined;
  let submitted: { ssid: string; password: string } | undefined;
  const controller: SetupController = {
    connect: async () => {},
    submitWifi: async (ssid, password) => {
      submitted = { ssid, password };
    },
  };
  const dependencies: PanelSetupDependencies = {
    serialSupported: () => true,
    requestPort: async () => ({
      open: async () => {},
      write: async () => {},
      readable: async function* () {},
      close: async () => {},
      restart: async () => {},
      onDisconnect: () => {},
    }),
    createSetup: (_deps, onStep) => {
      emit = onStep;
      return controller;
    },
  };

  const { container, cleanup } = await mount(
    <PanelSetup dependencies={dependencies} onDone={() => {}} />,
  );
  try {
    expect(container.textContent).toContain("Add your panel");
    expect(container.textContent).toContain(
      "Plug the panel into this computer with its USB cable.",
    );
    await act(async () => buttonWithText(container, "Connect")?.click());
    expect(emit).toBeDefined();

    await act(async () => emit?.({ kind: "connecting" }));
    expect(container.textContent).toContain("Connecting to the panel…");

    await act(async () => emit?.({ kind: "incompatible" }));
    expect(container.textContent).toContain("This panel needs a firmware update first.");

    await act(async () => emit?.({ kind: "no-response" }));
    expect(container.textContent).toContain(
      "The panel didn't respond. Unplug it, plug it back in, and try again.",
    );

    await act(async () => emit?.({ kind: "wifi-form" }));
    expect(container.textContent).toContain("Wi-Fi network");
    expect(container.textContent).toContain("Wi-Fi password");
    expect(container.textContent).toContain(
      "Your Wi-Fi password goes to the panel over the cable. It is never sent to Deskboy's server.",
    );
    await changeInput(container.querySelector('input[name="wifi-network"]'), "Studio");
    await changeInput(container.querySelector('input[name="wifi-password"]'), "secret");
    await act(async () => buttonWithText(container, "Connect to Wi-Fi")?.click());
    expect(submitted).toEqual({ ssid: "Studio", password: "secret" });

    await act(async () => emit?.({ kind: "writing" }));
    expect(container.textContent).toContain("Sending the network settings to your panel…");

    await act(async () => emit?.({ kind: "wifi-joining" }));
    expect(container.textContent).toContain("Joining Studio…");

    await act(async () => emit?.({ kind: "wifi-failed", boardError: "Wrong password" }));
    expect(container.textContent).toContain("The panel couldn't join Studio: Wrong password");
    expect(buttonWithText(container, "Connect to Wi-Fi")).toBeDefined();

    await act(async () => emit?.({ kind: "linking" }));
    expect(container.textContent).toContain(
      "Connected to Wi-Fi. Waiting for the panel to reach the server…",
    );

    await act(async () => emit?.({ kind: "unreachable-server", deviceId: "desk-0002" }));
    expect(container.textContent).toContain(
      "The panel is on Wi-Fi but hasn't reached the server yet. Leave it plugged in; it keeps trying. You can close this page.",
    );

    await act(async () => emit?.({ kind: "linked", deviceId: "desk-0002" }));
    expect(container.textContent).toContain(
      "Your panel is connected. You can unplug it from the computer.",
    );
    expect(buttonWithText(container, "Done")).toBeDefined();
  } finally {
    await cleanup();
  }
});

test("the dev fake answers the real status and network-config exchange", async () => {
  const port = new MockPanelPort();
  await port.open();
  const link = new PanelLink(port);

  const before = await link.status();
  expect(before.wifiState).toBe(0);
  await link.networkConfig({
    ssid: "Studio",
    psk: "secret",
    serverUrl: "wss://desk.example/v1/device/link",
    deviceId: "desk-0002",
    token: "device-token",
    utcOffsetMinutes: 240,
    tier: 1,
  });
  const after = await link.status();
  expect(after.wifiState).toBe(2);
  expect(after.capabilities & (1n << 7n)).not.toBe(0n);

  await port.close();
});

test("deleting an account requires the in-sheet confirmation", async () => {
  let deleteCalls = 0;
  const { container, cleanup } = await mount(
    <AccountSection
      instance={instance}
      api={accountApi({
        deleteAccount: async () => {
          deleteCalls += 1;
        },
      })}
      serialSupported={() => true}
      open
      onSessionEnded={() => {}}
      onPanelsChanged={() => {}}
    />,
  );
  try {
    await waitFor(() => expect(container.textContent).toContain(owner.email));
    await act(async () => buttonWithText(container, "Delete account")?.click());
    expect(deleteCalls).toBe(0);
    expect(container.textContent).toContain(
      "This deletes your cards and releases your panels. It can't be undone.",
    );
    expect(buttonWithText(container, "Keep it")).toBeDefined();

    await act(async () => buttonWithText(container, "Delete my account")?.click());
    await waitFor(() => expect(deleteCalls).toBe(1));
  } finally {
    await cleanup();
  }
});

test("a non-owner does not see the sign-ups switch", async () => {
  const ordinaryAccount = { ...owner, is_instance_owner: false };
  const { container, cleanup } = await mount(
    <AccountSection
      instance={instance}
      api={accountApi({ getAccount: async () => ordinaryAccount })}
      serialSupported={() => true}
      open
      onSessionEnded={() => {}}
      onPanelsChanged={() => {}}
    />,
  );
  try {
    await waitFor(() => expect(container.textContent).toContain(ordinaryAccount.email));
    expect(container.textContent).not.toContain("Let new people sign up");
  } finally {
    await cleanup();
  }
});

test("removing a panel confirms, calls the server, and drops its row", async () => {
  const removed: string[] = [];
  const { container, cleanup } = await mount(
    <AccountSection
      instance={instance}
      api={accountApi({
        removePanel: async (id) => {
          removed.push(id);
        },
      })}
      serialSupported={() => true}
      open
      onSessionEnded={() => {}}
      onPanelsChanged={() => {}}
    />,
  );
  try {
    await waitFor(() => expect(container.textContent).toContain(activePanel.id));
    expect(container.textContent).toContain("Online");
    await act(async () => buttonWithText(container, "Remove")?.click());
    expect(removed).toEqual([]);
    await act(async () => buttonWithText(container, "Remove panel")?.click());
    await waitFor(() => expect(removed).toEqual([activePanel.id]));
    expect(container.textContent).not.toContain(activePanel.id);
  } finally {
    await cleanup();
  }
});

test("an existing panel offers Change Wi-Fi, which opens the cable flow for that panel", async () => {
  const requests: string[] = [];
  const originalFetch = globalThis.fetch;
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    requests.push(`${init?.method ?? "GET"} ${String(input)}`);
    return new Response(
      JSON.stringify({ device_id: "desk-0001", token: "t", link_url: "wss://x/v1/device/link" }),
      { status: 200, headers: { "content-type": "application/json" } },
    );
  }) as typeof fetch;
  Object.defineProperty(navigator, "serial", { value: {}, configurable: true });
  const { container, cleanup } = await mount(
    <AccountSection
      instance={instance}
      api={accountApi()}
      serialSupported={() => true}
      open
      onSessionEnded={() => {}}
      onPanelsChanged={() => {}}
    />,
  );
  try {
    await waitFor(() => expect(buttonWithText(container, "Change Wi-Fi")).toBeDefined());
    await act(async () => buttonWithText(container, "Change Wi-Fi")?.click());
    expect(container.textContent).toContain("Change Wi-Fi on desk-0001");
  } finally {
    globalThis.fetch = originalFetch;
    Reflect.deleteProperty(navigator, "serial");
    await cleanup();
  }
});
