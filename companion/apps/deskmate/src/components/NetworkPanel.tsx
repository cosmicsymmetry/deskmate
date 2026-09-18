import { useEffect, useState } from "react";

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

interface NetworkPanelSettings {
  serverUrl: string;
  deviceId: string;
}

interface NetworkPanelProps {
  device: NetworkDeviceView;
  settings: NetworkPanelSettings;
  /** Signs this browser in: the admin token is traded for a session. */
  onSignIn: (deviceId: string, adminToken: string) => Promise<void>;
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
 * Device facts, device selection, and the one credential this browser needs.
 *
 * Cable operations are intentionally absent: provisioning, unpairing and factory
 * reset write the display's own settings over USB, and the web companion does not
 * implement cable access. `deskmate-cli` is the supported path for those operations.
 */
export function NetworkPanel({ device, settings, onSignIn }: NetworkPanelProps) {
  const [deviceId, setDeviceId] = useState(settings.deviceId);
  const [adminToken, setAdminToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [announcement, setAnnouncement] = useState("");
  const [error, setError] = useState("");

  useEffect(() => setDeviceId(settings.deviceId), [settings.deviceId]);

  const signIn = async () => {
    setBusy(true);
    setError("");
    setAnnouncement("");
    try {
      await onSignIn(deviceId, adminToken);
      setAdminToken("");
      setAnnouncement("This browser is signed in.");
    } catch (next) {
      setError(next instanceof Error ? next.message : String(next));
    } finally {
      setBusy(false);
    }
  };

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

      <form
        className="network-form"
        onSubmit={(event) => {
          event.preventDefault();
          void signIn();
        }}
      >
        <div className="network-fields">
          <label className="field network-field--wide">
            <span>Server base URL</span>
            <input type="url" value={settings.serverUrl} maxLength={128} readOnly />
            <small>
              This page is served by the server it configures, so this is its own address.
            </small>
          </label>
          <label className="field">
            <span>Device ID</span>
            <input
              value={deviceId}
              maxLength={32}
              autoComplete="off"
              onChange={(event) => setDeviceId(event.currentTarget.value)}
            />
            <small>Leave as-is unless this server owns more than one display.</small>
          </label>
          <label className="field">
            <span>Admin token</span>
            <input
              type="password"
              value={adminToken}
              autoComplete="current-password"
              onChange={(event) => setAdminToken(event.currentTarget.value)}
            />
            <small>
              Write-only. Traded for a session this browser keeps; never stored where script can
              read it.
            </small>
          </label>
        </div>

        <div className="network-actions">
          <button
            className="button button--primary"
            type="submit"
            disabled={busy || adminToken === ""}
          >
            {busy ? "Signing in…" : "Sign in"}
          </button>
        </div>
      </form>

      <div className="network-announcement" aria-live="polite">
        {announcement && <span className="save-success">{announcement}</span>}
        {error && (
          <span className="save-error" role="alert">
            {error}
          </span>
        )}
      </div>
    </section>
  );
}
