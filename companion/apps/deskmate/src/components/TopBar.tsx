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
 * The four status complications that used to live here were removed on purpose.
 * Link, owner, Wi-Fi and push state are facts you check twice — once while pairing
 * and once when something is broken — and paying for them with a permanent band
 * across the top of every session was the wrong trade. They live in the settings
 * sheet now; when one of them turns bad, the button carries a single dot and the
 * work column carries the sentence that explains it.
 */
export function TopBar({ attention, onOpenSettings }: TopBarProps) {
  return (
    <header className="topbar">
      <div className="wordmark">
        <span className="wordmark__mark" aria-hidden="true" />
        <span className="wordmark__text">Deskmate</span>
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
