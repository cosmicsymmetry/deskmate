import type { DeviceOtaState, DeviceTier, DeviceWifiState } from "../lib/types";

interface NetworkDeviceView {
  /** The link to the display, in a word. Kept here rather than in the page
   *  chrome: it is a pairing-and-troubleshooting fact, and it belongs beside the
   *  rest of them. */
  link: string;
  wifiState: DeviceWifiState | null;
  wifiRssi: number | null;
  ip: string;
  lastNetworkError: string | null;
  otaState: DeviceOtaState | null;
}

interface NetworkPanelProps {
  device: NetworkDeviceView;
}

/** Exported so the sheet can print it in its own head instead of this panel
 *  carrying a second heading inside a titled dialog. */
export function ownershipLabel(tier: DeviceTier | null): string {
  return tier === "networked" ? "Owned by the server" : "Ownership unavailable";
}

function readableState(value: string | null): string {
  if (!value) {
    return "Unavailable";
  }
  return value
    .split("-")
    .map((part) => `${part.slice(0, 1).toUpperCase()}${part.slice(1)}`)
    .join(" ");
}

/**
 * Device network facts for pairing and troubleshooting.
 *
 * Panels are set up from "Add a panel" in account settings. `deskmate-cli`
 * remains the supported path for cable maintenance.
 */
export function NetworkPanel({ device }: NetworkPanelProps) {
  return (
    <section className="network-panel" aria-label="Device ownership">
      <p className="settings-destination" role="status">
        Settings are saved to the server. The server sends them to this display.
      </p>

      <dl className="network-status" aria-label="Device network status">
        <div className="network-status__wide">
          <dt>Link</dt>
          <dd title={device.link}>{device.link}</dd>
        </div>
        <div>
          <dt>WiFi</dt>
          <dd>{readableState(device.wifiState)}</dd>
        </div>
        <div>
          <dt>Signal</dt>
          <dd>{device.wifiRssi === null ? "Unavailable" : `${device.wifiRssi} dBm`}</dd>
        </div>
        <div>
          <dt>IP address</dt>
          <dd>{device.ip || "Not assigned"}</dd>
        </div>
        <div>
          <dt>Update</dt>
          <dd>{readableState(device.otaState)}</dd>
        </div>
      </dl>

      {device.lastNetworkError && (
        <p className="network-error" role="status">
          Last network error: {device.lastNetworkError}
        </p>
      )}
    </section>
  );
}
