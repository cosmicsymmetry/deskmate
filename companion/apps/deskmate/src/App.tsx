import { useEffect, useRef, useState } from "react";

import { AccountSection } from "./components/AccountSection";
import { CardEditor } from "./components/CardEditor";
import { CardList } from "./components/CardList";
import { DevicePreview } from "./components/DevicePreview";
import { LinkLanding, SetupScreen, SignInScreen } from "./components/AuthScreens";
import { Icon } from "./components/Icon";
import { LoopRing } from "./components/LoopRing";
import { NetworkPanel, ownershipLabel } from "./components/NetworkPanel";
import { PanelSetup } from "./components/PanelSetup";
import { SaveBar, type SaveState, type ValidationState } from "./components/SaveBar";
import { SettingsSheet } from "./components/SettingsSheet";
import { TopBar } from "./components/TopBar";
import { getInstance, signOut, type Instance } from "./lib/account";
import {
  type AddCardRequest,
  addCard,
  cardLabel,
  copyConfig,
  firstRunSteps,
  firstSelectableCard,
  issuesForCard,
  nextCardName,
  removeCard,
  unclaimedIssues,
  updateWidget,
} from "./lib/configDraft";
import {
  controlPomodoro,
  listCreatableFaces,
  mintImageSource,
  resumePushing,
  toApiError,
  validateConfigDraft,
  isNoPanels,
  isSessionMissing,
} from "./lib/backend";
import type {
  AppConfig,
  AppSnapshot,
  CardSettings,
  DisplayOrientation,
  DraftValidation,
  FaceDescriptor,
  ApiError,
  ImageSource,
  MintedImageSource,
  PomodoroAction,
} from "./lib/types";
import { useAppState } from "./lib/useAppState";

/** The device link in one phrase for the troubleshooting facts in the settings sheet. */
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

const validDraft: DraftValidation = { valid: true, issues: [] };

export function App() {
  const [instance, setInstance] = useState<Instance | null>(null);
  const [instanceError, setInstanceError] = useState<ApiError | null>(null);
  const [instanceGeneration, setInstanceGeneration] = useState(0);
  const [routeGeneration, setRouteGeneration] = useState(0);

  // `instanceGeneration` is an explicit retry signal; the request itself has
  // no argument for the linter to trace as a dependency.
  // biome-ignore lint/correctness/useExhaustiveDependencies: retrying must rerun this effect
  useEffect(() => {
    let active = true;
    setInstanceError(null);
    void getInstance()
      .then((next) => {
        if (active) setInstance(next);
      })
      .catch((next) => {
        if (active) setInstanceError(toApiError(next));
      });
    return () => {
      active = false;
    };
  }, [instanceGeneration]);

  if (!instance) {
    if (instanceError) {
      return (
        <main className="startup startup--error">
          <span className="startup__ring startup__ring--error" aria-hidden="true" />
          <h1>Settings could not be loaded</h1>
          <p role="alert">{instanceError.message}</p>
          <button
            className="button button--primary"
            type="button"
            onClick={() => setInstanceGeneration((current) => current + 1)}
          >
            Try again
          </button>
        </main>
      );
    }
    return (
      <main className="startup" aria-busy="true">
        <span className="startup__ring" aria-hidden="true" />
        <h1>Waking the display…</h1>
        <p>The server keeps the display updated whether or not this page is open.</p>
      </main>
    );
  }

  if (instance.setup_required) {
    return (
      <SetupScreen
        onComplete={() =>
          setInstance((current) => (current ? { ...current, setup_required: false } : current))
        }
      />
    );
  }

  const route = `${window.location.pathname}?${routeGeneration}`;
  if (window.location.pathname === "/signin") {
    return (
      <LinkLanding
        token={new URLSearchParams(window.location.search).get("token") ?? ""}
        onComplete={() => setRouteGeneration((current) => current + 1)}
        onBack={() => {
          window.history.replaceState(null, "", "/");
          setRouteGeneration((current) => current + 1);
        }}
      />
    );
  }

  return (
    <DeviceApp
      key={route}
      instance={instance}
      signInError={new URLSearchParams(window.location.search).get("signin_error")}
      onSessionEnded={() => setRouteGeneration((current) => current + 1)}
    />
  );
}

function DeviceApp({
  instance,
  signInError,
  onSessionEnded,
}: {
  instance: Instance;
  signInError: string | null;
  onSessionEnded: () => void;
}) {
  const {
    snapshot,
    loading,
    error: stateError,
    refresh,
    dataGeneration,
    ownershipTier,
    saveConfig,
  } = useAppState();
  const [draft, setDraft] = useState<AppConfig | null>(null);
  const draftRef = useRef<AppConfig | null>(draft);
  draftRef.current = draft;
  const draftRevisionRef = useRef(0);
  const saveInFlightRef = useRef(false);
  const mintInFlightRef = useRef(false);
  const [minting, setMinting] = useState(false);
  const [resuming, setResuming] = useState(false);
  const [selectedCardId, setSelectedCardId] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);
  const dirtyRef = useRef(false);
  const [validation, setValidation] = useState<ValidationState>({
    kind: "idle",
    result: validDraft,
  });
  const [saveState, setSaveState] = useState<SaveState>({ kind: "idle" });
  const [actionError, setActionError] = useState<ApiError | null>(null);
  const [busyAction, setBusyAction] = useState<string | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsVisited, setSettingsVisited] = useState(false);
  const [mintedPicture, setMintedPicture] = useState<{
    cardId: string;
    access: MintedImageSource;
  } | null>(null);

  const [creatableFaces, setCreatableFaces] = useState<FaceDescriptor[]>([]);

  const hasSnapshot = snapshot !== null;

  // Requested once after the first authenticated snapshot. A
  // failure here must not block the page -- the menu simply offers the
  // built-in kinds, exactly as it did before the server could draw anything.
  useEffect(() => {
    if (!hasSnapshot) {
      return;
    }
    let cancelled = false;
    void listCreatableFaces()
      .then((faces) => {
        if (!cancelled) {
          setCreatableFaces(faces);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setCreatableFaces([]);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [hasSnapshot]);

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
              error: toApiError(nextError),
            });
          }
        });
    }, 180);
    return () => {
      active = false;
      window.clearTimeout(timeout);
    };
  }, [draft]);

  if (stateError && isSessionMissing(stateError)) {
    return <SignInScreen instance={instance} signInError={signInError} />;
  }

  if (loading && !snapshot) {
    return (
      <main className="startup" aria-busy="true">
        <span className="startup__ring" aria-hidden="true" />
        <h1>Waking the display…</h1>
        <p>The server keeps the display updated whether or not this page is open.</p>
      </main>
    );
  }

  if (!snapshot || !draft) {
    // A missing session is the one failure with a specific answer, so it gets a
    // specific screen. Everything else is "retry"; offering only that when the
    // real problem is "sign in" would make the page a dead end.
    if (stateError && isNoPanels(stateError)) {
      return (
        <main className="startup">
          <PanelSetup standalone onDone={() => window.location.assign("/")} />
          <button
            className="button button--quiet"
            type="button"
            disabled={busyAction === "sign-out"}
            onClick={() => {
              setBusyAction("sign-out");
              setActionError(null);
              void signOut()
                .then(onSessionEnded)
                .catch((next) => setActionError(toApiError(next)))
                .finally(() => setBusyAction(null));
            }}
          >
            {busyAction === "sign-out" ? "Signing out…" : "Sign out"}
          </button>
          {actionError && (
            <p className="save-error" role="alert">
              {actionError.message}
            </p>
          )}
        </main>
      );
    }
    return (
      <main className="startup startup--error">
        <span className="startup__ring startup__ring--error" aria-hidden="true" />
        <h1>Settings could not be loaded</h1>
        <p role="alert">{stateError?.message ?? "The server is unavailable."}</p>
        <button className="button button--primary" type="button" onClick={() => void refresh()}>
          Try again
        </button>
      </main>
    );
  }

  const selectedWidget = draft.cards.find((card) => card.id === selectedCardId) ?? null;
  const pomodoro =
    snapshot.pomodoros.find((candidate) => candidate.card_id === selectedCardId) ?? null;
  const issues = validation.result.issues;
  const cardIssues = selectedCardId ? issuesForCard(issues, draft, selectedCardId) : [];
  const selectedCardError =
    snapshot.card_errors.find((error) => error.card_id === selectedCardId) ?? null;
  const cardErrorCount = snapshot.card_errors.length;
  const allCardErrorsAreDataRefusals = snapshot.card_errors.every(
    (error) => error.kind === "data-refused",
  );
  const allCardErrorsAreSceneRefusals = snapshot.card_errors.every(
    (error) => error.kind === "scene-refused",
  );
  const cardErrorHeading = allCardErrorsAreDataRefusals
    ? cardErrorCount === 1
      ? "The display refused one card update"
      : `The display refused ${cardErrorCount} card updates`
    : allCardErrorsAreSceneRefusals
      ? cardErrorCount === 1
        ? "One card could not be rendered"
        : `${cardErrorCount} cards could not be rendered`
      : `${cardErrorCount} card updates need attention`;
  const affectedCards = cardErrorCount === 1 ? "the affected card" : "each affected card";
  const affectedCardPronoun = cardErrorCount === 1 ? "it" : "them";
  // The fallback contains issues not claimed by card, document-level advance, or timezone surfaces.
  const leftoverIssues = unclaimedIssues(issues, draft);
  const serverOwned = ownershipTier === "networked";
  const protocolMismatch =
    snapshot.device.protocol_version !== null &&
    snapshot.device.protocol_version !== snapshot.host_protocol_version;
  const paused = snapshot.config.preferences.paused || snapshot.runtime.kind === "paused";
  // The settings button is silent while everything is nominal, and carries a dot
  // only for the things whose answers live behind it.
  const needsAttention =
    !serverOwned ||
    protocolMismatch ||
    snapshot.device.wifi_state === "failed" ||
    snapshot.device.last_network_error !== null;

  const replaceDraft = (next: AppConfig) => {
    draftRevisionRef.current += 1;
    draftRef.current = next;
    setDraft(next);
    dirtyRef.current = true;
    setDirty(true);
    if (!saveInFlightRef.current) {
      setSaveState({ kind: "idle" });
    }
  };
  const handleAdd = (request: AddCardRequest): string | null => {
    // Source minting is asynchronous. Read the latest draft here so settings edited
    // while the server responds are not replaced by the render that began the mint.
    const currentDraft = draftRef.current;
    if (!currentDraft) {
      return null;
    }
    const result = addCard(currentDraft, request);
    if (!result.cardId) {
      return null;
    }
    replaceDraft(result.config);
    setSelectedCardId(result.cardId);
    return result.cardId;
  };
  const handleSelectCard = (cardId: string) => {
    setMintedPicture((current) => (current?.cardId === cardId ? current : null));
    setSelectedCardId(cardId);
  };
  const mintPictureCard = (label: string, faceKind?: string): void => {
    const currentDraft = draftRef.current;
    if (!currentDraft || mintInFlightRef.current || saveInFlightRef.current) {
      return;
    }
    const sourceName = nextCardName(currentDraft, label);
    mintInFlightRef.current = true;
    setMinting(true);
    setActionError(null);
    void mintImageSource(sourceName, faceKind)
      .then((access) => {
        const cardId = handleAdd({
          kind: "picture",
          sourceId: access.source_id,
          sourceName,
        });
        if (faceKind === undefined && cardId) {
          setMintedPicture({ cardId, access });
        }
      })
      .catch((nextError) => setActionError(toApiError(nextError)))
      .finally(() => {
        mintInFlightRef.current = false;
        setMinting(false);
      });
  };
  const handleAddPicture = (source: ImageSource | null) => {
    if (source) {
      setMintedPicture(null);
      handleAdd({ kind: "picture", sourceId: source.id, sourceName: source.name });
      return;
    }
    mintPictureCard("Picture");
  };

  /// Adding a server-drawn card: name it, mint a source carrying that face, and
  /// put the card in the loop. The owner types nothing -- the name is chosen and
  /// the source never surfaces.
  const handleAddFace = (kind: string, label: string) => {
    mintPictureCard(label, kind);
  };

  const handleWidgetChange = (widget: CardSettings) => {
    if (!selectedCardId) {
      return;
    }
    if (
      mintedPicture?.cardId === widget.id &&
      widget.kind === "picture" &&
      mintedPicture.access.source_id !== widget.source_id
    ) {
      setMintedPicture(null);
    }
    replaceDraft(updateWidget(draft, selectedCardId, widget));
  };
  const handleRemoveCard = (cardId: string) => {
    const next = removeCard(draft, cardId);
    setMintedPicture((current) => (current?.cardId === cardId ? null : current));
    replaceDraft(next);
    setSelectedCardId((current) => (current === cardId ? firstSelectableCard(next) : current));
  };
  const handleRemove = () => {
    if (!selectedCardId) {
      return;
    }
    handleRemoveCard(selectedCardId);
  };
  const runAction = async (name: string, operation: () => Promise<void>) => {
    setBusyAction(name);
    setActionError(null);
    try {
      await operation();
      await refresh();
    } catch (nextError) {
      setActionError(toApiError(nextError));
    } finally {
      setBusyAction(null);
    }
  };
  const handleSave = async () => {
    if (
      saveInFlightRef.current ||
      mintInFlightRef.current ||
      validation.kind !== "ready" ||
      !validation.result.valid
    ) {
      return;
    }
    const submittedDraft = draftRef.current;
    if (!submittedDraft) {
      return;
    }
    const submittedRevision = draftRevisionRef.current;
    saveInFlightRef.current = true;
    setSaveState({ kind: "saving" });
    setActionError(null);
    try {
      const result = await saveConfig(submittedDraft);
      await refresh();
      if (draftRevisionRef.current === submittedRevision) {
        dirtyRef.current = false;
        setDirty(false);
        setSaveState({
          kind: "saved",
          message: result.save.warning
            ? `Saved. ${result.save.warning.message}`
            : "Saved to the server. The server will update your display.",
        });
      } else {
        setSaveState({ kind: "idle" });
      }
    } catch (nextError) {
      setSaveState(
        draftRevisionRef.current === submittedRevision
          ? { kind: "error", error: toApiError(nextError) }
          : { kind: "idle" },
      );
    } finally {
      saveInFlightRef.current = false;
    }
  };
  const handleResume = async () => {
    if (dirtyRef.current || saveInFlightRef.current || mintInFlightRef.current) {
      return;
    }
    // Resuming writes configuration too, so it must not revoke a source being minted.
    saveInFlightRef.current = true;
    setResuming(true);
    try {
      await runAction("resume", resumePushing);
    } finally {
      saveInFlightRef.current = false;
      setResuming(false);
    }
  };
  const handleTimerAction = (action: "start" | "pause" | "reset") => {
    if (!selectedCardId) {
      return;
    }
    void runAction("timer", () => controlPomodoro(selectedCardId, action satisfies PomodoroAction));
  };

  const persistenceError =
    snapshot.persistence.kind === "recoverable-error" ? snapshot.persistence.message : null;
  const persistenceValidation =
    snapshot.persistence.kind === "validation-failed" ? snapshot.persistence : null;

  return (
    <div className="app">
      <TopBar
        attention={needsAttention}
        onOpenSettings={() => {
          setSettingsVisited(true);
          setSettingsOpen(true);
        }}
      />

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
            imageSources={draft.image_sources}
          />
          <LoopRing
            config={draft}
            issues={issues}
            selectedCardId={selectedCardId}
            onSelect={handleSelectCard}
            onChange={replaceDraft}
          />
        </aside>

        <main className="face__work">
          {/* Consequential failures stay in the working column where they remain
              visible without turning nominal state into permanent chrome. */}
          {(protocolMismatch || snapshot.runtime.kind === "error" || actionError || stateError) && (
            <aside className="notice notice--bad" role="alert">
              <div>
                <strong>
                  {protocolMismatch
                    ? `This display speaks protocol ${snapshot.device.protocol_version}; this server speaks protocol ${snapshot.host_protocol_version}.`
                    : snapshot.runtime.kind === "error"
                      ? snapshot.runtime.message
                      : (actionError ?? stateError)?.message}
                </strong>
              </div>
            </aside>
          )}

          {/* The schema can contain a paused configuration even though this page has
              no pause control, so the recovery action must remain reachable. */}
          {paused && (
            <aside className="notice notice--warn" role="status">
              <div>
                <strong>Sending to the display is paused</strong>
                <p>
                  Nothing you change here reaches the panel until you resume.
                  {dirty && " Save your changes before resuming."}
                </p>
                <button
                  className="button button--quiet"
                  type="button"
                  disabled={dirty || resuming || minting || saveState.kind === "saving"}
                  onClick={() => void handleResume()}
                >
                  {resuming ? "Resuming…" : "Resume sending"}
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

          {persistenceError && (
            <aside className="notice notice--warn" role="status">
              <div>
                <strong>Settings file needs attention</strong>
                <p>
                  {`${persistenceError}. The unreadable file was left untouched; review the settings shown here before saving a fresh valid configuration.`}
                </p>
              </div>
            </aside>
          )}

          {/* Card-scoped failures stay actionable without inventing a cause: data
              refusals name the display, while scene failures remain neutral because
              scene construction can fail before the display sees anything. */}
          {snapshot.card_errors.length > 0 && (
            <aside className="notice notice--warn" role="status">
              <div>
                <strong>{cardErrorHeading}</strong>
                {snapshot.card_errors.map((cardError) => {
                  const card = draft.cards.find((candidate) => candidate.id === cardError.card_id);
                  return (
                    <p key={cardError.card_id}>
                      <strong>{card ? cardLabel(card) : cardError.card_id}</strong> —{" "}
                      {cardError.message}
                    </p>
                  );
                })}
                <p>
                  Everything else kept updating. Open {affectedCards} to review the details and save
                  after correcting {affectedCardPronoun}.
                </p>
              </div>
            </aside>
          )}

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
            pomodoros={snapshot.pomodoros}
            selectedCardId={selectedCardId}
            onSelect={handleSelectCard}
            onAdd={handleAdd}
            onAddPicture={handleAddPicture}
            creatableFaces={creatableFaces}
            onAddFace={handleAddFace}
            pictureBusy={minting}
            saving={saveState.kind === "saving" || resuming}
            onChange={replaceDraft}
            onRemove={handleRemoveCard}
          />

          <CardEditor
            card={selectedWidget}
            config={draft}
            issues={cardIssues}
            cardError={selectedCardError}
            pomodoro={pomodoro}
            timerBusy={busyAction === "timer"}
            pictureAccess={mintedPicture?.cardId === selectedCardId ? mintedPicture.access : null}
            onChange={handleWidgetChange}
            onConfigChange={replaceDraft}
            onRemove={handleRemove}
            onTimerAction={handleTimerAction}
          />
        </main>
      </div>

      {/* Ownership, pairing and the device's own network state: a first-run task and
          a troubleshooting task, and nothing you look at while arranging cards. It
          answers for itself here rather than taxing every session for the privilege. */}
      <SettingsSheet open={settingsOpen} title="Settings" onClose={() => setSettingsOpen(false)}>
        {settingsVisited && (
          <AccountSection
            open={settingsOpen}
            instance={instance}
            onSessionEnded={onSessionEnded}
            onSignupsChanged={() => {}}
            onPanelsChanged={() => void refresh()}
          />
        )}

        <section className="sheet__section" aria-labelledby="preferences-heading">
          <h3 id="preferences-heading">Display</h3>
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
            <div className="field">
              <span className="brightness-label">
                <label htmlFor="display-brightness">Brightness</label>
                <output htmlFor="display-brightness">{draft.preferences.brightness}%</output>
              </span>
              <input
                id="display-brightness"
                type="range"
                aria-label="Brightness"
                aria-valuetext={`${draft.preferences.brightness}%`}
                aria-describedby="brightness-help"
                min={10}
                max={100}
                step={1}
                value={draft.preferences.brightness}
                onChange={(event) =>
                  replaceDraft({
                    ...draft,
                    preferences: {
                      ...draft.preferences,
                      brightness: Number(event.currentTarget.value),
                    },
                  })
                }
                aria-invalid={issues.some((issue) => issue.path === "preferences.brightness")}
              />
              <small id="brightness-help">
                {(snapshot.device.connection.kind === "online" ||
                  snapshot.device.connection.kind === "standalone") &&
                !snapshot.device.capabilities.includes("display-brightness")
                  ? "Your panel’s firmware does not support brightness yet. You can save a level for when it does."
                  : "Saved with your layout. The preview stays at full brightness."}
              </small>
              {issues
                .filter((issue) => issue.path === "preferences.brightness")
                .map((issue) => (
                  <small className="field-error" role="alert" key={issue.code}>
                    {issue.message}
                  </small>
                ))}
            </div>
          </div>
        </section>

        <section className="sheet__section" aria-labelledby="device-heading">
          <div className="sheet__section-head">
            <h3 id="device-heading">Device</h3>
            <strong
              className={`ownership-badge ownership-badge--${serverOwned ? "networked" : "unknown"}`}
            >
              {ownershipLabel(ownershipTier)}
            </strong>
          </div>
          <NetworkPanel
            device={{
              link: linkLabel(snapshot),
              wifiState: snapshot.device.wifi_state,
              wifiRssi: snapshot.device.wifi_rssi,
              ip: snapshot.device.ip ?? "",
              lastNetworkError: snapshot.device.last_network_error,
              otaState: snapshot.device.ota_state,
            }}
          />
        </section>

        {/* Preferences are draft state, so the sheet needs the same Save the page
            has — a modal that can strand an edit behind itself is a trap. */}
        <SaveBar
          validation={validation}
          saveState={saveState}
          ownershipTier={ownershipTier}
          dirty={dirty}
          busy={minting || resuming}
          variant="sheet"
          onSave={() => void handleSave()}
        />
      </SettingsSheet>

      <SaveBar
        validation={validation}
        saveState={saveState}
        ownershipTier={ownershipTier}
        dirty={dirty}
        busy={minting || resuming}
        onSave={() => void handleSave()}
      />
    </div>
  );
}
