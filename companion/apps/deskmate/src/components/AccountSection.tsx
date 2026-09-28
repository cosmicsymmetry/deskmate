import { useEffect, useState } from "react";

import {
  type Account,
  type Instance,
  type PanelRow,
  deleteAccount,
  getAccount,
  listPanels,
  removePanel,
  setSignupsOpen,
  signOut,
  signOutEverywhere,
} from "../lib/account";
import { serialSupported as browserSerialSupported } from "../lib/serial/port";
import { PanelSetup } from "./PanelSetup";

export interface AccountSectionApi {
  getAccount(): Promise<Account>;
  signOut(): Promise<void>;
  signOutEverywhere(): Promise<void>;
  deleteAccount(): Promise<void>;
  setSignupsOpen(open: boolean): Promise<boolean>;
  listPanels(): Promise<PanelRow[]>;
  removePanel(id: string): Promise<void>;
}

const browserApi: AccountSectionApi = {
  getAccount,
  signOut,
  signOutEverywhere,
  deleteAccount,
  setSignupsOpen,
  listPanels,
  removePanel,
};

function panelStatus(panel: PanelRow): string {
  if (panel.state === "pending") return "Waiting for first connection";
  return panel.connected ? "Online" : "Offline";
}

export function AccountSection({
  instance,
  api = browserApi,
  serialSupported = browserSerialSupported,
  onSessionEnded,
  onSignupsChanged,
  onPanelsChanged,
  open,
}: {
  instance: Instance;
  api?: AccountSectionApi;
  serialSupported?: () => boolean;
  onSessionEnded: () => void;
  onSignupsChanged: (open: boolean) => void;
  onPanelsChanged: () => void;
  /** Whether the settings sheet is showing; each opening reloads the account and panels. */
  open: boolean;
}) {
  const [account, setAccount] = useState<Account | null>(null);
  const [panels, setPanels] = useState<PanelRow[]>([]);
  const [panelsLoaded, setPanelsLoaded] = useState(false);
  const [signupsOpen, setLocalSignupsOpen] = useState(instance.signups_open);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState<string | null>(null);
  const [showPanelSetup, setShowPanelSetup] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const loadPanels = async () => {
    const next = await api.listPanels();
    setPanels(next);
    setPanelsLoaded(true);
  };

  useEffect(() => {
    if (!open) return;
    let active = true;
    void Promise.all([api.getAccount(), api.listPanels()])
      .then(([nextAccount, nextPanels]) => {
        if (!active) return;
        setAccount(nextAccount);
        setPanels(nextPanels);
        setPanelsLoaded(true);
      })
      .catch((next) => {
        if (active) setError(next instanceof Error ? next.message : String(next));
      });
    return () => {
      active = false;
    };
  }, [api, open]);

  const run = async (name: string, action: () => Promise<void>) => {
    setBusy(name);
    setError(null);
    try {
      await action();
    } catch (next) {
      setError(next instanceof Error ? next.message : String(next));
    } finally {
      setBusy(null);
    }
  };

  return (
    <>
      <section className="sheet__section" aria-labelledby="account-heading">
        <h3 id="account-heading">Account</h3>
        {account ? (
          <>
            <p>{account.email}</p>
            {account.is_instance_owner && (
              <label className="check-field">
                <input
                  type="checkbox"
                  checked={signupsOpen}
                  disabled={busy !== null}
                  onChange={(event) => {
                    const requested = event.currentTarget.checked;
                    const previous = signupsOpen;
                    setLocalSignupsOpen(requested);
                    void run("signups", async () => {
                      try {
                        const saved = await api.setSignupsOpen(requested);
                        setLocalSignupsOpen(saved);
                        onSignupsChanged(saved);
                      } catch (next) {
                        setLocalSignupsOpen(previous);
                        throw next;
                      }
                    });
                  }}
                />
                <span>
                  <strong>Let new people sign up</strong>
                </span>
              </label>
            )}
            <div className="network-actions">
              <button
                className="button button--quiet"
                type="button"
                disabled={busy !== null}
                onClick={() =>
                  void run("sign-out", async () => {
                    await api.signOut();
                    onSessionEnded();
                  })
                }
              >
                Sign out
              </button>
              <button
                className="button button--quiet"
                type="button"
                disabled={busy !== null}
                onClick={() =>
                  void run("sign-out-everywhere", async () => {
                    await api.signOutEverywhere();
                    onSessionEnded();
                  })
                }
              >
                Sign out everywhere
              </button>
              {!confirmDelete && (
                <button
                  className="text-button text-button--danger"
                  type="button"
                  disabled={busy !== null}
                  onClick={() => setConfirmDelete(true)}
                >
                  Delete account
                </button>
              )}
            </div>
            {confirmDelete && (
              <div className="notice notice--bad">
                <div>
                  <strong>
                    This deletes your cards and releases your panels. It can't be undone.
                  </strong>
                  <div className="network-actions">
                    <button
                      className="text-button text-button--danger"
                      type="button"
                      disabled={busy !== null}
                      onClick={() =>
                        void run("delete-account", async () => {
                          await api.deleteAccount();
                          onSessionEnded();
                        })
                      }
                    >
                      Delete my account
                    </button>
                    <button
                      className="button button--quiet"
                      type="button"
                      disabled={busy !== null}
                      onClick={() => setConfirmDelete(false)}
                    >
                      Keep it
                    </button>
                  </div>
                </div>
              </div>
            )}
          </>
        ) : (
          <p aria-busy="true">Loading account…</p>
        )}
        {error && (
          <p className="save-error" role="alert">
            {error}
          </p>
        )}
      </section>

      <section
        className="sheet__section"
        aria-labelledby={showPanelSetup ? "panel-setup-heading" : "panels-heading"}
      >
        {showPanelSetup ? (
          <PanelSetup
            headingId="panel-setup-heading"
            onDone={() => {
              setShowPanelSetup(false);
              void run("load-panels", async () => {
                await loadPanels();
                onPanelsChanged();
              });
            }}
          />
        ) : (
          <>
            <h3 id="panels-heading">Panels</h3>
            {!panelsLoaded ? (
              <p aria-busy="true">Loading panels…</p>
            ) : panels.length === 0 ? (
              <p>No panels yet.</p>
            ) : (
              <ul className="form-grid">
                {panels.map((panel) => (
                  <li key={panel.id}>
                    {confirmRemove === panel.id ? (
                      <div className="notice notice--bad">
                        <div>
                          <strong>{`Remove ${panel.id} from this account?`}</strong>
                          <div className="network-actions">
                            <button
                              className="text-button text-button--danger"
                              type="button"
                              disabled={busy !== null}
                              onClick={() =>
                                void run(`remove-${panel.id}`, async () => {
                                  await api.removePanel(panel.id);
                                  setPanels((current) =>
                                    current.filter((candidate) => candidate.id !== panel.id),
                                  );
                                  setConfirmRemove(null);
                                  onPanelsChanged();
                                })
                              }
                            >
                              Remove panel
                            </button>
                            <button
                              className="button button--quiet"
                              type="button"
                              disabled={busy !== null}
                              onClick={() => setConfirmRemove(null)}
                            >
                              Keep it
                            </button>
                          </div>
                        </div>
                      </div>
                    ) : (
                      <div className="sheet__section-head">
                        <span>
                          <strong className="numeral">{panel.id}</strong>
                          <br />
                          <small>{panelStatus(panel)}</small>
                        </span>
                        <button
                          className="text-button text-button--danger"
                          type="button"
                          disabled={busy !== null}
                          onClick={() => setConfirmRemove(panel.id)}
                        >
                          Remove
                        </button>
                      </div>
                    )}
                  </li>
                ))}
              </ul>
            )}
            {serialSupported() ? (
              <div className="network-actions">
                <button
                  className="button button--secondary"
                  type="button"
                  onClick={() => setShowPanelSetup(true)}
                >
                  Add a panel
                </button>
              </div>
            ) : (
              <p>Setting up a panel needs Chrome or Edge on a computer.</p>
            )}
          </>
        )}
      </section>
    </>
  );
}
