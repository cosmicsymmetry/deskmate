import { type ReactNode, useEffect, useRef } from "react";

import { Icon } from "./Icon";

interface SettingsSheetProps {
  open: boolean;
  title: string;
  badge?: ReactNode;
  onClose: () => void;
  children: ReactNode;
}

/**
 * A native `<dialog>` opened with `showModal()`, not a div with a high z-index.
 * The platform then owns the parts that are tedious and easy to get subtly wrong:
 * the focus trap, Escape, inertness of everything behind it, and the top layer —
 * so the sheet cannot be clipped by the scrolling columns it sits over.
 */
export function SettingsSheet({ open, title, badge, onClose, children }: SettingsSheetProps) {
  const ref = useRef<HTMLDialogElement | null>(null);

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) {
      return;
    }
    if (open && !dialog.open) {
      dialog.showModal();
    } else if (!open && dialog.open) {
      dialog.close();
    }
  }, [open]);

  // Backdrop dismissal. Attached to the element rather than passed as an onClick
  // prop: a click landing on the dialog element itself *is* a backdrop click (its
  // contents are the inner div), and a11y linting rightly asks any JSX onClick for
  // a keyboard twin — which here is Escape, already handled by the platform.
  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) {
      return;
    }
    const dismiss = (event: MouseEvent) => {
      if (event.target === dialog) {
        onClose();
      }
    };
    dialog.addEventListener("click", dismiss);
    return () => dialog.removeEventListener("click", dismiss);
  }, [onClose]);

  return (
    <dialog
      className="sheet"
      ref={ref}
      aria-labelledby="sheet-heading"
      // Escape closes the dialog itself; this keeps React's `open` in step with it.
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onClose={onClose}
    >
      <div className="sheet__body">
        <div className="sheet__head">
          <h2 id="sheet-heading">{title}</h2>
          {badge}
          <button
            className="sheet__close"
            type="button"
            onClick={onClose}
            aria-label="Close settings"
          >
            <Icon name="close" />
          </button>
        </div>
        {children}
      </div>
    </dialog>
  );
}
