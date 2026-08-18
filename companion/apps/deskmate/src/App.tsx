import { useEffect, useState } from "react";

import { CardEditor } from "./components/CardEditor";
import { CardList } from "./components/CardList";
import { DeviceHeader } from "./components/DeviceHeader";
import { DevicePreview } from "./components/DevicePreview";
import { Filmstrip } from "./components/Filmstrip";
import { NetworkPanel } from "./components/NetworkPanel";
import { PlaylistPanel } from "./components/PlaylistPanel";
import { ProviderStatus } from "./components/ProviderStatus";
import {
  addCard,
  cardName,
  copyConfig,
  firstRunSteps,
  firstSelectableCard,
  issuesForCard,
  removeCard,
  unclaimedIssues,
  updateWidget,
} from "./lib/configDraft";
import {
  chooseIcsFile,
  controlPomodoro,
  getAutostartStatus,
  refreshProvider,
  setAutostartEnabled,
  setPushingPaused,
  toIpcError,
  validateConfigDraft,
} from "./lib/tauri";
import type {
  AppConfig,
  CardKind,
  CardSettings,
  DisplayOrientation,
  DraftValidation,
  IpcError,
  PomodoroAction,
} from "./lib/types";
import { useAppState } from "./lib/useAppState";

type ValidationState =
  | { kind: "idle"; result: DraftValidation }
  | { kind: "checking"; result: DraftValidation }
  | { kind: "ready"; result: DraftValidation }
  | { kind: "error"; result: DraftValidation; error: IpcError };

type SaveState =
  | { kind: "idle" }
  | { kind: "saving" }
  | { kind: "saved"; message: string }
  | { kind: "error"; error: IpcError };

const validDraft: DraftValidation = { valid: true, issues: [] };

export function App() {
  const {
    snapshot,
    loading,
    error: stateError,
    refresh,
    dataGeneration,
    networkSettings,
    ownershipTier,
    saveConfig,
    saveServerAccess,
    pairDevice,
    unpairDevice,
    factoryReset,
  } = useAppState();
  const [draft, setDraft] = useState<AppConfig | null>(null);
  const [selectedCardId, setSelectedCardId] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);
  const [validation, setValidation] = useState<ValidationState>({
    kind: "idle",
    result: validDraft,
  });
  const [saveState, setSaveState] = useState<SaveState>({ kind: "idle" });
  const [commandError, setCommandError] = useState<IpcError | null>(null);
  const [busyAction, setBusyAction] = useState<string | null>(null);
  const [refreshingProviderId, setRefreshingProviderId] = useState<string | null>(null);
  const [autostartEnabled, setAutostartValue] = useState(false);
  const [autostartMismatch, setAutostartMismatch] = useState(false);

  useEffect(() => {
    if (!snapshot || dirty) {
      return;
    }
    const next = copyConfig(snapshot.config);
    setDraft((current) =>
      current && JSON.stringify(current) === JSON.stringify(next) ? current : next,
    );
    setSelectedCardId((current) =>
      current && next.cards.some((card) => card.id === current)
        ? current
        : firstSelectableCard(next),
    );
  }, [dirty, snapshot]);

  useEffect(() => {
    if (!draft) {
      return;
    }
    let active = true;
    const timeout = window.setTimeout(() => {
      setValidation((current) => ({ kind: "checking", result: current.result }));
      void validateConfigDraft(draft)
        .then((result) => {
          if (active) {
            setValidation({ kind: "ready", result });
          }
        })
        .catch((nextError) => {
          if (active) {
            setValidation({
              kind: "error",
              result: { valid: false, issues: [] },
              error: toIpcError(nextError),
            });
          }
        });
    }, 180);
    return () => {
      active = false;
      window.clearTimeout(timeout);
    };
  }, [draft]);

  useEffect(() => {
    let active = true;
    void getAutostartStatus()
      .then((status) => {
        if (active) {
          setAutostartValue(status.enabled);
          setAutostartMismatch(status.enabled !== status.preference_enabled);
        }
      })
      .catch((nextError) => {
        if (active) {
          setCommandError(toIpcError(nextError));
        }
      });
    return () => {
      active = false;
    };
  }, []);

  if (loading && !snapshot) {
    return (
      <main className="startup-state" aria-busy="true">
        <span className="startup-mark" aria-hidden="true">
          D
        </span>
        <p className="eyebrow">Deskmate</p>
        <h1>Opening your display settings…</h1>
        <p>The background service keeps running if this window is closed.</p>
      </main>
    );
  }

  if (!snapshot || !draft) {
    return (
      <main className="startup-state">
        <span className="startup-mark startup-mark--error" aria-hidden="true">
          !
        </span>
        <p className="eyebrow">Deskmate</p>
        <h1>Settings could not be loaded</h1>
        <p role="alert">{stateError?.message ?? "The background service is unavailable."}</p>
        <button className="button button--primary" type="button" onClick={() => void refresh()}>
          Try again
        </button>
      </main>
    );
  }

  const selectedWidget = draft.cards.find((card) => card.id === selectedCardId) ?? null;
  const pomodoro =
    snapshot.pomodoros.find((candidate) => candidate.widget_id === selectedCardId) ?? null;
  const issues = validation.result.issues;
  const cardIssues = selectedCardId ? issuesForCard(issues, draft, selectedCardId) : [];
  // Issues no card-, playlist-, or preference-scoped surface below claims — e.g. a
  // `device.capabilities` issue naming a card the connected display can't render.
  // Rendered as its own banner so an unclaimed issue is explained somewhere rather than
  // just blocking Save with no highlighted control anywhere in the UI (see
  // `unclaimedIssues`).
  const leftoverIssues = unclaimedIssues(issues, draft);
  const networkedTier = ownershipTier === "networked";
  const localTier = ownershipTier === "local";

  const replaceDraft = (next: AppConfig) => {
    setDraft(next);
    setDirty(true);
    setSaveState({ kind: "idle" });
  };
  const handleAdd = (kind: CardKind) => {
    const result = addCard(draft, kind);
    replaceDraft(result.config);
    setSelectedCardId(result.cardId);
  };
  const handleWidgetChange = (widget: CardSettings) => {
    if (!selectedCardId) {
      return;
    }
    replaceDraft(updateWidget(draft, selectedCardId, widget));
  };
  const handleRemoveCard = (cardId: string) => {
    const next = removeCard(draft, cardId);
    replaceDraft(next);
    setSelectedCardId((current) => (current === cardId ? firstSelectableCard(next) : current));
  };
  const handleRemove = () => {
    if (!selectedCardId) {
      return;
    }
    handleRemoveCard(selectedCardId);
  };
  const handleChooseCalendarFile = () => {
    if (selectedWidget?.kind !== "calendar") {
      return;
    }
    setBusyAction("calendar-file");
    setCommandError(null);
    void chooseIcsFile()
      .then((path) => {
        if (path) {
          handleWidgetChange({
            ...selectedWidget,
            source: { kind: "file", value: path },
          });
        }
      })
      .catch((nextError) => setCommandError(toIpcError(nextError)))
      .finally(() => setBusyAction(null));
  };
  const runAction = async (name: string, operation: () => Promise<void>) => {
    setBusyAction(name);
    setCommandError(null);
    try {
      await operation();
      await refresh();
    } catch (nextError) {
      setCommandError(toIpcError(nextError));
    } finally {
      setBusyAction(null);
    }
  };
  const handleSave = async () => {
    if (validation.kind !== "ready" || !validation.result.valid) {
      return;
    }
    setSaveState({ kind: "saving" });
    setCommandError(null);
    try {
      const result = await saveConfig(draft);
      setDirty(false);
      setSaveState({
        kind: "saved",
        message: result.save.warning
          ? `Saved. ${result.save.warning.message}`
          : networkedTier
            ? "Saved to the server. The server will update your display."
            : snapshot.device.connection.kind === "online"
              ? "Saved and applied to your display."
              : "Saved. It will sync when your display reconnects.",
      });
      await refresh();
    } catch (nextError) {
      const ipcError = toIpcError(nextError);
      if (localTier && ipcError.category === "device") {
        setDirty(false);
        setSaveState({
          kind: "saved",
          message: "Saved. The display could not update yet, so this is queued for reconnect.",
        });
        await refresh();
      } else {
        setSaveState({ kind: "error", error: ipcError });
      }
    }
  };
  const handleTimerAction = (action: "start" | "pause" | "reset") => {
    if (!selectedCardId) {
      return;
    }
    void runAction("timer", () => controlPomodoro(selectedCardId, action satisfies PomodoroAction));
  };
  const handleProviderRefresh = (widgetId: string) => {
    setRefreshingProviderId(widgetId);
    setCommandError(null);
    void refreshProvider(widgetId)
      .then(() => refresh())
      .catch((nextError) => setCommandError(toIpcError(nextError)))
      .finally(() => setRefreshingProviderId(null));
  };
  const handleAutostart = (enabled: boolean) => {
    setBusyAction("autostart");
    setCommandError(null);
    void setAutostartEnabled(enabled)
      .then((status) => {
        setAutostartValue(status.enabled);
        setAutostartMismatch(status.enabled !== status.preference_enabled);
        return refresh();
      })
      .catch((nextError) => setCommandError(toIpcError(nextError)))
      .finally(() => setBusyAction(null));
  };

  const persistenceError =
    snapshot.persistence.kind === "recoverable-error" ? snapshot.persistence.message : null;
  const persistenceValidation =
    snapshot.persistence.kind === "validation-failed" ? snapshot.persistence : null;

  return (
    <div className="app-shell">
      <DeviceHeader
        snapshot={snapshot}
        commandError={commandError ?? stateError}
        busyAction={busyAction}
        onTogglePause={() =>
          void runAction("pause", () => setPushingPaused(!snapshot.config.preferences.paused))
        }
      />

      {persistenceValidation && (
        <aside className="recovery-banner" role="alert">
          <span aria-hidden="true">!</span>
          <div>
            <strong>{persistenceValidation.message}</strong>
            <ul>
              {persistenceValidation.issues.map((issue) => (
                <li key={`${issue.path}:${issue.code}`}>{issue.message}</li>
              ))}
            </ul>
            <p>The saved file was left untouched. Review the issues before saving again.</p>
          </div>
        </aside>
      )}

      {(persistenceError || autostartMismatch) && (
        <aside className="recovery-banner" role="status">
          <span aria-hidden="true">↺</span>
          <div>
            <strong>
              {persistenceError ? "Settings file needs attention" : "Start-at-login differs"}
            </strong>
            <p>
              {persistenceError
                ? `${persistenceError}. The unreadable file was left untouched; review the settings shown here before saving a fresh valid configuration.`
                : "The operating-system setting and saved preference differ. Choose your preference below to reconcile them."}
            </p>
          </div>
        </aside>
      )}

      {/* A push the display understood and refused is card-scoped and actionable:
          name the card and say what it refused, rather than parking the whole app in
          an error state over one card's data. */}
      {snapshot.card_errors.length > 0 && (
        <aside className="recovery-banner" role="status">
          <span aria-hidden="true">!</span>
          <div>
            <strong>
              {snapshot.card_errors.length === 1
                ? "The display refused one card's data"
                : `The display refused ${snapshot.card_errors.length} cards' data`}
            </strong>
            {snapshot.card_errors.map((cardError) => {
              const card = draft.cards.find((candidate) => candidate.id === cardError.card_id);
              return (
                <p key={cardError.card_id}>
                  <strong>{card ? cardName(card) : cardError.card_id}</strong> — {cardError.message}
                </p>
              );
            })}
            <p>
              Everything else kept updating. Adjust the card below and save to send its data again.
            </p>
          </div>
        </aside>
      )}

      {/* A validation issue whose path no card-, playlist-, or preference-scoped
          surface below claims (see `unclaimedIssues`) — e.g. `device.capabilities`,
          emitted when the connected display lacks a feature the draft needs. Without
          this, such an issue still disabled Save but was never shown anywhere,
          which is strictly worse than not validating it at all. */}
      {leftoverIssues.length > 0 && (
        <aside className="recovery-banner" role="status">
          <span aria-hidden="true">!</span>
          <div>
            <strong>
              {leftoverIssues.length === 1
                ? "One more thing needs attention"
                : `${leftoverIssues.length} more things need attention`}
            </strong>
            {leftoverIssues.map((issue) => (
              <p key={`${issue.path}:${issue.code}`}>{issue.message}</p>
            ))}
          </div>
        </aside>
      )}

      {(() => {
        const steps = firstRunSteps(draft, snapshot.has_saved_config);
        if (steps.every((step) => step.done)) {
          return null;
        }
        return (
          <aside className="first-run" aria-labelledby="first-run-heading">
            <div>
              <p className="eyebrow">A quick first setup</p>
              <h2 id="first-run-heading">Make the display yours</h2>
            </div>
            <ol>
              {steps.map((step, index) => (
                <li key={step.label} className={step.done ? "is-done" : ""}>
                  <span className="numeral">{index + 1}</span> {step.label}
                </li>
              ))}
            </ol>
          </aside>
        );
      })()}

      <NetworkPanel
        device={{
          tier: ownershipTier,
          wifiState: snapshot.device.wifi_state,
          wifiRssi: snapshot.device.wifi_rssi,
          ip: snapshot.device.ip ?? "",
          lastNetworkError: snapshot.device.last_network_error,
          otaState: snapshot.device.ota_state,
        }}
        settings={{
          serverUrl: networkSettings.server_url,
          deviceId: networkSettings.device_id,
          ssid: "",
        }}
        onPair={async (input) => {
          await pairDevice(input);
          await refresh();
        }}
        onUnpair={async () => {
          await unpairDevice();
          await refresh();
        }}
        onFactoryReset={async () => {
          await factoryReset();
          await refresh();
        }}
        onSaveServerAccess={saveServerAccess}
      />

      <div className="workspace">
        <div className="workspace__editors">
          <div className="library-playlists">
            <CardList
              config={draft}
              issues={issues}
              selectedCardId={selectedCardId}
              onSelect={setSelectedCardId}
              onAdd={handleAdd}
              onRemove={handleRemoveCard}
            />
            <PlaylistPanel
              config={draft}
              issues={issues}
              onChange={replaceDraft}
              onSelectCard={setSelectedCardId}
            />
          </div>
          <CardEditor
            card={selectedWidget}
            issues={cardIssues}
            pomodoro={pomodoro}
            timerBusy={busyAction === "timer"}
            filePickerBusy={busyAction === "calendar-file"}
            onChange={handleWidgetChange}
            onRemove={handleRemove}
            onTimerAction={handleTimerAction}
            onChooseCalendarFile={handleChooseCalendarFile}
          />
        </div>

        <aside className="workspace__preview">
          <DevicePreview
            cards={draft.cards}
            selectedWidgetId={selectedCardId}
            orientation={draft.preferences.orientation}
            dataGeneration={dataGeneration}
          />
          <Filmstrip
            config={draft}
            selectedCardId={selectedCardId}
            onSelect={setSelectedCardId}
            onReorder={(next) => replaceDraft(next)}
          />
          <ProviderStatus
            providers={snapshot.providers}
            cards={draft.cards}
            refreshingId={refreshingProviderId}
            onRefresh={handleProviderRefresh}
          />
          <section className="preferences-panel" aria-labelledby="preferences-heading">
            <div className="panel-heading panel-heading--compact">
              <div>
                <p className="step-label">App preferences</p>
                <h2 id="preferences-heading">Keep it current</h2>
              </div>
            </div>
            <label className="field">
              <span>Display timezone</span>
              <input
                value={draft.preferences.timezone}
                list="common-timezones"
                onChange={(event) =>
                  replaceDraft({
                    ...draft,
                    preferences: { ...draft.preferences, timezone: event.currentTarget.value },
                  })
                }
                aria-invalid={issues.some((issue) => issue.path === "preferences.timezone")}
              />
              <datalist id="common-timezones">
                <option value="UTC" />
                <option value="Asia/Tbilisi" />
                <option value="Europe/London" />
                <option value="Europe/Paris" />
                <option value="America/New_York" />
                <option value="America/Los_Angeles" />
                <option value="Asia/Tokyo" />
              </datalist>
              {issues
                .filter((issue) => issue.path === "preferences.timezone")
                .map((issue) => (
                  <small className="field-error" role="alert" key={issue.code}>
                    {issue.message}
                  </small>
                ))}
            </label>
            <label className="field">
              <span>Display mounting</span>
              <select
                value={draft.preferences.orientation}
                onChange={(event) =>
                  replaceDraft({
                    ...draft,
                    preferences: {
                      ...draft.preferences,
                      orientation: event.currentTarget.value as DisplayOrientation,
                    },
                  })
                }
              >
                <option value="landscape">Landscape · USB cable below</option>
                <option value="landscape-flipped">Landscape flipped · USB cable above</option>
              </select>
              <small>Saved with your layout and reapplied whenever the display reconnects.</small>
            </label>
            <label className="check-field">
              <input
                type="checkbox"
                checked={autostartEnabled}
                disabled={busyAction === "autostart"}
                onChange={(event) => handleAutostart(event.currentTarget.checked)}
              />
              <span>
                <strong>Start Deskmate when I sign in</strong>
                <small>Disabled until you opt in.</small>
              </span>
            </label>
          </section>
        </aside>
      </div>

      <footer className="save-bar">
        <div aria-live="polite">
          {validation.kind === "checking" && <span>Checking settings…</span>}
          {validation.kind === "ready" && !validation.result.valid && (
            <span className="save-error">
              Fix {validation.result.issues.length} highlighted issue
              {validation.result.issues.length === 1 ? "" : "s"} before saving.
            </span>
          )}
          {validation.kind === "error" && (
            <span className="save-error">Validation unavailable: {validation.error.message}</span>
          )}
          {saveState.kind === "saved" && (
            <span className="save-success">✓ {saveState.message}</span>
          )}
          {saveState.kind === "error" && (
            <div className="save-error" role="alert">
              <span>{saveState.error.message}</span>
              {saveState.error.category === "validation" && saveState.error.issues.length > 0 && (
                <ul className="save-error__issues">
                  {saveState.error.issues.map((issue) => (
                    <li key={`${issue.path}:${issue.code}:${issue.message}`}>{issue.message}</li>
                  ))}
                </ul>
              )}
            </div>
          )}
          {ownershipTier === null && saveState.kind === "idle" && (
            <span className="save-error">Connect over USB to confirm ownership before saving.</span>
          )}
          {validation.kind === "ready" &&
            validation.result.valid &&
            saveState.kind === "idle" &&
            ownershipTier !== null && (
              <span>{dirty ? "Unsaved changes" : "Everything is up to date"}</span>
            )}
        </div>
        <button
          className="button button--primary"
          type="button"
          disabled={
            !dirty ||
            ownershipTier === null ||
            validation.kind !== "ready" ||
            !validation.result.valid ||
            saveState.kind === "saving"
          }
          onClick={() => void handleSave()}
        >
          {saveState.kind === "saving"
            ? networkedTier
              ? "Saving to server…"
              : "Saving & applying…"
            : ownershipTier === null
              ? "Ownership unavailable"
              : networkedTier
                ? "Save to server"
                : "Save & apply"}
        </button>
      </footer>
    </div>
  );
}
