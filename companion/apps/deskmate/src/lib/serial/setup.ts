import { CAPABILITY_NETWORKING, PROTOCOL_VERSION, type NetworkConfig } from "./codec";
import { PanelDeviceError, PanelDisconnectedError, PanelLink, type PanelPort } from "./port";

export type SetupStep =
  | { kind: "connecting" }
  | { kind: "incompatible"; message: string }
  | { kind: "no-response" }
  | { kind: "wifi-form"; error?: string }
  | { kind: "writing" }
  | { kind: "wifi-joining" }
  | { kind: "wifi-failed"; boardError: string }
  | { kind: "linking" }
  | { kind: "linked"; deviceId: string }
  | { kind: "unreachable-server"; deviceId: string };

type Claim = { device_id: string; token: string; link_url: string };

export type SetupDeps = {
  port: PanelPort;
  claim: () => Promise<Claim>;
  isLinked: (deviceId: string) => Promise<boolean>;
  utcOffsetMinutes: () => number;
  sleep: (ms: number) => Promise<void>;
  now: () => number;
};

const INCOMPATIBLE_MESSAGE = "This panel needs a firmware update first";
const CONNECT_DEADLINE_MS = 10_000;
const STATUS_INTERVAL_MS = 500;
const REOPEN_INTERVAL_MS = 1_000;
const LINK_DEADLINE_MS = 60_000;
/** A board that answers "connecting" for this long is not going to join. */
const WIFI_JOIN_DEADLINE_MS = 45_000;
export const WIFI_JOIN_TIMEOUT_MESSAGE =
  "The panel is still trying to join this network. Check the network name and password.";
const LINK_INTERVAL_MS = 1_000;
const MAX_REOPEN_ATTEMPTS = 3;

export class PanelSetup {
  private link: PanelLink | undefined;
  private claimResult: Claim | undefined;
  private disconnected = false;

  constructor(
    private readonly deps: SetupDeps,
    private readonly onStep: (step: SetupStep) => void,
  ) {
    deps.port.onDisconnect(() => {
      this.disconnected = true;
    });
  }

  async connect(): Promise<void> {
    this.onStep({ kind: "connecting" });
    try {
      await this.deps.port.open();
      this.disconnected = false;
      this.link = new PanelLink(this.deps.port);
    } catch {
      this.onStep({ kind: "no-response" });
      return;
    }

    const deadline = this.deps.now() + CONNECT_DEADLINE_MS;
    while (this.deps.now() < deadline) {
      try {
        const status = await this.link.status();
        if (
          status.protocolVersion !== PROTOCOL_VERSION ||
          (status.capabilities & CAPABILITY_NETWORKING) === 0n
        ) {
          this.onStep({ kind: "incompatible", message: INCOMPATIBLE_MESSAGE });
          return;
        }
        this.onStep({ kind: "wifi-form" });
        return;
      } catch (error) {
        if (error instanceof PanelDeviceError && error.deviceError.code === 3) {
          this.onStep({ kind: "incompatible", message: INCOMPATIBLE_MESSAGE });
          return;
        }
      }
      await this.sleepUntil(deadline, STATUS_INTERVAL_MS);
    }
    this.onStep({ kind: "no-response" });
  }

  async submitWifi(ssid: string, password: string): Promise<void> {
    if (this.link === undefined) {
      this.onStep({ kind: "no-response" });
      return;
    }

    this.onStep({ kind: "writing" });
    this.claimResult ??= await this.deps.claim();
    const claim = this.claimResult;
    const config: NetworkConfig = {
      ssid,
      psk: password,
      serverUrl: claim.link_url,
      deviceId: claim.device_id,
      token: claim.token,
      utcOffsetMinutes: this.deps.utcOffsetMinutes(),
      tier: 1,
    };
    let reopenAttempts = 0;
    const reopen = async (): Promise<boolean> => {
      while (reopenAttempts < MAX_REOPEN_ATTEMPTS) {
        reopenAttempts += 1;
        await this.deps.sleep(REOPEN_INTERVAL_MS);
        try {
          await this.deps.port.open();
          this.disconnected = false;
          this.link = new PanelLink(this.deps.port);
          return true;
        } catch {
          // A resetting board can leave the selected port unavailable for a moment.
        }
      }
      return false;
    };

    while (true) {
      try {
        await this.link.networkConfig(config);
        break;
      } catch (error) {
        if (!this.disconnected && !(error instanceof PanelDisconnectedError)) {
          this.onStep({
            kind: "wifi-failed",
            boardError: error instanceof Error ? error.message : "The panel refused its settings.",
          });
          return;
        }
        if (!(await reopen())) {
          this.onStep({ kind: "no-response" });
          return;
        }
      }
    }

    if (this.disconnected) {
      if (!(await reopen())) {
        this.onStep({ kind: "no-response" });
        return;
      }
    }

    this.onStep({ kind: "wifi-joining" });
    let lastResponseAt = this.deps.now();
    const joinDeadline = this.deps.now() + WIFI_JOIN_DEADLINE_MS;
    while (true) {
      if (this.deps.now() >= joinDeadline) {
        this.onStep({ kind: "wifi-failed", boardError: WIFI_JOIN_TIMEOUT_MESSAGE });
        return;
      }
      if (this.disconnected) {
        if (!(await reopen())) {
          this.onStep({ kind: "no-response" });
          return;
        }
      }

      try {
        const status = await this.link.status();
        lastResponseAt = this.deps.now();
        if (status.wifiState === 3) {
          this.onStep({
            kind: "wifi-failed",
            boardError: status.lastNetworkError ?? "The panel could not join this Wi-Fi network.",
          });
          return;
        }
        if (status.wifiState === 2) break;
      } catch (error) {
        if (this.disconnected || error instanceof PanelDisconnectedError) {
          if (!(await reopen())) {
            this.onStep({ kind: "no-response" });
            return;
          }
        } else if (error instanceof PanelDeviceError) {
          this.onStep({ kind: "wifi-failed", boardError: error.message });
          return;
        }
        if (this.deps.now() - lastResponseAt >= CONNECT_DEADLINE_MS) {
          this.onStep({ kind: "no-response" });
          return;
        }
      }
      await this.deps.sleep(STATUS_INTERVAL_MS);
    }

    this.onStep({ kind: "linking" });
    const linkDeadline = this.deps.now() + LINK_DEADLINE_MS;
    while (this.deps.now() < linkDeadline) {
      let linked = false;
      try {
        linked = await this.deps.isLinked(claim.device_id);
      } catch {
        // A transient server request failure consumes one poll, not the whole setup attempt.
      }
      if (linked) {
        this.onStep({ kind: "linked", deviceId: claim.device_id });
        return;
      }
      await this.sleepUntil(linkDeadline, LINK_INTERVAL_MS);
    }
    this.onStep({ kind: "unreachable-server", deviceId: claim.device_id });
  }

  private async sleepUntil(deadline: number, interval: number): Promise<void> {
    const remaining = deadline - this.deps.now();
    if (remaining > 0) await this.deps.sleep(Math.min(interval, remaining));
  }
}
