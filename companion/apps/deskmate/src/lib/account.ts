import { request } from "./backend";

export type Instance = {
  setup_required: boolean;
  google_enabled: boolean;
  signups_open: boolean;
  edition: "self-hosted" | "hosted";
};

export type Account = {
  id: string;
  email: string;
  email_verified: boolean;
  is_instance_owner: boolean;
};

export type PanelRow = {
  id: string;
  connected: boolean;
  has_saved_config: boolean;
  configured_at: number | null;
  state: "pending" | "active";
};

type AccountResponse = { account: Account };
type SignupsResponse = { signups_open: boolean };

export function getInstance(): Promise<Instance> {
  return request<Instance>("GET", "/v1/app/instance");
}

export async function completeSetup(code: string, email: string): Promise<Account> {
  const response = await request<AccountResponse>("POST", "/v1/app/setup", { code, email });
  return response.account;
}

export async function requestSignInLink(email: string): Promise<void> {
  await request<unknown>("POST", "/v1/app/auth/email", { email });
}

export async function consumeSignInLink(token: string): Promise<Account> {
  const response = await request<AccountResponse>("POST", "/v1/app/auth/link", { token });
  return response.account;
}

export function googleSignInUrl(): string {
  return "/v1/app/auth/google/start";
}

export function getAccount(): Promise<Account> {
  return request<Account>("GET", "/v1/app/account");
}

export function signOut(): Promise<void> {
  return request<void>("DELETE", "/v1/app/session");
}

export function signOutEverywhere(): Promise<void> {
  return request<void>("POST", "/v1/app/sessions/revoke-all");
}

export function deleteAccount(): Promise<void> {
  return request<void>("DELETE", "/v1/app/account");
}

export async function setSignupsOpen(open: boolean): Promise<boolean> {
  const response = await request<SignupsResponse>("PUT", "/v1/app/instance/signups", { open });
  return response.signups_open;
}

export function listPanels(): Promise<PanelRow[]> {
  return request<PanelRow[]>("GET", "/v1/app/devices");
}

export function claimPanel(): Promise<{ device_id: string; token: string; link_url: string }> {
  return request("POST", "/v1/app/devices/claim");
}

export function removePanel(id: string): Promise<void> {
  return request<void>("DELETE", `/v1/app/devices/${encodeURIComponent(id)}`);
}
