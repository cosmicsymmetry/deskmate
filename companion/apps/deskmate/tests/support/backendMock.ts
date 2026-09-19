import { mock } from "bun:test";
import * as backendModule from "../../src/lib/backend";
import type {
  AppConfig,
  AppSnapshot,
  ConfigApplyResult,
  DraftValidation,
  FaceDescriptor,
  ImageSourceDescriptor,
  NetworkSettings,
  PreviewFrame,
} from "../../src/lib/types";

import { snapshot } from "./fixtures";

// Bun mocks replace a process-global namespace, so capture the real wrapper first.
export const realMintImageSource = backendModule.mintImageSource;
export const realListenToAppState = backendModule.listenToAppState;
export const realSignInAndSelectDevice = backendModule.signInAndSelectDevice;
export const realSignIn = backendModule.signIn;
export { backendModule };

function defaults() {
  const previewImpl: (cardId: string) => Promise<PreviewFrame> = () =>
    Promise.reject(new Error("renderCardPreview not configured for this test"));
  const listenImpl: (onSnapshot: (next: AppSnapshot) => void) => Promise<() => void> =
    async () => () => {};
  const snapshotImpl: () => Promise<AppSnapshot> = async () => snapshot;
  const validateImpl: (config: AppConfig) => Promise<DraftValidation> = async () => ({
    valid: true,
    issues: [],
  });
  const saveConfigImpl: (config: AppConfig) => Promise<ConfigApplyResult> = async () => ({
    save: { generation: 1, warning: null },
  });
  const networkSettingsImpl: () => Promise<NetworkSettings> = async () => ({
    server_url: "https://desk.example",
    device_id: "desk-1",
    tier: "networked",
  });
  const signInAndSelectDeviceImpl: (
    deviceId: string,
    adminToken: string,
  ) => Promise<NetworkSettings> = async (deviceId) => ({
    server_url: "https://desk.example",
    device_id: deviceId,
    tier: "networked",
  });
  const imageSourcesImpl: () => Promise<ImageSourceDescriptor[]> = async () => [];
  const updateImageSourceFaceImpl: (
    sourceId: string,
    fields: Record<string, string>,
  ) => Promise<FaceDescriptor> = async () => {
    throw new Error("updateImageSourceFace not configured for this test");
  };

  return {
    previewImpl,
    snapshotImpl,
    listenImpl,
    validateImpl,
    saveConfigImpl,
    networkSettingsImpl,
    signInAndSelectDeviceImpl,
    imageSourcesImpl,
    updateImageSourceFaceImpl,
  };
}

export const backendMocks = defaults();
export function resetBackendMocks() {
  Object.assign(backendMocks, defaults());
}

mock.module("../../src/lib/backend", () => ({
  ...backendModule,
  renderCardPreview: (cardId: string) => backendMocks.previewImpl(cardId),
  getAppSnapshot: () => backendMocks.snapshotImpl(),
  listenToAppState: (onSnapshot: (next: AppSnapshot) => void) =>
    backendMocks.listenImpl(onSnapshot),
  validateConfigDraft: (config: AppConfig) => backendMocks.validateImpl(config),
  saveConfig: (config: AppConfig) => backendMocks.saveConfigImpl(config),
  getNetworkSettings: () => backendMocks.networkSettingsImpl(),
  signInAndSelectDevice: (deviceId: string, adminToken: string) =>
    backendMocks.signInAndSelectDeviceImpl(deviceId, adminToken),
  listImageSources: () => backendMocks.imageSourcesImpl(),
  updateImageSourceFace: (sourceId: string, fields: Record<string, string>) =>
    backendMocks.updateImageSourceFaceImpl(sourceId, fields),
}));
