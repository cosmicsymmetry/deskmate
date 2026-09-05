import {
  cardLabel,
  cardTitle,
  issuesForField,
  numberValue,
  tapActionDescription,
} from "../lib/configDraft";
import { providerTrouble } from "../lib/providers";
import { FieldIssues } from "./FieldIssues";
import { Icon } from "./Icon";
import type {
  AlertHold,
  CardAlert,
  CardError,
  CardSettings,
  JsonFieldMapping,
  PomodoroSnapshot,
  ProviderSnapshot,
  ValidationIssue,
  WeatherUnits,
} from "../lib/types";

const MAX_JSON_MAPPINGS = 16;
const MAX_RSS_ITEMS = 5;

interface CardEditorProps {
  card: CardSettings | null;
  /// Already scoped to this card by the caller via `issuesForCard` —
  /// CardEditor never resolves a card index itself, which is what makes
  /// dragging a card in the list safe: there is no stale index here for a
  /// reorder to invalidate.
  issues: ValidationIssue[];
  /// A typed device refusal for this card's last data or scene update.
  cardError: CardError | null;
  pomodoro: PomodoroSnapshot | null;
  /// The feed behind this card, when it has one. Only ever rendered when it is in
  /// trouble — see `providerTrouble`.
  provider: ProviderSnapshot | null;
  timerBusy: boolean;
  filePickerBusy: boolean;
  providerRefreshing: boolean;
  onChange: (card: CardSettings) => void;
  onRemove: () => void;
  onTimerAction: (action: "start" | "pause" | "reset") => void;
  onChooseCalendarFile: () => void;
  onRefreshProvider: () => void;
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

export function CardEditor({
  card,
  issues,
  cardError,
  pomodoro,
  provider,
  timerBusy,
  filePickerBusy,
  providerRefreshing,
  onChange,
  onRemove,
  onTimerAction,
  onChooseCalendarFile,
  onRefreshProvider,
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
  const trouble = providerTrouble(provider);
  const setAlert = (alert: CardAlert) => onChange({ ...card, alert });

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

      {/* The recovery half of the removed data-sources panel, moved to where it is
          actionable: beside the card whose data went bad, not in a list of every
          feed in the app that was healthy anyway. */}
      {trouble && (
        <p className="data-note" role="status">
          <span>{trouble}</span>
          <button
            className="text-button"
            type="button"
            disabled={providerRefreshing}
            onClick={onRefreshProvider}
          >
            <Icon name="refresh" />
            {providerRefreshing ? "Refreshing…" : "Refresh"}
          </button>
        </p>
      )}

      <div className="form-grid">
        {card.kind !== "pomodoro" && (
          <label className="field">
            <span>{card.kind === "clock" ? "Name" : "Heading"}</span>
            <input
              value={card.title}
              maxLength={64}
              onChange={(event) => onChange({ ...card, title: event.currentTarget.value })}
              aria-invalid={fieldIssues("title").length > 0}
            />
            <FieldIssues issues={fieldIssues("title")} />
          </label>
        )}

        {(card.kind === "json-feed" || card.kind === "rss") && (
          <label className="field">
            <span>Feed address</span>
            <input
              type="url"
              value={card.url}
              maxLength={2048}
              placeholder={`https://example.com/${card.kind === "json-feed" ? "data.json" : "feed.xml"}`}
              onChange={(event) => onChange({ ...card, url: event.currentTarget.value })}
              aria-invalid={fieldIssues("url").length > 0}
            />
            <FieldIssues issues={fieldIssues("url")} />
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

        {card.kind === "calendar" && (
          <>
            <fieldset className="source-picker">
              <legend>Calendar source</legend>
              <label>
                <input
                  type="radio"
                  name="source"
                  value="url"
                  checked={card.source.kind === "url"}
                  onChange={() => onChange({ ...card, source: { kind: "url", value: "" } })}
                />
                Web address
              </label>
              <label>
                <input
                  type="radio"
                  name="source"
                  value="file"
                  checked={card.source.kind === "file"}
                  onChange={() => onChange({ ...card, source: { kind: "file", value: "" } })}
                />
                File on this computer
              </label>
            </fieldset>
            <div className="field">
              <label htmlFor="calendar-source">
                {card.source.kind === "url" ? "ICS web address" : "ICS file"}
              </label>
              <div className={card.source.kind === "file" ? "file-picker-row" : undefined}>
                <input
                  id="calendar-source"
                  type={card.source.kind === "url" ? "url" : "text"}
                  value={card.source.value}
                  maxLength={2048}
                  placeholder={
                    card.source.kind === "url"
                      ? "https://calendar.example/my-calendar.ics"
                      : "Choose an iCalendar file"
                  }
                  onChange={(event) =>
                    onChange({
                      ...card,
                      source: { ...card.source, value: event.currentTarget.value },
                    })
                  }
                  aria-invalid={fieldIssues("source").length > 0}
                />
                {card.source.kind === "file" && (
                  <button
                    className="button button--quiet"
                    type="button"
                    disabled={filePickerBusy}
                    onClick={onChooseCalendarFile}
                  >
                    {filePickerBusy ? "Choosing…" : "Choose file…"}
                  </button>
                )}
              </div>
              <small>
                {card.source.kind === "file"
                  ? "Choose a local .ics or .ical file up to 1 MB."
                  : "Only this address is saved; downloaded events remain temporary."}
              </small>
              <FieldIssues issues={fieldIssues("source")} />
            </div>
          </>
        )}

        {card.kind === "weather" && (
          <>
            <label className="field">
              <span>Location</span>
              <input
                value={card.location}
                maxLength={128}
                placeholder="City, region, or postal code"
                onChange={(event) => onChange({ ...card, location: event.currentTarget.value })}
                aria-invalid={fieldIssues("location").length > 0}
              />
              <FieldIssues issues={fieldIssues("location")} />
            </label>
            <label className="field">
              <span>Units</span>
              <select
                value={card.units}
                onChange={(event) =>
                  onChange({ ...card, units: event.currentTarget.value as WeatherUnits })
                }
              >
                <option value="metric">Metric · °C</option>
                <option value="imperial">Imperial · °F</option>
              </select>
            </label>
          </>
        )}

        {card.kind === "json-feed" && (
          <fieldset className="mapping-list">
            <legend>Field mappings</legend>
            {card.mappings.length === 0 && (
              <p className="behaviour-hint">
                No fields mapped yet. Add one to pull a value out of the feed.
              </p>
            )}
            {card.mappings.map((mapping, index) => {
              const updateMapping = (patch: Partial<JsonFieldMapping>) =>
                onChange({
                  ...card,
                  mappings: card.mappings.map((entry, entryIndex) =>
                    entryIndex === index ? { ...entry, ...patch } : entry,
                  ),
                });
              return (
                // biome-ignore lint/suspicious/noArrayIndexKey: mappings have no id field; rows are addressed by position.
                <div className="mapping-row" key={index}>
                  <input
                    aria-label="Field name"
                    value={mapping.field}
                    maxLength={32}
                    placeholder="field name"
                    onChange={(event) => updateMapping({ field: event.currentTarget.value })}
                    aria-invalid={fieldIssues(`mappings[${index}].field`).length > 0}
                  />
                  <input
                    aria-label="JSON path"
                    value={mapping.path}
                    maxLength={256}
                    placeholder="json.path.to.value"
                    onChange={(event) => updateMapping({ path: event.currentTarget.value })}
                    aria-invalid={fieldIssues(`mappings[${index}].path`).length > 0}
                  />
                  <button
                    type="button"
                    className="text-button text-button--danger"
                    onClick={() =>
                      onChange({
                        ...card,
                        mappings: card.mappings.filter((_, entryIndex) => entryIndex !== index),
                      })
                    }
                  >
                    Remove
                  </button>
                  <FieldIssues issues={fieldIssues(`mappings[${index}].field`)} />
                  <FieldIssues issues={fieldIssues(`mappings[${index}].path`)} />
                </div>
              );
            })}
            <button
              type="button"
              className="button button--quiet"
              disabled={card.mappings.length >= MAX_JSON_MAPPINGS}
              onClick={() =>
                onChange({ ...card, mappings: [...card.mappings, { field: "", path: "" }] })
              }
            >
              Add field mapping
            </button>
            <FieldIssues issues={fieldIssues("mappings")} />
          </fieldset>
        )}

        {card.kind === "rss" && (
          <label className="field">
            <span>Headlines shown</span>
            <input
              type="number"
              className="numeral"
              min={1}
              max={MAX_RSS_ITEMS}
              step={1}
              value={card.max_items}
              onChange={(event) =>
                onChange({ ...card, max_items: numberValue(event.currentTarget.value) })
              }
              aria-invalid={fieldIssues("max_items").length > 0}
            />
            <FieldIssues issues={fieldIssues("max_items")} />
          </label>
        )}

        {/* Shared across every kind whose refresh policy is an interval — calendar,
            weather, json-feed, and rss. Clock and pomodoro are device-local and never
            reach here. This used to live only inside the calendar block, so weather/
            json-feed/rss cards kept whatever `addCard` chose forever with no way to
            change it. */}
        {(card.kind === "calendar" ||
          card.kind === "weather" ||
          card.kind === "json-feed" ||
          card.kind === "rss") && (
          <label className="field">
            <span>Refresh every</span>
            <select
              className="numeral"
              value={card.refresh.kind === "interval" ? card.refresh.minutes : 15}
              onChange={(event) =>
                onChange({
                  ...card,
                  refresh: { kind: "interval", minutes: numberValue(event.currentTarget.value) },
                })
              }
            >
              {card.refresh.kind === "interval" &&
                ![5, 15, 30, 60].includes(card.refresh.minutes) && (
                  <option value={card.refresh.minutes}>{card.refresh.minutes} minutes</option>
                )}
              <option value="5">5 minutes</option>
              <option value="15">15 minutes</option>
              <option value="30">30 minutes</option>
              <option value="60">1 hour</option>
            </select>
            <FieldIssues issues={fieldIssues("refresh")} />
          </label>
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

        {card.kind === "calendar" && (
          <fieldset className="alert-fieldset">
            <legend>Alert</legend>
            <label className="check-field">
              <input
                type="checkbox"
                checked={card.alert.kind === "before-event"}
                onChange={(event) =>
                  setAlert(
                    event.currentTarget.checked
                      ? {
                          kind: "before-event",
                          lead_minutes:
                            card.alert.kind === "before-event" ? card.alert.lead_minutes : 10,
                          hold:
                            card.alert.kind === "before-event"
                              ? card.alert.hold
                              : { kind: "until-dismissed" },
                        }
                      : { kind: "none" },
                  )
                }
              />
              <span>
                <strong>Take over the screen before an event</strong>
                <small>Shows full-screen ahead of the next calendar item.</small>
              </span>
            </label>
            {card.alert.kind === "before-event" && (
              <>
                <label className="field">
                  <span>Lead time in minutes</span>
                  <input
                    type="number"
                    className="numeral"
                    min={1}
                    max={60}
                    step={1}
                    value={card.alert.lead_minutes}
                    onChange={(event) =>
                      setAlert({
                        kind: "before-event",
                        lead_minutes: numberValue(event.currentTarget.value),
                        hold:
                          card.alert.kind === "before-event"
                            ? card.alert.hold
                            : { kind: "until-dismissed" },
                      })
                    }
                    aria-invalid={fieldIssues("alert.lead_minutes").length > 0}
                  />
                  <FieldIssues issues={fieldIssues("alert.lead_minutes")} />
                </label>
                <HoldSelector
                  hold={card.alert.hold}
                  onChange={(hold) =>
                    setAlert({
                      kind: "before-event",
                      lead_minutes:
                        card.alert.kind === "before-event" ? card.alert.lead_minutes : 10,
                      hold,
                    })
                  }
                  issues={fieldIssues("alert.hold.value")}
                />
              </>
            )}
            <FieldIssues issues={fieldIssues("alert")} />
          </fieldset>
        )}

        <div className="gesture-note">
          <p>{tapActionDescription(card)}</p>
          <p>
            Swiping the screen moves through the active playlist. While an alert is on screen, a tap
            dismisses it instead of performing the card's usual tap action.
          </p>
        </div>
      </div>
    </section>
  );
}
