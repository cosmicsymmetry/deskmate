import { useState } from "react";

import {
  type Account,
  type Instance,
  completeSetup,
  consumeSignInLink,
  googleSignInUrl,
  requestSignInLink,
} from "../lib/account";

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function SignInError({ reason }: { reason: string | null }) {
  if (!reason) return null;
  const message =
    reason === "signups-closed"
      ? "This server isn't taking new accounts."
      : reason === "expired"
        ? "That sign-in took too long. Try again."
        : "Google sign-in didn't work. Try again.";
  return (
    <p className="save-error" role="alert">
      {message}
    </p>
  );
}

export function SetupScreen({ onComplete }: { onComplete: (account: Account) => void }) {
  const [code, setCode] = useState("");
  const [email, setEmail] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  return (
    <main className="startup">
      <h1>Set up this server</h1>
      <form
        className="startup__form"
        onSubmit={(event) => {
          event.preventDefault();
          setBusy(true);
          setError(null);
          void completeSetup(code, email)
            .then(onComplete)
            .catch((next) => setError(errorMessage(next)))
            .finally(() => setBusy(false));
        }}
      >
        <label className="field">
          <span>Setup code</span>
          <input
            name="setup-code"
            value={code}
            autoComplete="one-time-code"
            autoCapitalize="characters"
            spellCheck={false}
            aria-invalid={error ? true : undefined}
            onChange={(event) => setCode(event.currentTarget.value)}
          />
          <small>It's in the server's log.</small>
        </label>
        <label className="field">
          <span>Email</span>
          <input
            type="email"
            value={email}
            autoComplete="email"
            aria-invalid={error ? true : undefined}
            onChange={(event) => setEmail(event.currentTarget.value)}
          />
        </label>
        <button
          className="button button--primary"
          type="submit"
          disabled={busy || code.trim() === "" || email.trim() === ""}
        >
          {busy ? "Setting up…" : "Set up"}
        </button>
        {error && (
          <span className="save-error" role="alert">
            {error}
          </span>
        )}
      </form>
    </main>
  );
}

export function CheckInbox({ email, edition }: { email: string; edition: Instance["edition"] }) {
  return (
    <main className="startup">
      <h1>Check your inbox</h1>
      <p>We sent a link to {email}. It works for 15 minutes.</p>
      {edition === "self-hosted" && (
        <p>No email set up on this server? The link is in the server's log.</p>
      )}
    </main>
  );
}

export function SignInScreen({
  instance,
  signInError,
}: {
  instance: Instance;
  signInError: string | null;
}) {
  const [email, setEmail] = useState("");
  const [sentTo, setSentTo] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (sentTo) return <CheckInbox email={sentTo} edition={instance.edition} />;

  return (
    <main className="startup">
      <h1>Sign in to Deskmate</h1>
      <SignInError reason={signInError} />
      <form
        className="startup__form"
        onSubmit={(event) => {
          event.preventDefault();
          setBusy(true);
          setError(null);
          const submittedEmail = email.trim();
          void requestSignInLink(submittedEmail)
            .then(() => setSentTo(submittedEmail))
            .catch((next) => setError(errorMessage(next)))
            .finally(() => setBusy(false));
        }}
      >
        <label className="field">
          <span>Email</span>
          <input
            type="email"
            value={email}
            autoComplete="email"
            aria-invalid={error ? true : undefined}
            onChange={(event) => setEmail(event.currentTarget.value)}
          />
        </label>
        <button
          className="button button--primary"
          type="submit"
          disabled={busy || email.trim() === ""}
        >
          {busy ? "Sending…" : "Email me a sign-in link"}
        </button>
        {instance.google_enabled && (
          <a className="button button--secondary" href={googleSignInUrl()}>
            Sign in with Google
          </a>
        )}
        {error && (
          <span className="save-error" role="alert">
            {error}
          </span>
        )}
      </form>
    </main>
  );
}

export function LinkLanding({
  token,
  onComplete,
  onBack,
}: {
  token: string;
  onComplete: (account: Account) => void;
  onBack: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  return (
    <main className="startup">
      <h1>Sign in to Deskmate</h1>
      {error ? (
        <>
          <p className="save-error" role="alert">
            {error}
          </p>
          <button className="button button--primary" type="button" onClick={onBack}>
            Back to sign-in
          </button>
        </>
      ) : (
        <button
          className="button button--primary"
          type="button"
          disabled={busy}
          onClick={() => {
            setBusy(true);
            void consumeSignInLink(token)
              .then((account) => {
                window.history.replaceState(null, "", "/");
                onComplete(account);
              })
              .catch((next) => setError(errorMessage(next)))
              .finally(() => setBusy(false));
          }}
        >
          {busy ? "Signing in…" : "Sign in"}
        </button>
      )}
    </main>
  );
}
