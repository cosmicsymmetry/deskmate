import { useEffect, useState } from "react";

import type { DeviceOtaState, DeviceTier, DeviceWifiState } from "../lib/types";

interface NetworkDeviceView {
  tier: DeviceTier | null;
  /** The link to the display, in a word. Kept here rather than in the window
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
  onSaveServerAccess: (serverUrl: string, deviceId: string, adminToken: string) => Promise<void>;
}

/** Exported so the sheet can print it in its own head instead of this panel
 *  carrying a second heading inside a titled dialog. */
export function ownershipLabel(tier: DeviceTier | null): string {
  if (tier === "local") {
    return "Owned by a cable";
  }
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
 * Device facts, and the one credential this window needs.
 *
 * Provisioning, unpairing and factory reset used to live here. They are cable
 * operations -- they write the display's Wi-Fi and server settings over USB --
 * and a browser has no cable, so they are not offered rather than offered and
 * broken. `deskmate-cli` performs all of them; see the web-companion design's §2.
 */
export function NetworkPanel({ device, settings, onSaveServerAccess }: NetworkPanelProps) {
  const [serverUrl, setServerUrl] = useState(settings.serverUrl);
  const [deviceId, setDeviceId] = useState(settings.deviceId);
  const [adminToken, setAdminToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [announcement, setAnnouncement] = useState("");
  const [error, setError] = useState("");

  useEffect(() => setServerUrl(settings.serverUrl), [settings.serverUrl]);
  useEffect(() => setDeviceId(settings.deviceId), [settings.deviceId]);

  const signIn = async () => {
    setBusy(true);
    setError("");
    setAnnouncement("");
    try {
      await onSaveServerAccess(serverUrl, deviceId, adminToken);
      setAdminToken("");
      setAnnouncement("This browser is signed in.");
    } catch (next) {
      setError(next instanceof Error ? next.message : String(next));
    } finally {
      setBusy(false);
    }
  };

  const destination =
    device.tier === "local"
      ? "This display is owned by a cable. Settings saved here reach it only once it is owned by the server."
      : "Settings are saved to the server. The server sends them to this display.";

  return (
    <section className="network-panel" aria-label="Device ownership">
      <p className="settings-destination" role="status">
        {destination}
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
            <input type="url" value={serverUrl} maxLength={128} readOnly />
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
