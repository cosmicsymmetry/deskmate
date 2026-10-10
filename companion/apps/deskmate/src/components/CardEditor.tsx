import { useEffect, useRef, useState } from "react";

import {
  cardIdentity,
  issuesForField,
  numberValue,
  setCardDwell,
  tapActionDescription,
} from "../lib/configDraft";
import { listImageSources, toApiError, updateImageSourceFace } from "../lib/backend";
import type {
  AlertHold,
  AppConfig,
  CardAlert,
  CardError,
  CardSettings,
  FaceDescriptor,
  FaceStatus,
  ApiError,
  MintedImageSource,
  PomodoroSnapshot,
  ValidationIssue,
} from "../lib/types";
import { PRODUCT_NAME } from "../lib/product";
import { FieldIssues } from "./FieldIssues";

interface CardEditorProps {
  card: CardSettings | null;
  config: AppConfig;
  /// Card-scoped by the caller via `issuesForCard`, with absolute paths retained
  /// for field matching.
  issues: ValidationIssue[];
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
    <>
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
        {hold.kind === "seconds" && <small>The alert stays on screen until you tap it.</small>}
        <FieldIssues issues={issues} />
      </label>
      <details className="editor-help">
        <summary>How alert timing works</summary>
        <p>
          This setting controls when {PRODUCT_NAME} can accept another alert. It does not clear the
          display; tap the display to dismiss the current alert.
        </p>
      </details>
    </>
  );
}

function descriptorValues(descriptor: FaceDescriptor): Record<string, string> {
  return Object.fromEntries(descriptor.fields.map((field) => [field.key, field.value]));
}

/** How often the face's status is re-read while its settings are on screen. */
const FACE_STATUS_POLL_MS = 5_000;

/** Ends a server message with a full stop, without touching how it starts. */
const terminated = (text: string): string => (/[.!?]$/.test(text) ? text : `${text}.`);
/** For a message that stands alone. Never for one that may open with a hostname. */
const capitalized = (text: string): string => `${text.charAt(0).toUpperCase()}${text.slice(1)}`;

/** What the owner is told about a face, and whether it is a fault they must act on. */
function describeFaceStatus(
  status: FaceStatus | null,
  descriptor: FaceDescriptor,
): { text: string; alert: boolean } | null {
  switch (status?.state) {
    case "needs-settings": {
      const missing = descriptor.fields
        .filter((field) => field.type !== "enum" && !field.value.trim())
        .map((field) => field.label);
      return {
        text: `Fill in ${missing.join(" and ") || "the settings"} to start this face.`,
        alert: false,
      };
    }
    case "drawing":
      return { text: "Drawing the first frame…", alert: false };
    case "drawn": {
      const at = status.at_unix_seconds;
      const time =
        at === null
          ? ""
          : ` at ${new Date(at * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`;
      return { text: `Drawn${time}. It redraws on its own.`, alert: false };
    }
    case "needs-attention":
      return {
        text: capitalized(terminated(status.message ?? "these settings were refused")),
        alert: true,
      };
    case "retrying":
      return {
        text: `Couldn’t refresh: ${terminated(status.message ?? "the source did not answer")} The display keeps the last frame and this retries on its own.`,
        alert: false,
      };
    case "unavailable":
      return { text: "This server can’t draw this face right now.", alert: true };
    default:
      return null;
  }
}

/**
 * The settings of a server-drawn face. **They save themselves** -- on blur, on Enter,
 * and at once for a choice -- and there is deliberately no button here.
 *
 * There used to be one, "Save source settings", beside the window's own "Save to
 * server". Two saves for one card meant the prominent one silently discarded whatever
 * was typed here: the face stayed blank, was never fetched, and the panel said "Waiting
 * for the first picture" while the window said "Saved to the server". Clicking the
 * window's save still works, because the click blurs the field first.
 */
function PictureFaceSettings({
  sourceId,
  onTapSentence,
}: {
  sourceId: string;
  /** Lifts the face's own tap sentence to the editor's one gesture note. */
  onTapSentence: (sentence: string | null) => void;
}) {
  const requestGeneration = useRef(0);
  const saveInFlight = useRef(false);
  const [descriptor, setDescriptor] = useState<FaceDescriptor | null>(null);
  const [status, setStatus] = useState<FaceStatus | null>(null);
  const [values, setValues] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<ApiError | null>(null);

  useEffect(() => {
    const generation = requestGeneration.current + 1;
    requestGeneration.current = generation;
    setDescriptor(null);
    onTapSentence(null);
    setStatus(null);
    setValues({});
    setLoading(true);
    setSaving(false);
    setError(null);
    saveInFlight.current = false;
    const current = () => requestGeneration.current === generation;
    void listImageSources()
      .then((sources) => {
        if (!current()) {
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
        onTapSentence(source.face?.tap ?? null);
        setStatus(source.face_status);
        setValues(source.face ? descriptorValues(source.face) : {});
      })
      .catch((nextError) => {
        if (current()) {
          setError(toApiError(nextError));
        }
      })
      .finally(() => {
        if (current()) {
          setLoading(false);
        }
      });
    // The status moves on its own -- the first frame lands seconds after the last field
    // is filled in, and an API can start refusing at any refresh -- so it is re-read
    // while the form is on screen. Only the status: never the fields, mid-typing.
    const poll = window.setInterval(() => {
      void listImageSources()
        .then((sources) => {
          if (current()) {
            setStatus(sources.find((candidate) => candidate.id === sourceId)?.face_status ?? null);
          }
        })
        .catch(() => {});
    }, FACE_STATUS_POLL_MS);
    return () => {
      requestGeneration.current += 1;
      onTapSentence(null);
      window.clearInterval(poll);
    };
  }, [sourceId, onTapSentence]);

  if (loading) {
    return (
      <p className="source-settings__status" role="status">
        Loading source settings…
      </p>
    );
  }
  if (error && !descriptor) {
    return (
      <p className="data-note" role="alert">
        <span>{error.message}</span>
      </p>
    );
  }
  // An external producer has no server-owned face settings. Its existing
  // source identity line above remains the whole UI.
  if (!descriptor) {
    return null;
  }

  const save = (next: Record<string, string>) => {
    const changed = Object.fromEntries(
      descriptor.fields
        .filter((field) => next[field.key] !== field.value && (next[field.key] ?? "").trim())
        .map((field) => [field.key, next[field.key] ?? ""]),
    );
    // A blank required field is an unfinished form, not something to send: the server
    // would refuse it, and the status line already says what is missing.
    if (Object.keys(changed).length === 0 || saveInFlight.current) {
      return;
    }
    const generation = requestGeneration.current;
    const current = () => requestGeneration.current === generation;
    saveInFlight.current = true;
    setSaving(true);
    setError(null);
    void updateImageSourceFace(sourceId, changed)
      .then(async (updated) => {
        if (!current()) {
          return;
        }
        setDescriptor(updated);
        // Fields typed while this save was in flight stay as typed.
        setValues((typed) => ({ ...descriptorValues(updated), ...typed, ...changed }));
        const sources = await listImageSources().catch(() => null);
        if (current() && sources) {
          setStatus(sources.find((candidate) => candidate.id === sourceId)?.face_status ?? null);
        }
      })
      .catch((nextError) => {
        if (current()) {
          setError(toApiError(nextError));
        }
      })
      .finally(() => {
        if (current()) {
          saveInFlight.current = false;
          setSaving(false);
        }
      });
  };
  const setValue = (key: string, value: string): Record<string, string> => {
    const next = { ...values, [key]: value };
    setValues(next);
    setError(null);
    return next;
  };
  const shown = saving ? { text: "Saving…", alert: false } : describeFaceStatus(status, descriptor);

  return (
    <form
      className="source-settings"
      onSubmit={(event) => {
        event.preventDefault();
        save(values);
      }}
    >
      <fieldset className="source-settings__fields">
        <legend>{descriptor.label}</legend>
        {/* What is missing goes above the fields, where it is seen: below them it sat
            off-screen while the preview only said "No frame yet". */}
        {status?.state === "needs-settings" && !saving && shown && (
          <p className="data-note" role="alert">
            <span>{shown.text}</span>
          </p>
        )}
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
                    onClick={() => save(setValue(field.key, option.value))}
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
                // A bare example ("SOL", "Dubai") in an empty field reads as a value
                // already entered; two cards sat blank for days behind one.
                placeholder={field.placeholder ? `e.g. ${field.placeholder}` : undefined}
                aria-invalid={!(values[field.key] ?? "").trim() || undefined}
                onChange={(event) => setValue(field.key, event.currentTarget.value)}
                onBlur={() => save(values)}
              />
            </label>
          ),
        )}
      </fieldset>
      {error ? (
        <p className="data-note" role="alert">
          <span>{error.message}</span>
        </p>
      ) : shown?.alert ? (
        <p className="data-note" role="alert">
          <span>{shown.text}</span>
        </p>
      ) : shown && !(status?.state === "needs-settings" && !saving) ? (
        <p className="source-settings__status" role="status">
          {shown.text}
        </p>
      ) : null}
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
  // The face's tap sentence, lifted out of the settings form so the editor's one
  // gesture note can state it instead of claiming a tap does nothing.
  const [faceTap, setFaceTap] = useState<string | null>(null);
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
  // Names the card the way the heading does, so a listener and a reader are
  // told the same thing about the field they are on.
  const controlName = cardIdentity(card, config.image_sources);
  // Dwell is a field of the card since schema v10, so its issues arrive with
  // the card's own rather than through a separate playlist-entry path.
  const dwellIssues = fieldIssues("dwell_seconds");
  const showDwell = isTimed || card.dwell_seconds !== null || dwellIssues.length > 0;

  return (
    <section className="panel" aria-labelledby="editor-heading">
      <div className="panel-heading">
        <div className="editor-title">
          {/* The same words the tile shows. It read "Digital clock  Desk" while
              the tile it belonged to read "Clock" -- the heading was naming the
              template and repeating a title nothing could edit. */}
          <h2 id="editor-heading">{cardIdentity(card, config.image_sources)}</h2>
        </div>
        <button className="text-button text-button--danger" type="button" onClick={onRemove}>
          Remove
        </button>
      </div>

      {cardError && (
        <p className="data-note" role="alert">
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
        {/* No Name field. It decided nothing: a clock is a clock and a picture
            is named by its source, so the box only invited the owner to type a
            second name for something already named. `title` stays in the schema
            and keeps whatever it was created with -- removing a field would be a
            migration, and this is a change to what is offered, not to what is
            stored. The pomodoro's "Timer label" below is NOT this: that one is
            drawn on the panel. */}

        {card.kind === "clock" && (
          <label className="check-field">
            <input
              type="checkbox"
              checked={card.show_seconds}
              onChange={(event) => onChange({ ...card, show_seconds: event.currentTarget.checked })}
            />
            <span>
              <strong>Show seconds</strong>
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
                {/* The word carries the state; the colour only repeats it. */}
                <strong className={`timer-state timer-state--${pomodoro?.state ?? "idle"}`}>
                  {pomodoro?.state ?? "idle"}
                </strong>
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
            {/* Stated, not selected. Re-pointing a card would change its identity,
                so the source is chosen once when the card is added. */}
            <details
              className="editor-help"
              open={
                !config.image_sources.some((source) => source.id === card.source_id) ||
                fieldIssues("source_id").length > 0
                  ? true
                  : undefined
              }
            >
              <summary>Picture source</summary>
              <div className="field">
                <strong>
                  {config.image_sources.find((source) => source.id === card.source_id)?.name ??
                    `${card.source_id} · Missing source`}
                </strong>
                <span className="source-identifier">{card.source_id}</span>
                <FieldIssues issues={fieldIssues("source_id")} />
              </div>
            </details>

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
                  This plaintext token is shown once. Copy it now; {PRODUCT_NAME} cannot show it
                  again.
                </small>
              </div>
            )}

            <PictureFaceSettings sourceId={card.source_id} onTapSentence={setFaceTap} />
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

        <details className="editor-help editor-help--gestures">
          <summary>Display gestures</summary>
          <p>{tapActionDescription(card, faceTap)}</p>
          <p>Swipe to change cards. While an alert is on screen, a tap dismisses it instead.</p>
        </details>
      </div>
    </section>
  );
}
