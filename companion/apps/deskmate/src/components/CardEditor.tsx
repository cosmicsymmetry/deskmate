import { useEffect, useRef, useState } from "react";

import {
  cardLabel,
  cardTitle,
  issuesForField,
  numberValue,
  setCardDwell,
  tapActionDescription,
} from "../lib/configDraft";
import { listImageSources, toIpcError, updateImageSourceFace } from "../lib/backend";
import type {
  AlertHold,
  AppConfig,
  CardAlert,
  CardError,
  CardSettings,
  FaceDescriptor,
  IpcError,
  MintedImageSource,
  PomodoroSnapshot,
  ValidationIssue,
} from "../lib/types";
import { FieldIssues } from "./FieldIssues";

interface CardEditorProps {
  card: CardSettings | null;
  config: AppConfig;
  /// Already scoped to this card by the caller via `issuesForCard` —
  /// CardEditor never resolves a card index itself, which is what makes
  /// dragging a card in the list safe: there is no stale index here for a
  /// reorder to invalidate.
  issues: ValidationIssue[];
  /// Issues scoped to this card's active-loop entry. Empty for a card outside
  /// A typed device refusal for this card's last data or scene update.
  cardError: CardError | null;
  pomodoro: PomodoroSnapshot | null;
  timerBusy: boolean;
  /** Present only for the picture card whose source was minted this session. */
  pictureAccess?: MintedImageSource | null;
  onChange: (card: CardSettings) => void;
  onConfigChange: (config: AppConfig) => void;
  onRemove: () => void;
  onTimerAction: (action: "start" | "pause" | "reset") => void;
}

function HoldSelector({
  hold,
  onChange,
  issues,
}: {
  hold: AlertHold;
  onChange: (hold: AlertHold) => void;
  issues: ValidationIssue[];
}) {
  return (
    <label className="field">
      <span>Hold as outstanding</span>
      <select
        value={hold.kind}
        onChange={(event) =>
          onChange(
            event.currentTarget.value === "until-dismissed"
              ? { kind: "until-dismissed" }
              : { kind: "seconds", value: hold.kind === "seconds" ? hold.value : 30 },
          )
        }
      >
        <option value="until-dismissed">Until you tap it</option>
        <option value="seconds">For a limited time</option>
      </select>
      {hold.kind === "seconds" && (
        <input
          type="number"
          className="numeral"
          min={5}
          max={600}
          step={1}
          aria-label="Hold duration in seconds"
          value={hold.value}
          onChange={(event) =>
            onChange({ kind: "seconds", value: numberValue(event.currentTarget.value) })
          }
          aria-invalid={issues.length > 0}
        />
      )}
      <small>
        The alert stays on screen until you tap it, regardless of this setting — it only bounds how
        long Deskmate treats the alert as outstanding, freeing it for the next one.
      </small>
      <FieldIssues issues={issues} />
    </label>
  );
}

function descriptorValues(descriptor: FaceDescriptor): Record<string, string> {
  return Object.fromEntries(descriptor.fields.map((field) => [field.key, field.value]));
}

function PictureFaceSettings({ sourceId }: { sourceId: string }) {
  const requestGeneration = useRef(0);
  const [descriptor, setDescriptor] = useState<FaceDescriptor | null>(null);
  const [values, setValues] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<IpcError | null>(null);

  useEffect(() => {
    const generation = requestGeneration.current + 1;
    requestGeneration.current = generation;
    setDescriptor(null);
    setValues({});
    setLoading(true);
    setSaving(false);
    setSaved(false);
    setError(null);
    void listImageSources()
      .then((sources) => {
        if (requestGeneration.current !== generation) {
          return;
        }
        const source = sources.find((candidate) => candidate.id === sourceId);
        if (!source) {
          setError({
            category: "not-found",
            message: "This picture source no longer exists on the server.",
          });
          return;
        }
        setDescriptor(source.face);
        setValues(source.face ? descriptorValues(source.face) : {});
      })
      .catch((nextError) => {
        if (requestGeneration.current === generation) {
          setError(toIpcError(nextError));
        }
      })
      .finally(() => {
        if (requestGeneration.current === generation) {
          setLoading(false);
        }
      });
    return () => {
      requestGeneration.current += 1;
    };
  }, [sourceId]);

  if (loading) {
    return (
      <p className="source-settings__status" role="status">
        Loading source settings…
      </p>
    );
  }
  if (error && !descriptor) {
    return (
      <p className="data-note data-note--bad" role="alert">
        <span>{error.message}</span>
      </p>
    );
  }
  // An external producer has no server-owned face settings. Its existing
  // source identity line above remains the whole UI.
  if (!descriptor) {
    return null;
  }

  const dirty = descriptor.fields.some((field) => values[field.key] !== field.value);
  const setValue = (key: string, value: string) => {
    setValues((current) => ({ ...current, [key]: value }));
    setSaved(false);
    setError(null);
  };
  const save = () => {
    const generation = requestGeneration.current;
    setSaving(true);
    setSaved(false);
    setError(null);
    void updateImageSourceFace(sourceId, values)
      .then((updated) => {
        if (requestGeneration.current !== generation) {
          return;
        }
        setDescriptor(updated);
        setValues(descriptorValues(updated));
        setSaved(true);
      })
      .catch((nextError) => {
        if (requestGeneration.current === generation) {
          setError(toIpcError(nextError));
        }
      })
      .finally(() => {
        if (requestGeneration.current === generation) {
          setSaving(false);
        }
      });
  };

  return (
    <form
      className="source-settings"
      onSubmit={(event) => {
        event.preventDefault();
        if (dirty && !saving) {
          save();
        }
      }}
    >
      <fieldset className="source-settings__fields">
        <legend>{descriptor.label}</legend>
        {descriptor.fields.map((field) =>
          field.type === "enum" ? (
            <fieldset className="source-settings__enum" key={field.key}>
              <legend>{field.label}</legend>
              <div className="source-settings__segments">
                {field.options.map((option) => (
                  <button
                    type="button"
                    className="source-settings__segment"
                    aria-pressed={values[field.key] === option.value}
                    key={option.value}
                    onClick={() => setValue(field.key, option.value)}
                  >
                    {option.label}
                  </button>
                ))}
              </div>
            </fieldset>
          ) : (
            <label className="field" key={field.key}>
              <span>{field.label}</span>
              <input
                type={field.type}
                required
                maxLength={2048}
                value={values[field.key] ?? ""}
                placeholder={field.placeholder}
                onChange={(event) => setValue(field.key, event.currentTarget.value)}
              />
            </label>
          ),
        )}
      </fieldset>
      <div className="source-settings__actions">
        <button className="button button--secondary" type="submit" disabled={!dirty || saving}>
          {saving ? "Saving…" : "Save source settings"}
        </button>
        {saved && (
          <small role="status">
            Saved on the server. The refreshed face will use these settings.
          </small>
        )}
      </div>
      {error && (
        <p className="data-note data-note--bad" role="alert">
          <span>{error.message}</span>
        </p>
      )}
    </form>
  );
}

export function CardEditor({
  card,
  config,
  issues,
  cardError,
  pomodoro,
  timerBusy,
  pictureAccess = null,
  onChange,
  onConfigChange,
  onRemove,
  onTimerAction,
}: CardEditorProps) {
  if (!card) {
    return (
      <section className="panel" aria-labelledby="editor-heading">
        <div className="panel-heading">
          <div>
            <h2 id="editor-heading">Choose a card</h2>
          </div>
        </div>
        <div className="empty-state empty-state--large">
          <strong>No card selected</strong>
          <span>Select a card on the left, or add a new one.</span>
        </div>
      </section>
    );
  }

  const fieldIssues = (field: string) => issuesForField(issues, field);
  const setAlert = (alert: CardAlert) => onChange({ ...card, alert });
  const isTimed = config.advance.kind === "timed";
  const title = cardTitle(card);
  const controlName = title ? `${cardLabel(card)} — ${title}` : cardLabel(card);
  // Dwell is a field of the card since schema v10, so its issues arrive with
  // the card's own rather than through a separate playlist-entry path.
  const dwellIssues = fieldIssues("dwell_seconds");
  const showDwell = isTimed || card.dwell_seconds !== null || dwellIssues.length > 0;

  return (
    <section className="panel" aria-labelledby="editor-heading">
      <div className="panel-heading">
        <div className="editor-title">
          <h2 id="editor-heading">{cardLabel(card)}</h2>
          {cardTitle(card) && <span className="editor-title__kind">{cardTitle(card)}</span>}
        </div>
        <button className="text-button text-button--danger" type="button" onClick={onRemove}>
          Remove
        </button>
      </div>

      {cardError && (
        <p className="data-note data-note--bad" role="alert">
          <span>
            <strong>
              {cardError.kind === "scene-refused"
                ? "This card could not be rendered."
                : "This card’s data was refused."}
            </strong>{" "}
            {cardError.message} Adjust the card and save to try again.
          </span>
        </p>
      )}

      <div className="form-grid">
        {card.kind !== "pomodoro" && (
          <label className="field">
            <span>Name</span>
            <input
              value={card.title}
              maxLength={64}
              onChange={(event) => onChange({ ...card, title: event.currentTarget.value })}
              aria-invalid={fieldIssues("title").length > 0}
            />
            <FieldIssues issues={fieldIssues("title")} />
          </label>
        )}

        {card.kind === "clock" && (
          <label className="check-field">
            <input
              type="checkbox"
              checked={card.show_seconds}
              onChange={(event) => onChange({ ...card, show_seconds: event.currentTarget.checked })}
            />
            <span>
              <strong>Show seconds</strong>
              <small>Add seconds beside the large time.</small>
            </span>
          </label>
        )}

        {card.kind === "pomodoro" && (
          <>
            <label className="field">
              <span>Timer label</span>
              <input
                value={card.label}
                maxLength={64}
                onChange={(event) => onChange({ ...card, label: event.currentTarget.value })}
                aria-invalid={fieldIssues("label").length > 0}
              />
              <FieldIssues issues={fieldIssues("label")} />
            </label>
            <label className="field">
              <span>Duration in minutes</span>
              <input
                type="number"
                className="numeral"
                min="1"
                max="1440"
                step="1"
                value={card.duration_seconds / 60}
                onChange={(event) =>
                  onChange({
                    ...card,
                    duration_seconds: Math.round(numberValue(event.currentTarget.value) * 60),
                  })
                }
                aria-invalid={fieldIssues("duration_seconds").length > 0}
              />
              <FieldIssues issues={fieldIssues("duration_seconds")} />
            </label>
            <fieldset className="timer-controls">
              <legend className="sr-only">Pomodoro controls</legend>
              <span>
                <strong>{pomodoro?.state ?? "idle"}</strong>
                <small className={pomodoro ? "numeral" : undefined}>
                  {pomodoro
                    ? `${Math.ceil(pomodoro.remaining_seconds / 60)} min remaining`
                    : "Save this card to start it"}
                </small>
              </span>
              <div>
                {pomodoro?.state === "running" ? (
                  <button
                    className="button button--secondary"
                    type="button"
                    disabled={timerBusy}
                    onClick={() => onTimerAction("pause")}
                  >
                    Pause
                  </button>
                ) : (
                  <button
                    className="button button--secondary"
                    type="button"
                    disabled={timerBusy || !pomodoro}
                    onClick={() => onTimerAction("start")}
                  >
                    Start
                  </button>
                )}
                <button
                  className="button button--quiet"
                  type="button"
                  disabled={timerBusy || !pomodoro}
                  onClick={() => onTimerAction("reset")}
                >
                  Reset
                </button>
              </div>
            </fieldset>
          </>
        )}

        {card.kind === "picture" && (
          <>
            {/* Stated, not selected. A dropdown here implied that re-pointing a
                card at another source was a safe edit, when it silently turned the
                card into a different card -- the same reasoning that retired the
                plugin dropdown in 324c981. The source is chosen once, when the
                card is added. */}
            <div className="field">
              <span>Picture source</span>
              <strong>
                {config.image_sources.find((source) => source.id === card.source_id)?.name ??
                  `${card.source_id} · Missing source`}
              </strong>
              <small>{card.source_id}</small>
              <FieldIssues issues={fieldIssues("source_id")} />
            </div>

            {/* `pictureAccess &&` first, deliberately. Optional chaining alone
                compared `undefined === undefined` whenever there was no access
                AND the card had no source, passing a guard whose whole job was
                to keep a null out of the next line. */}
            {pictureAccess && pictureAccess.source_id === card.source_id && (
              <div className="field" role="status">
                <label htmlFor="picture-push-url">Push URL</label>
                <input id="picture-push-url" readOnly value={pictureAccess.push_url} />
                <label htmlFor="picture-source-token">Source token</label>
                <div className="file-picker-row">
                  <input id="picture-source-token" readOnly value={pictureAccess.token} />
                  <button
                    className="button button--quiet"
                    type="button"
                    onClick={() => void navigator.clipboard.writeText(pictureAccess.token)}
                  >
                    Copy token
                  </button>
                </div>
                <small>
                  This plaintext token is shown once. Copy it now; Deskmate cannot show it again.
                </small>
              </div>
            )}

            <PictureFaceSettings sourceId={card.source_id} />
          </>
        )}

        {card.kind === "pomodoro" && (
          <fieldset className="alert-fieldset">
            <legend>Alert</legend>
            <label className="check-field">
              <input
                type="checkbox"
                checked={card.alert.kind === "on-timer-finish"}
                onChange={(event) =>
                  setAlert(
                    event.currentTarget.checked
                      ? {
                          kind: "on-timer-finish",
                          hold:
                            card.alert.kind === "on-timer-finish"
                              ? card.alert.hold
                              : { kind: "until-dismissed" },
                        }
                      : { kind: "none" },
                  )
                }
              />
              <span>
                <strong>Take over the screen when the timer ends</strong>
                <small>
                  Shows full-screen until you tap it. The hold setting below controls how long
                  Deskmate treats it as outstanding, not how long it's shown.
                </small>
              </span>
            </label>
            {card.alert.kind === "on-timer-finish" && (
              <HoldSelector
                hold={card.alert.hold}
                onChange={(hold) => setAlert({ kind: "on-timer-finish", hold })}
                issues={fieldIssues("alert.hold.value")}
              />
            )}
            <FieldIssues issues={fieldIssues("alert")} />
          </fieldset>
        )}

        {showDwell && (
          <label className="field">
            <span>Stays on the panel for</span>
            <input
              type="number"
              className="numeral"
              inputMode="numeric"
              min={5}
              max={3600}
              step={1}
              value={card.dwell_seconds ?? ""}
              placeholder={
                config.advance.kind === "timed"
                  ? `${config.advance.default_dwell_seconds} s, the loop's default`
                  : undefined
              }
              aria-label={`Stays on the panel for ${controlName}`}
              aria-invalid={dwellIssues.length > 0}
              onChange={(event) =>
                onConfigChange(
                  setCardDwell(
                    config,
                    card.id,
                    event.currentTarget.value === ""
                      ? null
                      : numberValue(event.currentTarget.value),
                  ),
                )
              }
            />
            {!isTimed && <small>Dwell applies when the loop is timed.</small>}
            <FieldIssues issues={dwellIssues} />
          </label>
        )}

        <div className="gesture-note">
          <p>{tapActionDescription(card)}</p>
          <p>
            Swiping the screen moves through the loop. While an alert is on screen, a tap dismisses
            it instead of performing the card's usual tap action.
          </p>
        </div>
      </div>
    </section>
  );
}
