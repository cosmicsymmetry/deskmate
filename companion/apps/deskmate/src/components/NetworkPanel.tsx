import { useEffect, useState } from "react";

import type { DeviceOtaState, DeviceTier, DeviceWifiState } from "../lib/types";
import type { PairDeviceInput } from "../lib/useAppState";

interface NetworkDeviceView {
  tier: DeviceTier | null;
  /** The USB link, in a word. Kept here rather than in the window chrome: it is a
   *  pairing-and-troubleshooting fact, and it belongs beside the rest of them. */
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
  ssid: string;
}

interface NetworkPanelProps {
  device: NetworkDeviceView;
  settings: NetworkPanelSettings;
  onPair: (input: PairDeviceInput) => Promise<void>;
  onUnpair: () => Promise<void>;
  onFactoryReset: () => Promise<void>;
  onSaveServerAccess?: (serverUrl: string, deviceId: string, adminToken: string) => Promise<void>;
  allowLocalOverride?: boolean;
  onUseLocalMode?: () => Promise<void>;
}

type NetworkAction = "pair" | "unpair" | "factory-reset" | "server-access" | "local-override";

/** Exported so the sheet can print it in its own head instead of this panel
 *  carrying a second heading inside a titled dialog. */
export function ownershipLabel(tier: DeviceTier | null): string {
  if (tier === "local") {
    return "Owned by this Mac";
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

export function NetworkPanel({
  device,
  settings,
  onPair,
  onUnpair,
  onFactoryReset,
  onSaveServerAccess,
  allowLocalOverride = false,
  onUseLocalMode,
}: NetworkPanelProps) {
  const [ssid, setSsid] = useState(settings.ssid);
  const [serverUrl, setServerUrl] = useState(settings.serverUrl);
  const [deviceId, setDeviceId] = useState(settings.deviceId);
  const [passphrase, setPassphrase] = useState("");
  const [deviceToken, setDeviceToken] = useState("");
  const [adminToken, setAdminToken] = useState("");
  const [busy, setBusy] = useState<NetworkAction | null>(null);
  const [announcement, setAnnouncement] = useState("");
  const [error, setError] = useState("");

  useEffect(() => setSsid(settings.ssid), [settings.ssid]);
  useEffect(() => setServerUrl(settings.serverUrl), [settings.serverUrl]);
  useEffect(() => setDeviceId(settings.deviceId), [settings.deviceId]);

  const run = async (
    action: NetworkAction,
    operation: () => Promise<void>,
    success: string,
    clear: "admin" | "all" | "none" = "none",
  ) => {
    setBusy(action);
    setError("");
    setAnnouncement("");
    try {
      await operation();
      if (clear === "admin" || clear === "all") {
        setAdminToken("");
      }
      if (clear === "all") {
        setPassphrase("");
        setDeviceToken("");
      }
      setAnnouncement(success);
    } catch (next) {
      setError(next instanceof Error ? next.message : String(next));
    } finally {
      setBusy(null);
    }
  };

  const local = device.tier === "local";
  const networked = device.tier === "networked";
  const destination = networked
    ? "Settings are saved to the server. The server sends them to this display."
    : local
      ? "Settings are saved on this Mac and sent to the display over USB."
      : "Connect over USB to confirm where settings will be saved.";
  const pairReady =
    ssid.trim() !== "" &&
    serverUrl.trim() !== "" &&
    deviceId.trim() !== "" &&
    deviceToken !== "" &&
    adminToken !== "";

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
          void run(
            "pair",
            () =>
              onPair({
                ssid,
                passphrase,
                server_url: serverUrl,
                device_id: deviceId,
                device_token: deviceToken,
                admin_token: adminToken,
                tier: "networked",
              }),
            "Pairing settings were written over USB. The display will reconnect to the server.",
            "all",
          );
        }}
      >
        <div className="network-fields">
          <label className="field">
            <span>WiFi network</span>
            <input
              value={ssid}
              maxLength={32}
              autoComplete="off"
              onChange={(event) => setSsid(event.currentTarget.value)}
            />
          </label>
          <label className="field">
            <span>WiFi passphrase</span>
            <input
              type="password"
              value={passphrase}
              maxLength={64}
              autoComplete="new-password"
              onChange={(event) => setPassphrase(event.currentTarget.value)}
            />
            <small>
              Write-only. It is cleared after submission and is never read from the device.
            </small>
          </label>
          <label className="field network-field--wide">
            <span>Server base URL</span>
            <input
              type="url"
              value={serverUrl}
              maxLength={128}
              placeholder="https://desk.example"
              onChange={(event) => setServerUrl(event.currentTarget.value)}
            />
            <small>Enter the HTTPS base. Deskmate derives the secure WebSocket device link.</small>
          </label>
          <label className="field">
            <span>Device ID</span>
            <input
              value={deviceId}
              maxLength={32}
              autoComplete="off"
              onChange={(event) => setDeviceId(event.currentTarget.value)}
            />
          </label>
          <label className="field">
            <span>Device token</span>
            <input
              type="password"
              value={deviceToken}
              maxLength={128}
              autoComplete="new-password"
              onChange={(event) => setDeviceToken(event.currentTarget.value)}
            />
            <small>Write-only. Enter it when pairing; Deskmate never displays it back.</small>
          </label>
          <label className="field">
            <span>Admin token</span>
            <input
              type="password"
              value={adminToken}
              autoComplete="new-password"
              onChange={(event) => setAdminToken(event.currentTarget.value)}
            />
            <small>Write-only. Re-enter it after relaunch before saving to the server.</small>
          </label>
        </div>

        <div className="network-actions">
          {allowLocalOverride && onUseLocalMode && (
            <p className="network-local-recovery">
              Pairing was not confirmed over USB? This changes only the Mac's routing; it does not
              reset the display.
            </p>
          )}
          <button
            className="button button--primary"
            type="submit"
            disabled={busy !== null || !pairReady}
          >
            {busy === "pair" ? "Pairing…" : "Pair with server"}
          </button>
          <button
            className="button button--quiet"
            type="button"
            disabled={busy !== null || !onSaveServerAccess || !serverUrl.trim() || !adminToken}
            onClick={() => {
              if (!onSaveServerAccess) {
                return;
              }
              void run(
                "server-access",
                () => onSaveServerAccess(serverUrl, deviceId, adminToken),
                "Server access was saved for this app session.",
                "admin",
              );
            }}
          >
            {busy === "server-access" ? "Saving access…" : "Save server access"}
          </button>
          {allowLocalOverride && onUseLocalMode && (
            <button
              className="button button--quiet"
              type="button"
              disabled={busy !== null}
              onClick={() =>
                void run(
                  "local-override",
                  onUseLocalMode,
                  "This Mac will use local routing. The display was not changed.",
                )
              }
            >
              {busy === "local-override" ? "Switching…" : "Use local on this Mac"}
            </button>
          )}
          <button
            className="button button--quiet"
            type="button"
            disabled={busy !== null || !networked}
            onClick={() =>
              void run("unpair", onUnpair, "Local ownership was requested over USB.", "all")
            }
          >
            {busy === "unpair" ? "Unpairing…" : "Return to local ownership"}
          </button>
          <button
            className="button button--danger"
            type="button"
            disabled={busy !== null}
            onClick={() =>
              void run(
                "factory-reset",
                onFactoryReset,
                "The display's network settings were erased.",
                "all",
              )
            }
          >
            {busy === "factory-reset" ? "Resetting…" : "Factory-reset network settings"}
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
