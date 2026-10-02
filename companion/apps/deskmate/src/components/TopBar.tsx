import { PRODUCT_NAME } from "../lib/product";
import { Icon } from "./Icon";

interface TopBarProps {
  /** True when something in the sheet wants a look: no ownership, Wi-Fi down, a
   *  protocol the app cannot speak. Nominal state says nothing at all. */
  attention: boolean;
  onOpenSettings: () => void;
}

/**
 * The whole chrome: a wordmark and the one door out of the working view.
 *
 * Link, owner, Wi-Fi and push state live in the settings sheet because they are
 * pairing-and-troubleshooting facts rather than working-view controls. When one
 * needs attention, the button carries a dot and the work column states the cause.
 */
export function TopBar({ attention, onOpenSettings }: TopBarProps) {
  return (
    <header className="topbar">
      <div className="wordmark">
        <span className="wordmark__mark" aria-hidden="true" />
        <span className="wordmark__text">{PRODUCT_NAME}</span>
      </div>

      <button
        className={`topbar__settings${attention ? " has-attention" : ""}`}
        type="button"
        onClick={onOpenSettings}
      >
        <Icon name="settings" />
        <span>Settings</span>
        {attention && <span className="topbar__dot" aria-hidden="true" />}
        {attention && <span className="sr-only">Needs attention</span>}
      </button>
    </header>
  );
}
