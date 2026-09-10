import {
  activePlaylist,
  cardLabel,
  cardTitle,
  issuesForField,
  numberValue,
  setEntryDwell,
  tapActionDescription,
} from "../lib/configDraft";
import { providerTrouble } from "../lib/providers";
import type {
  AlertHold,
  AppConfig,
  CardAlert,
  CardError,
  CardSettings,
  DeviceTier,
  MintedImageSource,
  PluginCatalog,
  PomodoroSnapshot,
  ProviderSnapshot,
  ValidationIssue,
} from "../lib/types";
import { FieldIssues } from "./FieldIssues";
import { Icon } from "./Icon";

interface CardEditorProps {
  card: CardSettings | null;
  config: AppConfig;
  /// Already scoped to this card by the caller via `issuesForCard` —
  /// CardEditor never resolves a card index itself, which is what makes
  /// dragging a card in the list safe: there is no stale index here for a
  /// reorder to invalidate.
  issues: ValidationIssue[];
  /// Issues scoped to this card's active-loop entry. Empty for a card outside
  /// the loop, where no dwell field is rendered.
  entryIssues: ValidationIssue[];
  /// A typed device refusal for this card's last data or scene update.
  cardError: CardError | null;
  pomodoro: PomodoroSnapshot | null;
  /// The feed behind this card, when it has one. Only ever rendered when it is in
  /// trouble — see `providerTrouble`.
  provider: ProviderSnapshot | null;
  /// The server's plugin registry, or null in local tier and before the first read.
  catalog: PluginCatalog | null;
  ownershipTier: DeviceTier | null;
  timerBusy: boolean;
  providerRefreshing: boolean;
  /** Present only for the picture card whose source was minted this session. */
  pictureAccess?: MintedImageSource | null;
  onChange: (card: CardSettings) => void;
  onConfigChange: (config: AppConfig) => void;
  onRemove: () => void;
  onTimerAction: (action: "start" | "pause" | "reset") => void;
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
  config,
  issues,
  entryIssues,
  cardError,
  pomodoro,
  provider,
  catalog,
  ownershipTier,
  timerBusy,
  providerRefreshing,
  pictureAccess = null,
  onChange,
  onConfigChange,
  onRemove,
  onTimerAction,
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
  const catalogEntry =
    card.kind === "plugin"
      ? (catalog?.plugins.find((entry) => entry.id === card.plugin_id) ?? null)
      : null;
  // What the identity statement says when there is no catalog to check against.
  // It states only what the tier makes true, never whether the plugin is installed.
  const catalogUnavailableReason =
    ownershipTier === "local"
      ? "Needs the server to render"
      : "The plugin list comes from the server";
  // The server owns plugin and picture updates, so the Mac has nothing to refresh.
  // The reason rides the same sentence as the trouble, because a disabled control
  // with no stated reason is worse than no control.
  const refreshesOnServer = card.kind === "plugin" || card.kind === "picture";
  const setAlert = (alert: CardAlert) => onChange({ ...card, alert });
  const playlist = activePlaylist(config);
  const entryIndex = playlist?.entries.findIndex((entry) => entry.card_id === card.id) ?? -1;
  const entry = entryIndex >= 0 ? playlist?.entries[entryIndex] : undefined;
  const isTimed = playlist?.advance.kind === "timed";
  const title = cardTitle(card);
  const controlName = title ? `${cardLabel(card, catalog)} — ${title}` : cardLabel(card, catalog);
  const dwellIssues = entryIssues.filter((issue) => issue.path.endsWith(".dwell_seconds"));
  const showDwell =
    entry !== undefined && (isTimed || entry.dwell_seconds !== null || dwellIssues.length > 0);

  return (
    <section className="panel" aria-labelledby="editor-heading">
      <div className="panel-heading">
        <div className="editor-title">
          <h2 id="editor-heading">{cardLabel(card, catalog)}</h2>
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
          <span>
            {refreshesOnServer ? `${trouble} This card refreshes on the server.` : trouble}
          </span>
          <button
            className="text-button"
            type="button"
            disabled={providerRefreshing || refreshesOnServer}
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

        {card.kind === "plugin" && (
          <div className="field">
            <span>Plugin</span>
            {/* The plugin id is the card's identity, fixed when the card is added.
                Rendering identity as a control implied that changing it was a safe
                edit, even though it silently turned the card into a different thing. */}
            <strong>
              {catalogEntry
                ? `${catalogEntry.display_name ?? catalogEntry.id} · ${catalogEntry.version}`
                : card.plugin_id}
            </strong>
            {/* The machine id stays visible in the small line. Without a catalog it
                is the only identity we know, and the reason says only why we could
                not verify it rather than claiming the plugin is missing. */}
            <small>
              {catalog === null
                ? `${card.plugin_id} · ${catalogUnavailableReason}`
                : catalogEntry
                  ? card.plugin_id
                  : `${card.plugin_id} · Not installed on the server`}
            </small>
            <FieldIssues issues={fieldIssues("plugin_id")} />
          </div>
        )}

        {card.kind === "picture" && (
          <>
            <label className="field">
              <span>Picture source</span>
              <select
                value={card.source_id}
                onChange={(event) => onChange({ ...card, source_id: event.currentTarget.value })}
                aria-invalid={fieldIssues("source_id").length > 0}
              >
                {!config.image_sources.some((source) => source.id === card.source_id) && (
                  <option value={card.source_id}>{`${card.source_id} · Missing source`}</option>
                )}
                {config.image_sources.map((source) => (
                  <option key={source.id} value={source.id}>
                    {source.name}
                  </option>
                ))}
              </select>
              <small>{card.source_id}</small>
              <FieldIssues issues={fieldIssues("source_id")} />
            </label>

            {pictureAccess?.source_id === card.source_id && (
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
          </>
        )}

        {card.kind === "plugin" && (
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
            {catalogEntry && (
              <small>
                {`The server fetches this plugin every ${catalogEntry.refresh_minutes} minutes.`}
              </small>
            )}
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

        {showDwell && playlist && entry && (
          <label className="field">
            <span>Stays on the panel for</span>
            <input
              type="number"
              className="numeral"
              inputMode="numeric"
              min={5}
              max={3600}
              step={1}
              value={entry.dwell_seconds ?? ""}
              placeholder={
                playlist.advance.kind === "timed"
                  ? `${playlist.advance.default_dwell_seconds} s, the loop's default`
                  : undefined
              }
              aria-label={`Stays on the panel for ${controlName}`}
              aria-invalid={dwellIssues.length > 0}
              onChange={(event) =>
                onConfigChange(
                  setEntryDwell(
                    config,
                    playlist.id,
                    entryIndex,
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
