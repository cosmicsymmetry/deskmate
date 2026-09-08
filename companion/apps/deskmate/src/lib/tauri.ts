import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  AppConfig,
  AppSnapshot,
  AutostartStatus,
  ConfigApplyResult,
  DraftValidation,
  IpcError,
  NetworkSettings,
  PluginCatalog,
  PomodoroAction,
  PreviewFrame,
  ProvisionDeviceInput,
  ServerCardState,
} from "./types";

export const APP_STATE_EVENT = "app-state";
export const MAX_DRAFT_BYTES = 64 * 1024;

const IPC_ERROR_CATEGORIES = new Set<IpcError["category"]>([
  "invalid-payload",
  "payload-too-large",
  "validation",
  "persistence",
  "runtime-busy",
  "runtime-unavailable",
  "incompatible-server",
  "not-found",
  "device",
  "provider",
  "autostart",
  "window",
  "internal",
  "unsupported",
]);

export class DeskmateCommandError extends Error {
  readonly details: IpcError;

  constructor(details: IpcError) {
    super(details.message);
    this.name = "DeskmateCommandError";
    this.details = details;
  }
}

export function toIpcError(error: unknown): IpcError {
  if (
    typeof error === "object" &&
    error !== null &&
    "category" in error &&
    "message" in error &&
    typeof error.category === "string" &&
    IPC_ERROR_CATEGORIES.has(error.category as IpcError["category"]) &&
    typeof error.message === "string"
  ) {
    return error as IpcError;
  }
  if (error instanceof DeskmateCommandError) {
    return error.details;
  }
  return {
    category: "internal",
    message: error instanceof Error ? error.message : String(error),
  };
}

async function invokeTyped<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw new DeskmateCommandError(toIpcError(error));
  }
}

function draftPayload(config: AppConfig): { json: string } {
  let json: string;
  try {
    json = JSON.stringify(config);
  } catch (error) {
    throw new DeskmateCommandError({
      category: "invalid-payload",
      message: error instanceof Error ? error.message : "configuration cannot be serialized",
    });
  }
  if (new TextEncoder().encode(json).byteLength > MAX_DRAFT_BYTES) {
    throw new DeskmateCommandError({
      category: "payload-too-large",
      message: `Configuration exceeds the ${MAX_DRAFT_BYTES}-byte IPC limit.`,
      maximum_bytes: MAX_DRAFT_BYTES,
    });
  }
  return { json };
}

export function getAppSnapshot(): Promise<AppSnapshot> {
  return invokeTyped("get_app_snapshot");
}

export function validateConfigDraft(config: AppConfig): Promise<DraftValidation> {
  return invokeTyped("validate_config_draft", { draft: draftPayload(config) });
}

export function saveApplyConfig(config: AppConfig): Promise<ConfigApplyResult> {
  return invokeTyped("save_apply_config", { draft: draftPayload(config) });
}

export function saveServerConfig(config: AppConfig): Promise<ConfigApplyResult> {
  return invokeTyped("save_server_config", {
    request: {
      draft: draftPayload(config),
    },
  });
}

export function getNetworkSettings(): Promise<NetworkSettings> {
  return invokeTyped("get_network_settings");
}

export function setServerEndpoint(
  serverUrl: string,
  deviceId: string,
  adminToken: string,
): Promise<NetworkSettings> {
  return invokeTyped("set_server_endpoint", {
    request: { server_url: serverUrl, device_id: deviceId, admin_token: adminToken },
  });
}

export function provisionDevice(input: ProvisionDeviceInput): Promise<NetworkSettings> {
  return invokeTyped("provision_device", { request: input });
}

export function factoryResetDevice(): Promise<void> {
  return invokeTyped("factory_reset_device");
}

export function chooseLocalOwnership(): Promise<NetworkSettings> {
  return invokeTyped("use_local_ownership");
}

export function resumePushing(): Promise<void> {
  return invokeTyped("resume_pushing");
}

export function controlPomodoro(widgetId: string, action: PomodoroAction): Promise<void> {
  return invokeTyped("control_pomodoro", {
    target: { widget_id: widgetId },
    action,
  });
}

export function refreshProvider(widgetId: string): Promise<void> {
  return invokeTyped("refresh_provider", { target: { widget_id: widgetId } });
}

export function chooseIcsFile(): Promise<string | null> {
  return invokeTyped("choose_ics_file");
}

export function getAutostartStatus(): Promise<AutostartStatus> {
  return invokeTyped("get_autostart_status");
}

export function setAutostartEnabled(enabled: boolean): Promise<AutostartStatus> {
  return invokeTyped("set_autostart_enabled", { enabled });
}

export function renderCardPreview(cardId: string): Promise<PreviewFrame> {
  return invokeTyped("render_card_preview", { cardId });
}

export function getServerPlugins(): Promise<PluginCatalog> {
  return invokeTyped("get_server_plugins");
}

export function getServerCardState(): Promise<ServerCardState[]> {
  return invokeTyped("get_server_card_state");
}

export function listenToAppState(onSnapshot: (snapshot: AppSnapshot) => void): Promise<UnlistenFn> {
  return listen<AppSnapshot>(APP_STATE_EVENT, (event) => onSnapshot(event.payload));
}
