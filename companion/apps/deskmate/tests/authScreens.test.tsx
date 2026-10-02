import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import { act } from "react";

import type { Account, Instance } from "../src/lib/account";
import type { AppSnapshot } from "../src/lib/types";

import { backendMocks, backendModule, resetBackendMocks } from "./support/backendMock";
import { buttonWithText, installDomLifecycle, waitFor } from "./support/dom";
import { installHttpLifecycle } from "./support/http";

const ordinaryInstance: Instance = {
  setup_required: false,
  google_enabled: false,
  email_delivery: "email",
  signups_open: true,
  edition: "self-hosted",
};
const owner: Account = {
  id: "account-1",
  email: "owner@example.com",
  email_verified: false,
  is_instance_owner: true,
};

function accountDefaults() {
  return {
    getInstanceImpl: async (): Promise<Instance> => ordinaryInstance,
    completeSetupImpl: async (_code: string, _email: string): Promise<Account> => owner,
    requestSignInLinkImpl: async (_email: string): Promise<void> => {},
    consumeSignInLinkImpl: async (_token: string): Promise<Account> => owner,
    signOutImpl: async (): Promise<void> => {},
  };
}

const accountMocks = accountDefaults();

mock.module("../src/lib/account", () => ({
  getInstance: () => accountMocks.getInstanceImpl(),
  completeSetup: (code: string, email: string) => accountMocks.completeSetupImpl(code, email),
  requestSignInLink: (email: string) => accountMocks.requestSignInLinkImpl(email),
  consumeSignInLink: (token: string) => accountMocks.consumeSignInLinkImpl(token),
  googleSignInUrl: () => "/v1/app/auth/google/start",
  signOut: () => accountMocks.signOutImpl(),
}));

const { App } = await import("../src/App");

const { mount, cleanupMountedRoots } = installDomLifecycle();
installHttpLifecycle(cleanupMountedRoots);

let previousUrl = "";

beforeEach(() => {
  resetBackendMocks();
  Object.assign(accountMocks, accountDefaults());
  previousUrl = window.location.href;
  window.location.href = "http://localhost/";
});

afterEach(() => {
  window.location.href = previousUrl;
});

async function changeInput(input: HTMLInputElement | null, value: string) {
  expect(input).not.toBeNull();
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, value);
    input?.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

function requireMissingSession() {
  backendMocks.snapshotImpl = async (): Promise<AppSnapshot> => {
    throw new backendModule.DeskmateApiError({
      category: "runtime-unavailable",
      message: backendModule.SESSION_REQUIRED_MESSAGE,
    });
  };
}

test("instance discovery finishes before any account-scoped snapshot work starts", async () => {
  const pending = Promise.withResolvers<Instance>();
  let snapshotCalls = 0;
  accountMocks.getInstanceImpl = () => pending.promise;
  backendMocks.snapshotImpl = async () => {
    snapshotCalls += 1;
    return (await import("./support/fixtures")).snapshot;
  };

  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<App />));
    expect(container.textContent).toContain("Waking the display");
    expect(snapshotCalls).toBe(0);

    await act(async () => pending.resolve(ordinaryInstance));
    await waitFor(() => expect(snapshotCalls).toBeGreaterThan(0));
  } finally {
    await cleanup();
  }
});

test("setup submits the log code and email before loading the device window", async () => {
  const submissions: { code: string; email: string }[] = [];
  accountMocks.getInstanceImpl = async () => ({ ...ordinaryInstance, setup_required: true });
  accountMocks.completeSetupImpl = async (code, email) => {
    submissions.push({ code, email });
    return owner;
  };

  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<App />));
    await waitFor(() => expect(container.textContent).toContain("Set up this server"));
    expect(container.textContent).toContain("It's in the server's log.");

    await changeInput(container.querySelector('input[name="setup-code"]'), "ABCD-EFGH");
    await changeInput(container.querySelector('input[type="email"]'), "owner@example.com");
    await act(async () => buttonWithText(container, "Set up")?.click());

    await waitFor(() => expect(submissions).toEqual([{ code: "ABCD-EFGH", email: owner.email }]));
    await waitFor(() => expect(container.textContent).toContain("Settings"));
    expect(container.textContent).not.toContain("Admin token");
  } finally {
    await cleanup();
  }
});

test("the sign-in screen names the product and links back to its site", async () => {
  requireMissingSession();

  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<App />));
    await waitFor(() => expect(container.textContent).toContain("Sign in to Deskboy"));
    expect(container.textContent).not.toContain("Deskmate");

    const site = container.querySelector<HTMLAnchorElement>("a.startup__site");
    expect(site?.getAttribute("href")).toBe("https://deskboy.sh");
    expect(site?.textContent).toBe("deskboy.sh");
  } finally {
    await cleanup();
  }
});

test("email sign-in always leads to the same inbox screen", async () => {
  const requested: string[] = [];
  requireMissingSession();
  accountMocks.requestSignInLinkImpl = async (email) => {
    requested.push(email);
  };

  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<App />));
    await waitFor(() => expect(container.textContent).toContain("Sign in to Deskboy"));
    expect(container.textContent).not.toContain("Admin token");

    await changeInput(container.querySelector('input[type="email"]'), "anyone@example.com");
    await act(async () => buttonWithText(container, "Email me a sign-in link")?.click());

    await waitFor(() => expect(requested).toEqual(["anyone@example.com"]));
    expect(container.textContent).toContain("Check your inbox");
    expect(container.textContent).toContain(
      "If anyone@example.com can sign in, you'll receive a link shortly.",
    );
    expect(container.textContent).not.toContain("server's log");
    expect(container.textContent).not.toContain("account exists");
    expect(container.textContent).not.toContain("no account");
  } finally {
    await cleanup();
  }
});

test("Google sign-in appears only when the instance enables it", async () => {
  requireMissingSession();
  accountMocks.getInstanceImpl = async () => ({ ...ordinaryInstance, google_enabled: true });

  const first = await mount();
  try {
    await act(async () => first.root.render(<App />));
    await waitFor(() => expect(first.container.textContent).toContain("Sign in with Google"));
    expect(
      first.container.querySelector<HTMLAnchorElement>('a[href="/v1/app/auth/google/start"]'),
    ).not.toBeNull();
  } finally {
    await first.cleanup();
  }

  accountMocks.getInstanceImpl = async () => ordinaryInstance;
  const second = await mount();
  try {
    await act(async () => second.root.render(<App />));
    await waitFor(() => expect(second.container.textContent).toContain("Sign in to Deskboy"));
    expect(second.container.textContent).not.toContain("Sign in with Google");
  } finally {
    await second.cleanup();
  }
});

test("a closed-signups Google return explains why sign-in stopped", async () => {
  requireMissingSession();
  window.location.href = "http://localhost/?signin_error=signups-closed";

  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<App />));
    await waitFor(() =>
      expect(container.textContent).toContain("This server isn't taking new accounts."),
    );
  } finally {
    await cleanup();
  }
});

test("opening an email link does not consume it until Sign in is pressed", async () => {
  const tokens: string[] = [];
  window.location.href = "http://localhost/signin?token=abc";
  accountMocks.consumeSignInLinkImpl = async (token) => {
    tokens.push(token);
    throw new Error("This sign-in link has expired.");
  };

  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<App />));
    await waitFor(() => expect(container.textContent).toContain("Sign in to Deskboy"));
    expect(tokens).toEqual([]);

    await act(async () => buttonWithText(container, "Sign in")?.click());
    await waitFor(() => expect(tokens).toEqual(["abc"]));
    expect(container.textContent).toContain("This sign-in link has expired.");
    expect(buttonWithText(container, "Back to sign-in")).toBeDefined();
  } finally {
    await cleanup();
  }
});

test("a consumed email link leaves the token URL before loading the window", async () => {
  const tokens: string[] = [];
  window.location.href = "http://localhost/signin?token=working";
  accountMocks.consumeSignInLinkImpl = async (token) => {
    tokens.push(token);
    return owner;
  };

  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<App />));
    await waitFor(() => expect(buttonWithText(container, "Sign in")).toBeDefined());
    await act(async () => buttonWithText(container, "Sign in")?.click());

    await waitFor(() => expect(container.textContent).toContain("Settings"));
    expect(tokens).toEqual(["working"]);
    expect(window.location.pathname).toBe("/");
    expect(window.location.search).toBe("");
  } finally {
    await cleanup();
  }
});

test("email recovery supports retries, rate-limit errors, and address correction", async () => {
  requireMissingSession();
  const requested: string[] = [];
  accountMocks.requestSignInLinkImpl = async (email) => {
    requested.push(email);
    if (requested.length === 2) throw new Error("Try again in 60 seconds.");
  };
  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<App />));
    await waitFor(() => expect(buttonWithText(container, "Email me a sign-in link")).toBeDefined());
    await changeInput(container.querySelector('input[type="email"]'), "wrong@example.com");
    await act(async () => buttonWithText(container, "Email me a sign-in link")?.click());
    await act(async () => buttonWithText(container, "Request another link")?.click());
    expect(container.textContent).toContain("Try again in 60 seconds.");
    await act(async () => buttonWithText(container, "Request another link")?.click());
    expect(container.textContent).toContain("Another sign-in link was requested.");
    await act(async () => buttonWithText(container, "Use a different email")?.click());
    expect(container.querySelector<HTMLInputElement>('input[type="email"]')?.value).toBe(
      "wrong@example.com",
    );
    await changeInput(container.querySelector('input[type="email"]'), "right@example.com");
    await act(async () => buttonWithText(container, "Email me a sign-in link")?.click());
    expect(requested).toEqual([
      "wrong@example.com",
      "wrong@example.com",
      "wrong@example.com",
      "right@example.com",
    ]);
    expect(container.textContent).toContain("If right@example.com can sign in");
    expect(container.textContent).not.toContain("Try again in 60 seconds.");
  } finally {
    await cleanup();
  }
});

test("log delivery never claims to send an email", async () => {
  requireMissingSession();
  accountMocks.getInstanceImpl = async () => ({
    ...ordinaryInstance,
    email_delivery: "server-log",
  });
  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<App />));
    await waitFor(() => expect(container.textContent).toContain("Ask the server owner"));
    await changeInput(container.querySelector('input[type="email"]'), "owner@example.com");
    await act(async () => buttonWithText(container, "Get a sign-in link")?.click());
    expect(container.textContent).toContain("will appear in the server's log");
    expect(container.textContent).not.toContain("Check your inbox");
  } finally {
    await cleanup();
  }
});

test("a missing link token offers recovery without attempting sign-in", async () => {
  window.location.href = "http://localhost/signin";
  let attempts = 0;
  accountMocks.consumeSignInLinkImpl = async () => {
    attempts += 1;
    return owner;
  };
  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<App />));
    await waitFor(() => expect(container.textContent).toContain("missing its token"));
    expect(buttonWithText(container, "Back to sign-in")).toBeDefined();
    expect(attempts).toBe(0);
  } finally {
    await cleanup();
  }
});

test("session expiry closes the stream and ignores late snapshots", async () => {
  let publish: ((snapshot: AppSnapshot) => void) | undefined;
  let stops = 0;
  backendMocks.listenImpl = async (onSnapshot) => {
    publish = onSnapshot;
    return () => {
      stops += 1;
    };
  };
  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<App />));
    await waitFor(() => expect(container.textContent).toContain("Settings"));
    requireMissingSession();
    await act(async () => window.dispatchEvent(new Event("focus")));
    await waitFor(() => expect(container.textContent).toContain("Sign in to Deskboy"));
    expect(stops).toBe(1);
    const { snapshot } = await import("./support/fixtures");
    await act(async () => publish?.(snapshot));
    expect(container.textContent).toContain("Sign in to Deskboy");
    expect(container.textContent).not.toContain("Settings");
  } finally {
    await cleanup();
  }
});

test("an account without a panel can sign out", async () => {
  backendMocks.snapshotImpl = async () => {
    throw new backendModule.DeskmateApiError({
      category: "not-found",
      message: backendModule.NO_PANELS_MESSAGE,
    });
  };
  accountMocks.signOutImpl = async () => requireMissingSession();
  const { container, root, cleanup } = await mount();
  try {
    await act(async () => root.render(<App />));
    await waitFor(() => expect(buttonWithText(container, "Sign out")).toBeDefined());
    await act(async () => buttonWithText(container, "Sign out")?.click());
    await waitFor(() => expect(container.textContent).toContain("Sign in to Deskboy"));
  } finally {
    await cleanup();
  }
});
