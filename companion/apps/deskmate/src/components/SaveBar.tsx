import type { ApiError, DeviceTier, DraftValidation } from "../lib/types";
import { Icon } from "./Icon";

export type ValidationState =
  | { kind: "idle"; result: DraftValidation }
  | { kind: "checking"; result: DraftValidation }
  | { kind: "ready"; result: DraftValidation }
  | { kind: "error"; result: DraftValidation; error: ApiError };

export type SaveState =
  | { kind: "idle" }
  | { kind: "saving" }
  | { kind: "saved"; message: string }
  | { kind: "error"; error: ApiError };

const MAX_RENDERED_SAVE_ISSUES = 5;

function SaveError({ error }: { error: ApiError }) {
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

interface SaveBarProps {
  validation: ValidationState;
  saveState: SaveState;
  ownershipTier: DeviceTier | null;
  dirty: boolean;
  /** The sheet is modal, so the page's primary save bar is unreachable while it is
   *  open. Rather than let a preference edit strand itself behind a dialog, the
   *  sheet renders this same bar — one implementation, so the two can never
   *  disagree about whether the draft is saveable. */
  variant?: "page" | "sheet";
  onSave: () => void;
}

export function SaveBar({
  validation,
  saveState,
  ownershipTier,
  dirty,
  variant = "page",
  onSave,
}: SaveBarProps) {
  const serverOwned = ownershipTier === "networked";
  const blocked =
    !dirty ||
    !serverOwned ||
    validation.kind !== "ready" ||
    !validation.result.valid ||
    saveState.kind === "saving";

  return (
    <footer className={variant === "sheet" ? "save-bar save-bar--sheet" : "save-bar"}>
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
        {!serverOwned && saveState.kind === "idle" && (
          <span className="save-error">
            Server ownership is unavailable. Check the device link before saving.
          </span>
        )}
        {validation.kind === "ready" &&
          validation.result.valid &&
          saveState.kind === "idle" &&
          serverOwned && <span>{dirty ? "Unsaved changes" : "Everything is up to date"}</span>}
      </div>
      <button className="button button--primary" type="button" disabled={blocked} onClick={onSave}>
        {saveState.kind === "saving"
          ? "Saving to server…"
          : serverOwned
            ? "Save to server"
            : "Ownership unavailable"}
      </button>
    </footer>
  );
}
