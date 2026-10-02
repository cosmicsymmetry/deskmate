import { type FormEvent, useEffect, useRef, useState } from "react";

import { claimPanel, listPanels } from "../lib/account";
import {
  type PanelPort,
  requestPanelPort,
  serialSupported as browserSerialSupported,
} from "../lib/serial/port";
import {
  PanelSetup as PanelSetupMachine,
  type SetupDeps,
  type SetupStep,
} from "../lib/serial/setup";
import { PRODUCT_NAME } from "../lib/product";

export interface SetupController {
  connect(): Promise<void>;
  submitWifi(ssid: string, password: string): Promise<void>;
}

export interface PanelSetupDependencies {
  serialSupported(): boolean;
  requestPort(): Promise<PanelPort>;
  createSetup(deps: SetupDeps, onStep: (step: SetupStep) => void): SetupController;
}

type MockPanelPortGlobal = typeof globalThis & {
  __DESKMATE_MOCK_PANEL_PORT__?: () => PanelPort;
};

function mockPanelPortFactory(): (() => PanelPort) | undefined {
  // Vite folds this condition away in production, so the fake itself remains
  // reachable only from the mock build that installs the global factory.
  return import.meta.env.VITE_DESKMATE_MOCK === "1"
    ? (globalThis as MockPanelPortGlobal).__DESKMATE_MOCK_PANEL_PORT__
    : undefined;
}

const dependenciesForBrowser: PanelSetupDependencies = {
  serialSupported: () => mockPanelPortFactory() !== undefined || browserSerialSupported(),
  requestPort: () => {
    const mockFactory = mockPanelPortFactory();
    return mockFactory ? Promise.resolve(mockFactory()) : requestPanelPort();
  },
  createSetup: (deps, onStep) => new PanelSetupMachine(deps, onStep),
};

type ViewStep = { kind: "intro" } | SetupStep;

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}

function WifiForm({
  ssid,
  password,
  error,
  busy,
  onSsidChange,
  onPasswordChange,
  onSubmit,
}: {
  ssid: string;
  password: string;
  error?: string;
  busy: boolean;
  onSsidChange: (value: string) => void;
  onPasswordChange: (value: string) => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
}) {
  return (
    <form className="form-grid" onSubmit={onSubmit}>
      {error && (
        <p className="save-error" role="alert">
          {error}
        </p>
      )}
      <label className="field">
        <span>Wi-Fi network</span>
        <input
          name="wifi-network"
          value={ssid}
          autoComplete="off"
          required
          onChange={(event) => onSsidChange(event.currentTarget.value)}
        />
      </label>
      <label className="field">
        <span>Wi-Fi password</span>
        <input
          name="wifi-password"
          type="password"
          value={password}
          autoComplete="current-password"
          onChange={(event) => onPasswordChange(event.currentTarget.value)}
        />
        <small>
          Your Wi-Fi password goes to the panel over the cable. It is never sent to {PRODUCT_NAME}'s
          server.
        </small>
      </label>
      <div className="network-actions">
        <button
          className="button button--primary"
          type="submit"
          disabled={busy || ssid.trim() === ""}
        >
          {busy ? "Connecting…" : "Connect to Wi-Fi"}
        </button>
      </div>
    </form>
  );
}

export function PanelSetup({
  onDone,
  standalone = false,
  headingId,
  dependencies = dependenciesForBrowser,
}: {
  onDone: () => void;
  standalone?: boolean;
  headingId?: string;
  dependencies?: PanelSetupDependencies;
}) {
  const [step, setStep] = useState<ViewStep>({ kind: "intro" });
  const [ssid, setSsid] = useState("");
  const [password, setPassword] = useState("");
  const [submittingWifi, setSubmittingWifi] = useState(false);
  const setupRef = useRef<SetupController | null>(null);
  const portRef = useRef<PanelPort | null>(null);

  useEffect(
    () => () => {
      const port = portRef.current;
      portRef.current = null;
      if (port) void port.close().catch(() => {});
    },
    [],
  );

  if (!dependencies.serialSupported()) {
    return <p>Setting up a panel needs Chrome or Edge on a computer.</p>;
  }

  const connect = async () => {
    setStep({ kind: "connecting" });
    try {
      const previousPort = portRef.current;
      if (previousPort) await previousPort.close().catch(() => {});
      const port = await dependencies.requestPort();
      portRef.current = port;
      const setup = dependencies.createSetup(
        {
          port,
          claim: claimPanel,
          isLinked: async (deviceId) =>
            (await listPanels()).some(
              (panel) => panel.id === deviceId && panel.connected && panel.state === "active",
            ),
          utcOffsetMinutes: () => -new Date().getTimezoneOffset(),
          sleep,
          now: Date.now,
        },
        setStep,
      );
      setupRef.current = setup;
      await setup.connect();
    } catch {
      setStep({ kind: "no-response" });
    }
  };

  const submitWifi = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const setup = setupRef.current;
    if (!setup || ssid.trim() === "") return;
    setSubmittingWifi(true);
    void setup
      .submitWifi(ssid, password)
      .catch((error) =>
        setStep({
          kind: "wifi-failed",
          boardError: error instanceof Error ? error.message : "The panel refused its settings.",
        }),
      )
      .finally(() => setSubmittingWifi(false));
  };

  const heading = standalone ? (
    <h1 id={headingId}>Add your panel</h1>
  ) : (
    <h3 id={headingId}>Add your panel</h3>
  );
  const wifiError =
    step.kind === "wifi-failed" ? `The panel couldn't join ${ssid}: ${step.boardError}` : undefined;

  return (
    <div className="form-grid" aria-live="polite">
      {heading}
      {step.kind === "intro" && (
        <>
          <p>Plug the panel into this computer with its USB cable.</p>
          <div className="network-actions">
            <button className="button button--primary" type="button" onClick={() => void connect()}>
              Connect
            </button>
          </div>
        </>
      )}
      {step.kind === "connecting" && <p>Connecting to the panel…</p>}
      {step.kind === "incompatible" && (
        <p className="save-error" role="alert">
          This panel needs a firmware update first.
        </p>
      )}
      {step.kind === "no-response" && (
        <>
          <p className="save-error" role="alert">
            The panel didn't respond. Unplug it, plug it back in, and try again.
          </p>
          <div className="network-actions">
            <button className="button button--primary" type="button" onClick={() => void connect()}>
              Try again
            </button>
          </div>
        </>
      )}
      {(step.kind === "wifi-form" || step.kind === "wifi-failed") && (
        <WifiForm
          ssid={ssid}
          password={password}
          error={wifiError}
          busy={submittingWifi}
          onSsidChange={setSsid}
          onPasswordChange={setPassword}
          onSubmit={submitWifi}
        />
      )}
      {step.kind === "writing" && <p>Sending the network settings to your panel…</p>}
      {step.kind === "wifi-joining" && <p>{`Joining ${ssid}…`}</p>}
      {step.kind === "linking" && (
        <p>Connected to Wi-Fi. Waiting for the panel to reach the server…</p>
      )}
      {step.kind === "unreachable-server" && (
        <p className="save-error" role="alert">
          The panel is on Wi-Fi but hasn't reached the server yet. Leave it plugged in; it keeps
          trying. You can close this page.
        </p>
      )}
      {step.kind === "linked" && (
        <>
          <p>Your panel is connected. You can unplug it from the computer.</p>
          <div className="network-actions">
            <button className="button button--primary" type="button" onClick={onDone}>
              Done
            </button>
          </div>
        </>
      )}
    </div>
  );
}
