import { useRef, useState } from "react";

import {
  type Account,
  type Instance,
  completeSetup,
  consumeSignInLink,
  googleSignInUrl,
  requestSignInLink,
} from "../lib/account";
import { PRODUCT_NAME, PRODUCT_SITE } from "../lib/product";

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function SignInError({ reason }: { reason: string | null }) {
  if (!reason) return null;
  const message =
    reason === "email-link-required"
      ? "Use an email link to sign in to this account. Google can't confirm ownership of this email address."
      : reason === "signups-closed"
        ? "This server isn't taking new accounts."
        : reason === "expired"
          ? "That sign-in took too long. Try again."
          : reason === "declined"
            ? "Google sign-in was canceled. Try again or use an email link."
            : reason === "email-unverified"
              ? "Google hasn't verified your email address. Use an email link to sign in."
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
  const [resent, setResent] = useState(false);
  const submitting = useRef(false);
  const logDelivery = instance.email_delivery === "server-log";

  const sendLink = async (address: string) => {
    if (submitting.current) return;
    submitting.current = true;
    setBusy(true);
    setError(null);
    setResent(false);
    try {
      await requestSignInLink(address);
      setResent(sentTo !== null);
      setSentTo(address);
    } catch (next) {
      setError(errorMessage(next));
    } finally {
      submitting.current = false;
      setBusy(false);
    }
  };

  if (sentTo) {
    return (
      <main className="startup">
        <h1>{logDelivery ? "Get your sign-in link" : "Check your inbox"}</h1>
        <p>
          {logDelivery
            ? `A sign-in link for ${sentTo} will appear in the server's log if this address can sign in.`
            : `If ${sentTo} can sign in, you'll receive a link shortly. Check your spam folder too.`}{" "}
          The link works for 15 minutes.
        </p>
        <div className="startup__form" aria-busy={busy}>
          <button
            className="button button--primary"
            type="button"
            disabled={busy}
            onClick={() => void sendLink(sentTo)}
          >
            {busy ? "Requesting…" : "Request another link"}
          </button>
          <button
            className="button button--quiet"
            type="button"
            disabled={busy}
            onClick={() => {
              setSentTo(null);
              setError(null);
              setResent(false);
            }}
          >
            Use a different email
          </button>
          {resent && <p role="status">Another sign-in link was requested.</p>}
          {error && (
            <p className="save-error" role="alert">
              {error}
            </p>
          )}
        </div>
      </main>
    );
  }

  return (
    <main className="startup">
      <h1>Sign in to {PRODUCT_NAME}</h1>
      <p>
        {logDelivery
          ? "This server puts sign-in links in its log. Ask the server owner for your link."
          : "Use a sign-in link. No password needed."}
      </p>
      <SignInError reason={signInError} />
      <form
        className="startup__form"
        aria-busy={busy}
        onSubmit={(event) => {
          event.preventDefault();
          void sendLink(email.trim());
        }}
      >
        <label className="field">
          <span>Email</span>
          <input
            type="email"
            name="email"
            required
            maxLength={254}
            disabled={busy}
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
          {busy ? "Requesting…" : logDelivery ? "Get a sign-in link" : "Email me a sign-in link"}
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
      <SiteLink />
    </main>
  );
}

/** The way back to the product's own site. A visitor who followed "Sign in"
 *  from there and has no account otherwise has nowhere to go but the back
 *  button. An anchor, not a paragraph: `.startup p:last-of-type` styles the
 *  screen's explanation, and a trailing <p> would take that role from it. */
function SiteLink() {
  return (
    <a className="startup__site" href={PRODUCT_SITE}>
      {new URL(PRODUCT_SITE).host}
    </a>
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
  const [error, setError] = useState<string | null>(
    token ? null : "This sign-in link is missing its token. Request a new link.",
  );

  return (
    <main className="startup">
      <h1>Sign in to {PRODUCT_NAME}</h1>
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
