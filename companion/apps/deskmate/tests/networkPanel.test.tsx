import { afterEach, beforeEach, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";

import { NetworkPanel, ownershipLabel } from "../src/components/NetworkPanel";

import { resetBackendMocks } from "./support/backendMock";
import { installDomLifecycle } from "./support/dom";
import { installHttpLifecycle } from "./support/http";

beforeEach(resetBackendMocks);
const { cleanupMountedRoots } = installDomLifecycle();
installHttpLifecycle(cleanupMountedRoots);
afterEach(resetBackendMocks);

function renderNetworkPanel({ wifiState, ip }: { wifiState: "down" | "connected"; ip: string }) {
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

test("network panel contains status only, not account or selection controls", () => {
  const html = renderNetworkPanel({
    wifiState: "connected",
    ip: "192.168.1.42",
  });
  expect(html).not.toContain("Server base URL");
  expect(html).not.toContain("Device ID");
  expect(html).not.toContain("Admin token");
  expect(html).not.toContain("Sign in");
});

test("the web companion exposes no cable operations", () => {
  // Panel setup has its own account flow. Assert its controls and the CLI-only
  // maintenance actions cannot quietly migrate into this status component.
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

test("the settings sheet shows the device link and unavailable ownership", () => {
  const html = renderNetworkPanel({
    wifiState: "connected",
    ip: "192.168.1.42",
  });
  expect(html).toContain("network:desk-1");
  expect(ownershipLabel(null)).toBe("Ownership unavailable");
});
