import { useEffect, useState } from "react";

import { CardEditor } from "./components/CardEditor";
import { CardList } from "./components/CardList";
import { Icon } from "./components/Icon";
import { LoopRing } from "./components/LoopRing";
import { DevicePreview } from "./components/DevicePreview";
import { NetworkPanel, ownershipLabel } from "./components/NetworkPanel";
import { PlaylistPanel } from "./components/PlaylistPanel";
import { SettingsSheet } from "./components/SettingsSheet";
import { TopBar } from "./components/TopBar";
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
  AppSnapshot,
  CardKind,
  CardSettings,
  DisplayOrientation,
  DraftValidation,
  IpcError,
  PomodoroAction,
} from "./lib/types";
import { useAppState } from "./lib/useAppState";

/**
 * The USB link in one word, for the settings sheet. It used to be a permanent
 * complication in the window chrome; it is a pairing-and-troubleshooting fact, so
 * it sits with the rest of them now.
 */
function linkLabel(snapshot: AppSnapshot): string {
  switch (snapshot.device.connection.kind) {
    case "online":
      return snapshot.device.port_name ?? "Connected";
    case "connecting":
      return "Looking for the display";
    case "standalone":
      return "Standalone";
    case "disconnected":
      return snapshot.device.connection.reason ?? "Not connected";
  }
}

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
const MAX_RENDERED_SAVE_ISSUES = 5;

function SaveError({ error }: { error: IpcError }) {
  const issues = error.category === "validation" ? error.issues : [];
  const visibleIssues = issues.slice(0, MAX_RENDERED_SAVE_ISSUES);
  const hiddenIssueCount = issues.length - visibleIssues.length;
  return (
    <div className="save-error" role="alert">
      <span>{error.message}</span>
      {visibleIssues.length > 0 && (
        <ul className="save-error__issues">
          {visibleIssues.map((issue) => (
            <li key={`${issue.path}:${issue.code}:${issue.message}`}>{issue.message}</li>
          ))}
        </ul>
      )}
      {hiddenIssueCount > 0 && (
        <span className="save-error__more">
          and {hiddenIssueCount} more issue{hiddenIssueCount === 1 ? "" : "s"}
        </span>
      )}
    </div>
  );
}

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
    chooseLocalMode,
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
  const [settingsOpen, setSettingsOpen] = useState(false);

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
      <main className="startup" aria-busy="true">
        <span className="startup__ring" aria-hidden="true" />
        <h1>Waking the display…</h1>
        <p>The background service keeps running even if you close this window.</p>
      </main>
    );
  }

  if (!snapshot || !draft) {
    return (
      <main className="startup startup--error">
        <span className="startup__ring startup__ring--error" aria-hidden="true" />
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
  const selectedProvider =
    snapshot.providers.find((candidate) => candidate.widget_id === selectedCardId) ?? null;
  const protocolMismatch =
    snapshot.device.protocol_version !== null && snapshot.device.protocol_version !== 1;
  const paused = snapshot.config.preferences.paused || snapshot.runtime.kind === "paused";
  // The settings button is silent while everything is nominal, and carries a dot
  // only for the things whose answers live behind it.
  const needsAttention =
    ownershipTier === null ||
    protocolMismatch ||
    snapshot.device.wifi_state === "failed" ||
    snapshot.device.last_network_error !== null;

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
    <div className="app">
      <TopBar attention={needsAttention} onOpenSettings={() => setSettingsOpen(true)} />

      <div className="face">
        {/* The rail is the face: what the panel is showing, what the loop looks
            like, and whether the data behind it is fresh. It stays put while the
            work column scrolls, because every edit in that column is aimed at it. */}
        <aside className="face__rail">
          <DevicePreview
            cards={draft.cards}
            selectedWidgetId={selectedCardId}
            orientation={draft.preferences.orientation}
            dataGeneration={dataGeneration}
          />
          <LoopRing
            config={draft}
            selectedCardId={selectedCardId}
            onSelect={setSelectedCardId}
            onReorder={(next) => replaceDraft(next)}
          />
        </aside>

        <main className="face__work">
          {/* What the header alert used to carry. A sentence in the column you are
              already reading beats a permanent band that is blank 99% of the time. */}
          {(protocolMismatch ||
            snapshot.runtime.kind === "error" ||
            commandError ||
            stateError) && (
            <aside className="notice notice--bad" role="alert">
              <div>
                <strong>
                  {protocolMismatch
                    ? `This display speaks protocol ${snapshot.device.protocol_version}; this app speaks protocol 1.`
                    : snapshot.runtime.kind === "error"
                      ? snapshot.runtime.message
                      : (commandError ?? stateError)?.message}
                </strong>
              </div>
            </aside>
          )}

          {/* Nothing in this window can pause pushing any more — the control was
              removed as a knob nobody reached for. A config saved while it was still
              here can still arrive paused, so the way out has to stay reachable. */}
          {paused && (
            <aside className="notice notice--warn" role="status">
              <div>
                <strong>Sending to the display is paused</strong>
                <p>Nothing you change here reaches the panel until you resume.</p>
                <button
                  className="button button--quiet"
                  type="button"
                  disabled={busyAction === "pause"}
                  onClick={() => void runAction("pause", () => setPushingPaused(false))}
                >
                  {busyAction === "pause" ? "Resuming…" : "Resume sending"}
                </button>
              </div>
            </aside>
          )}

          {persistenceValidation && (
            <aside className="notice notice--bad" role="alert">
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
            <aside className="notice notice--warn" role="status">
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
            <aside className="notice notice--warn" role="status">
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
                      <strong>{card ? cardName(card) : cardError.card_id}</strong> —{" "}
                      {cardError.message}
                    </p>
                  );
                })}
                <p>
                  Everything else kept updating. Adjust the card below and save to send its data
                  again.
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
            <aside className="notice notice--bad" role="status">
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
            const done = steps.filter((step) => step.done).length;
            return (
              <aside className="first-run" aria-labelledby="first-run-heading">
                <div className="first-run__head">
                  <div>
                    <h2 id="first-run-heading">Make the display yours</h2>
                  </div>
                  <span className="first-run__count numeral">
                    {done}/{steps.length}
                  </span>
                </div>
                <ol className="first-run__steps">
                  {steps.map((step, index) => (
                    <li key={step.label} className={step.done ? "is-done" : ""}>
                      <span className="first-run__pip numeral">
                        {step.done ? <Icon name="check" /> : index + 1}
                      </span>
                      {step.label}
                    </li>
                  ))}
                </ol>
              </aside>
            );
          })()}

          <CardList
            config={draft}
            issues={issues}
            cardData={snapshot.card_data}
            pomodoros={snapshot.pomodoros}
            providers={snapshot.providers}
            selectedCardId={selectedCardId}
            onSelect={setSelectedCardId}
            onAdd={handleAdd}
            onRemove={handleRemoveCard}
          />

          <CardEditor
            card={selectedWidget}
            issues={cardIssues}
            pomodoro={pomodoro}
            provider={selectedProvider}
            timerBusy={busyAction === "timer"}
            filePickerBusy={busyAction === "calendar-file"}
            providerRefreshing={refreshingProviderId === selectedCardId}
            onChange={handleWidgetChange}
            onRemove={handleRemove}
            onTimerAction={handleTimerAction}
            onChooseCalendarFile={handleChooseCalendarFile}
            onRefreshProvider={() => selectedCardId && handleProviderRefresh(selectedCardId)}
          />

          <PlaylistPanel
            config={draft}
            issues={issues}
            onChange={replaceDraft}
            onSelectCard={setSelectedCardId}
          />

          <section className="panel preferences" aria-labelledby="preferences-heading">
            <div className="panel-heading">
              <div>
                <h2 id="preferences-heading">Preferences</h2>
              </div>
            </div>
            <div className="form-grid">
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
            </div>
          </section>
        </main>
      </div>

      {/* Ownership, pairing and the device's own network state: a first-run task and
          a troubleshooting task, and nothing you look at while arranging cards. It
          answers for itself here rather than taxing every session for the privilege. */}
      <SettingsSheet
        open={settingsOpen}
        title="Device"
        badge={
          <strong className={`ownership-badge ownership-badge--${ownershipTier ?? "unknown"}`}>
            {ownershipLabel(ownershipTier)}
          </strong>
        }
        onClose={() => setSettingsOpen(false)}
      >
        <NetworkPanel
          device={{
            tier: ownershipTier,
            link: linkLabel(snapshot),
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
          allowLocalOverride={
            snapshot.device.tier === null &&
            networkSettings.tier !== "local" &&
            Boolean(networkSettings.server_url || networkSettings.device_id)
          }
          onUseLocalMode={async () => {
            await chooseLocalMode();
            await refresh();
          }}
        />
      </SettingsSheet>

      <footer className="save-bar">
        <div className="save-bar__state" aria-live="polite">
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
            <span className="save-success">
              <Icon name="check" />
              {saveState.message}
            </span>
          )}
          {saveState.kind === "error" && <SaveError error={saveState.error} />}
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
