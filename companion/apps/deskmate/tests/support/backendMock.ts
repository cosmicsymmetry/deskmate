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
  const resumeImpl: () => Promise<void> = async () => {};
  const networkSettingsImpl: () => Promise<NetworkSettings> = async () => ({
    server_url: "https://desk.example",
    device_id: "desk-1",
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
    resumeImpl,
    networkSettingsImpl,
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
  resumePushing: () => backendMocks.resumeImpl(),
  getNetworkSettings: () => backendMocks.networkSettingsImpl(),
  listImageSources: () => backendMocks.imageSourcesImpl(),
  updateImageSourceFace: (sourceId: string, fields: Record<string, string>) =>
    backendMocks.updateImageSourceFaceImpl(sourceId, fields),
}));
