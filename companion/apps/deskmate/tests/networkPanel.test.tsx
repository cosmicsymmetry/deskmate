import { afterEach, beforeEach, expect, test } from "bun:test";
import { act } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { NetworkPanel, ownershipLabel } from "../src/components/NetworkPanel";

import { resetBackendMocks } from "./support/backendMock";
import { installDomLifecycle, waitFor, buttonWithText } from "./support/dom";
import { installHttpLifecycle } from "./support/http";

beforeEach(resetBackendMocks);
const { mount, cleanupMountedRoots } = installDomLifecycle();
installHttpLifecycle(cleanupMountedRoots);
afterEach(resetBackendMocks);

function renderNetworkPanel({ wifiState, ip }: { wifiState: "down" | "connected"; ip: string }) {
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
        link: "network:desk-1",
        wifiState,
        wifiRssi: wifiState === "connected" ? -54 : null,
        ip,
        lastNetworkError: null,
        otaState: "idle",
      }}
      settings={publicSettingsWithStoredSecrets}
      onSignIn={async () => {}}
    />,
  );
}

test("network panel says where settings are written once networked", () => {
  const html = renderNetworkPanel({
    wifiState: "connected",
    ip: "192.168.1.42",
  });
  expect(ownershipLabel("networked")).toBe("Owned by the server");
  expect(html).toContain("192.168.1.42");
  expect(html).toContain("Settings are saved to the server");
});

test("network panel renders public settings but never a stored secret", () => {
  const html = renderNetworkPanel({
    wifiState: "connected",
    ip: "192.168.1.42",
  });
  expect(html).toContain("https://desk.example");
  expect(html).toContain("desk-1");
  expect(html).not.toContain("stored-wifi-secret");
  expect(html).not.toContain("stored-device-secret");
  expect(html).not.toContain("stored-admin-secret");
});

test("the web companion exposes no cable operations", () => {
  // Provisioning, ownership changes, and factory reset remain CLI-only. Assert
  // their visible labels are absent so an unimplemented control cannot quietly
  // reappear in the web companion.
  const html = renderNetworkPanel({
    wifiState: "connected",
    ip: "10.0.0.2",
  });
  expect(html).not.toContain("Pair with server");
  expect(html).not.toContain("Return to local ownership");
  expect(html).not.toContain("Factory-reset");
  expect(html).not.toContain("WiFi passphrase");
  expect(html).not.toContain("Device token");
});

test("signing in submits the device selection and token, then clears the token", async () => {
  const attempts: { deviceId: string; adminToken: string }[] = [];
  const { container, root, cleanup } = await mount();
  try {
    await act(async () =>
      root.render(
        <NetworkPanel
          device={{
            link: "server link",
            wifiState: "connected",
            wifiRssi: -54,
            ip: "10.0.0.2",
            lastNetworkError: null,
            otaState: "idle",
          }}
          settings={{ serverUrl: "https://desk.example", deviceId: "desk-1" }}
          onSignIn={async (deviceId, adminToken) => {
            attempts.push({ deviceId, adminToken });
          }}
        />,
      ),
    );

    const token = container.querySelector<HTMLInputElement>('input[type="password"]');
    const serverUrl = container.querySelector<HTMLInputElement>('input[type="url"]');
    expect(token).not.toBeNull();
    expect(serverUrl?.readOnly).toBe(true);
    expect(serverUrl?.value).toBe("https://desk.example");
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(
        token,
        "admin-secret",
      );
      token?.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => buttonWithText(container, "Sign in")?.click());

    await waitFor(() => expect(attempts).toHaveLength(1));
    expect(attempts[0]?.deviceId).toBe("desk-1");
    expect(attempts[0]?.adminToken).toBe("admin-secret");
    expect(serverUrl?.value).toBe("https://desk.example");
    // Cleared after use: the token buys a session and is not kept in the DOM
    // where a later screenshot or a stray autofill could resurface it.
    await waitFor(() => expect(token?.value).toBe(""));
  } finally {
    await cleanup();
  }
});

test("the settings sheet shows the device link and unavailable ownership", () => {
  const html = renderNetworkPanel({
    wifiState: "connected",
    ip: "192.168.1.42",
  });
  expect(html).toContain("network:desk-1");
  expect(ownershipLabel(null)).toBe("Ownership unavailable");
});
