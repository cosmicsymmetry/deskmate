import type { AppSnapshot, IpcError } from "../lib/types";

interface DeviceHeaderProps {
  snapshot: AppSnapshot;
  commandError: IpcError | null;
  busyAction: string | null;
  onTogglePause: () => void;
}

function connectionCopy(snapshot: AppSnapshot): { label: string; detail: string; tone: string } {
  const { connection } = snapshot.device;
  switch (connection.kind) {
    case "online":
      return {
        label: "Connected",
        detail: snapshot.device.port_name ?? "USB device ready",
        tone: "positive",
      };
    case "connecting":
      return { label: "Looking for your display", detail: "Connect it with USB", tone: "busy" };
    case "standalone":
      return {
        label: "Display is in standalone mode",
        detail: "Your settings are saved and will sync when USB returns",
        tone: "warning",
      };
    case "disconnected":
      return {
        label: "Display not connected",
        detail: connection.reason ?? "Connect it with USB to sync",
        tone: "neutral",
      };
  }
}

export function DeviceHeader({
  snapshot,
  commandError,
  busyAction,
  onTogglePause,
}: DeviceHeaderProps) {
  const connection = connectionCopy(snapshot);
  const paused = snapshot.config.preferences.paused || snapshot.runtime.kind === "paused";
  const protocolMismatch =
    snapshot.device.protocol_version !== null && snapshot.device.protocol_version !== 1;

  return (
    <header className="device-header">
      <div className="brand-lockup">
        <span className="brand-mark" aria-hidden="true">
          D
        </span>
        <div>
          <p className="eyebrow">Deskmate</p>
          <h1>Display settings</h1>
        </div>
      </div>
      <div className="device-summary" aria-live="polite">
        <span className={`status-dot status-dot--${connection.tone}`} aria-hidden="true" />
        <span>
          <strong>{connection.label}</strong>
          <small>{connection.detail}</small>
        </span>
      </div>
      <button
        className="button button--quiet"
        type="button"
        onClick={onTogglePause}
        disabled={busyAction === "pause"}
      >
        {busyAction === "pause" ? "Updating…" : paused ? "Resume syncing" : "Pause syncing"}
      </button>
      {(protocolMismatch || snapshot.runtime.kind === "error" || commandError) && (
        <div className="header-alert" role="alert">
          {protocolMismatch
            ? `This display uses protocol ${snapshot.device.protocol_version}; this app supports protocol 1.`
            : snapshot.runtime.kind === "error"
              ? snapshot.runtime.message
              : commandError?.message}
        </div>
      )}
    </header>
  );
}
