import { cardKindName, issuesForPath } from "../lib/configDraft";
import type { CardSettings, PomodoroSnapshot, ValidationIssue } from "../lib/types";

interface WidgetEditorProps {
  widget: CardSettings | null;
  widgetIndex: number;
  issues: ValidationIssue[];
  pomodoro: PomodoroSnapshot | null;
  timerBusy: boolean;
  filePickerBusy: boolean;
  onChange: (widget: CardSettings) => void;
  onRemove: () => void;
  onTimerAction: (action: "start" | "pause" | "reset") => void;
  onChooseCalendarFile: () => void;
}

function FieldIssues({ issues }: { issues: ValidationIssue[] }) {
  if (issues.length === 0) {
    return null;
  }
  return (
    <ul className="field-errors" role="alert">
      {issues.map((issue) => (
        <li key={`${issue.path}:${issue.code}`}>{issue.message}</li>
      ))}
    </ul>
  );
}

function numberValue(value: string): number {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : 0;
}

export function WidgetEditor({
  widget,
  widgetIndex,
  issues,
  pomodoro,
  timerBusy,
  filePickerBusy,
  onChange,
  onRemove,
  onTimerAction,
  onChooseCalendarFile,
}: WidgetEditorProps) {
  if (!widget) {
    return (
      <section className="panel editor-panel" aria-labelledby="editor-heading">
        <div className="panel-heading">
          <div>
            <p className="step-label">Customize</p>
            <h2 id="editor-heading">Choose a widget</h2>
          </div>
        </div>
        <div className="empty-state empty-state--large">
          <strong>No widget selected</strong>
          <span>Select a widget on the left, or add a new one.</span>
        </div>
      </section>
    );
  }

  const path = `cards[${widgetIndex}]`;
  const pathIssues = (field: string) => issuesForPath(issues, `${path}.${field}`);

  return (
    <section className="panel editor-panel" aria-labelledby="editor-heading">
      <div className="panel-heading">
        <div>
          <p className="step-label">Customize</p>
          <h2 id="editor-heading">{cardKindName(widget.kind)}</h2>
        </div>
        <button className="text-button text-button--danger" type="button" onClick={onRemove}>
          Remove
        </button>
      </div>

      <div className="form-grid">
        <p className="canvas-note">Clean 448 × 368 canvas · no status strip</p>
        <label className="field">
          <span>Widget ID</span>
          <input value={widget.id} readOnly aria-describedby="widget-id-help" />
          <small id="widget-id-help">Stable identifier · kept when screens move</small>
        </label>

        {widget.kind === "clock" && (
          <>
            <label className="field">
              <span>Heading</span>
              <input
                value={widget.title}
                maxLength={64}
                onChange={(event) => onChange({ ...widget, title: event.currentTarget.value })}
                aria-invalid={pathIssues("title").length > 0}
              />
              <FieldIssues issues={pathIssues("title")} />
            </label>
            <label className="check-field">
              <input
                type="checkbox"
                checked={widget.show_seconds}
                onChange={(event) =>
                  onChange({ ...widget, show_seconds: event.currentTarget.checked })
                }
              />
              <span>
                <strong>Show seconds</strong>
                <small>Add seconds beside the large time.</small>
              </span>
            </label>
          </>
        )}

        {widget.kind === "pomodoro" && (
          <>
            <label className="field">
              <span>Timer label</span>
              <input
                value={widget.label}
                maxLength={64}
                onChange={(event) => onChange({ ...widget, label: event.currentTarget.value })}
                aria-invalid={pathIssues("label").length > 0}
              />
              <FieldIssues issues={pathIssues("label")} />
            </label>
            <label className="field">
              <span>Duration in minutes</span>
              <input
                type="number"
                min="1"
                max="1440"
                step="1"
                value={widget.duration_seconds / 60}
                onChange={(event) =>
                  onChange({
                    ...widget,
                    duration_seconds: Math.round(numberValue(event.currentTarget.value) * 60),
                  })
                }
                aria-invalid={pathIssues("duration_seconds").length > 0}
              />
              <FieldIssues issues={pathIssues("duration_seconds")} />
            </label>
            <fieldset className="timer-controls">
              <legend className="sr-only">Pomodoro controls</legend>
              <span>
                <strong>{pomodoro?.state ?? "idle"}</strong>
                <small>
                  {pomodoro
                    ? `${Math.ceil(pomodoro.remaining_seconds / 60)} min remaining`
                    : "Save this widget to start it"}
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

        {widget.kind === "calendar" && (
          <>
            <label className="field">
              <span>Heading</span>
              <input
                value={widget.title}
                maxLength={64}
                onChange={(event) => onChange({ ...widget, title: event.currentTarget.value })}
                aria-invalid={pathIssues("title").length > 0}
              />
              <FieldIssues issues={pathIssues("title")} />
            </label>
            <fieldset className="source-picker">
              <legend>Calendar source</legend>
              <label>
                <input
                  type="radio"
                  name={`source-${widget.id}`}
                  value="url"
                  checked={widget.source.kind === "url"}
                  onChange={() => onChange({ ...widget, source: { kind: "url", value: "" } })}
                />
                Web address
              </label>
              <label>
                <input
                  type="radio"
                  name={`source-${widget.id}`}
                  value="file"
                  checked={widget.source.kind === "file"}
                  onChange={() => onChange({ ...widget, source: { kind: "file", value: "" } })}
                />
                File on this computer
              </label>
            </fieldset>
            <div className="field">
              <label htmlFor={`calendar-source-${widget.id}`}>
                {widget.source.kind === "url" ? "ICS web address" : "ICS file"}
              </label>
              <div className={widget.source.kind === "file" ? "file-picker-row" : undefined}>
                <input
                  id={`calendar-source-${widget.id}`}
                  type={widget.source.kind === "url" ? "url" : "text"}
                  value={widget.source.value}
                  maxLength={2048}
                  placeholder={
                    widget.source.kind === "url"
                      ? "https://calendar.example/my-calendar.ics"
                      : "Choose an iCalendar file"
                  }
                  onChange={(event) =>
                    onChange({
                      ...widget,
                      source: { ...widget.source, value: event.currentTarget.value },
                    })
                  }
                  aria-invalid={pathIssues("source").length > 0}
                />
                {widget.source.kind === "file" && (
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
                {widget.source.kind === "file"
                  ? "Choose a local .ics or .ical file up to 1 MB."
                  : "Only this address is saved; downloaded events remain temporary."}
              </small>
              <FieldIssues issues={pathIssues("source")} />
            </div>
            <label className="field">
              <span>Refresh every</span>
              <select
                value={widget.refresh.kind === "interval" ? widget.refresh.minutes : 15}
                onChange={(event) =>
                  onChange({
                    ...widget,
                    refresh: {
                      kind: "interval",
                      minutes: numberValue(event.currentTarget.value),
                    },
                  })
                }
              >
                {widget.refresh.kind === "interval" &&
                  ![5, 15, 30, 60].includes(widget.refresh.minutes) && (
                    <option value={widget.refresh.minutes}>{widget.refresh.minutes} minutes</option>
                  )}
                <option value="5">5 minutes</option>
                <option value="15">15 minutes</option>
                <option value="30">30 minutes</option>
                <option value="60">1 hour</option>
              </select>
              <FieldIssues issues={pathIssues("refresh")} />
            </label>
          </>
        )}
      </div>
    </section>
  );
}
