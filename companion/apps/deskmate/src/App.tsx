import { useEffect, useState } from "react";

import { CardEditor } from "./components/CardEditor";
import { CardList } from "./components/CardList";
import { DeviceHeader } from "./components/DeviceHeader";
import { DevicePreview } from "./components/DevicePreview";
import { Filmstrip } from "./components/Filmstrip";
import { ProviderStatus } from "./components/ProviderStatus";
import {
  addCard,
  copyConfig,
  firstRunSteps,
  firstSelectableCard,
  issuesForCard,
  removeCard,
  updateWidget,
} from "./lib/configDraft";
import {
  chooseIcsFile,
  controlPomodoro,
  getAutostartStatus,
  refreshProvider,
  saveApplyConfig,
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
  const { snapshot, loading, error: stateError, refresh } = useAppState();
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
  const defaultDwellSeconds =
    draft.carousel.advance.kind === "timed" ? draft.carousel.advance.default_dwell_seconds : null;

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
      const result = await saveApplyConfig(draft);
      setDirty(false);
      setSaveState({
        kind: "saved",
        message: result.save.warning
          ? `Saved. ${result.save.warning.message}`
          : snapshot.device.connection.kind === "online"
            ? "Saved and applied to your display."
            : "Saved. It will sync when your display reconnects.",
      });
      await refresh();
    } catch (nextError) {
      const ipcError = toIpcError(nextError);
      if (ipcError.category === "device") {
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

      {(persistenceError || autostartMismatch) && (
        <aside className="recovery-banner" role="status">
          <span aria-hidden="true">↺</span>
          <div>
            <strong>
              {persistenceError ? "Using your last working settings" : "Start-at-login differs"}
            </strong>
            <p>
              {persistenceError
                ? `${persistenceError}. The unreadable file was left untouched; saving will create a fresh valid configuration.`
                : "The operating-system setting and saved preference differ. Choose your preference below to reconcile them."}
            </p>
          </div>
        </aside>
      )}

      {(() => {
        const steps = firstRunSteps(draft, snapshot.device.connection.kind === "online");
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

      <div className="workspace">
        <div className="workspace__editors">
          <CardList
            config={draft}
            selectedCardId={selectedCardId}
            onSelect={setSelectedCardId}
            onAdd={handleAdd}
            onRemove={handleRemoveCard}
            onReorder={(next) => replaceDraft(next)}
          />
          <CardEditor
            card={selectedWidget}
            issues={cardIssues}
            pomodoro={pomodoro}
            defaultDwellSeconds={defaultDwellSeconds}
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
            cardData={snapshot.card_data}
            pomodoros={snapshot.pomodoros}
            orientation={draft.preferences.orientation}
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
            <span className="save-error" role="alert">
              {saveState.error.message}
            </span>
          )}
          {validation.kind === "ready" && validation.result.valid && saveState.kind === "idle" && (
            <span>{dirty ? "Unsaved changes" : "Everything is up to date"}</span>
          )}
        </div>
        <button
          className="button button--primary"
          type="button"
          disabled={
            !dirty ||
            validation.kind !== "ready" ||
            !validation.result.valid ||
            saveState.kind === "saving"
          }
          onClick={() => void handleSave()}
        >
          {saveState.kind === "saving" ? "Saving & applying…" : "Save & apply"}
        </button>
      </footer>
    </div>
  );
}
