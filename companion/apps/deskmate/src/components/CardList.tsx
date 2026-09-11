import {
  type CSSProperties,
  type DragEvent,
  type KeyboardEvent,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import {
  cardKindName,
  cardLabel,
  cardMoveFromKey,
  cardsContainerIssues,
  cardTitle,
  issuesForCard,
  issuesForPath,
  loopEntries,
  MAX_CARDS,
  moveEntry,
} from "../lib/configDraft";
import { placeAddMenu } from "../lib/menuPlacement";
import {
  type AddableCardKind,
  type AppConfig,
  type CardSettings,
  type DeviceTier,
  type PomodoroSnapshot,
  type ValidationIssue,
} from "../lib/types";
import { FieldIssues } from "./FieldIssues";
import { Icon } from "./Icon";

interface CardListProps {
  config: AppConfig;
  issues: ValidationIssue[];
  pomodoros: PomodoroSnapshot[];
  ownershipTier: DeviceTier | null;
  selectedCardId: string | null;
  onSelect: (cardId: string) => void;
  onAdd: (kind: AddableCardKind) => void;
  onAddPicture?: () => void;
  onChange: (config: AppConfig) => void;
  onRemove: (cardId: string) => void;
}

const addableKinds: { kind: AddableCardKind; description: string }[] = [
  { kind: "clock", description: "Time and date" },
  { kind: "pomodoro", description: "Focus timer" },
];

/** The live fact that makes a tile a complication rather than a list row. */
function tileValue(
  card: CardSettings,
  pomodoro: PomodoroSnapshot | undefined,
  now: Date,
  timezone: string,
): string {
  switch (card.kind) {
    case "clock":
      try {
        return new Intl.DateTimeFormat("en-GB", {
          hour: "2-digit",
          minute: "2-digit",
          timeZone: timezone,
        }).format(now);
      } catch {
        return "--:--";
      }
    case "pomodoro": {
      const seconds = pomodoro?.remaining_seconds ?? card.duration_seconds;
      return `${Math.floor(seconds / 60)}:${String(Math.floor(seconds % 60)).padStart(2, "0")}`;
    }
    case "picture":
      return "PNG";
  }
}

/** Every control that acts on one card names template and typed title together. */
function controlLabel(card: CardSettings): string {
  const title = cardTitle(card);
  return title ? `${cardLabel(card)} — ${title}` : cardLabel(card);
}

export function CardList({
  config,
  issues,
  pomodoros,
  ownershipTier,
  selectedCardId,
  onSelect,
  onAdd,
  onAddPicture = () => {},
  onChange,
  onRemove,
}: CardListProps) {
  const entries = loopEntries(config);
  // One list, one bound: adding a card IS joining the loop.
  const atCapacity = config.cards.length >= MAX_CARDS;
  const capacityDescription = atCapacity ? `The limit is ${MAX_CARDS} cards.` : null;
  const containerIssues = cardsContainerIssues(issues);
  const [now, setNow] = useState(() => new Date());
  const [draggedCardId, setDraggedCardId] = useState<string | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const [menuAlignEnd, setMenuAlignEnd] = useState(false);
  // Viewport coordinates, because the menu is `position: fixed` — see
  // `menuPlacement.ts` for why it cannot be positioned within the work column.
  const [menuStyle, setMenuStyle] = useState<CSSProperties>({});
  const menuRef = useRef<HTMLDivElement>(null);
  const menuRootRef = useRef<HTMLLIElement>(null);
  const addButtonRef = useRef<HTMLButtonElement>(null);
  const menuItemRefs = useRef<HTMLButtonElement[]>([]);
  const tileBodyRefs = useRef(new Map<string, HTMLButtonElement>());
  const pendingFocusCardIdRef = useRef<string | null>(null);
  const pendingNewCardIdsRef = useRef<Set<string> | null>(null);
  const hasClock = config.cards.some((card) => card.kind === "clock");
  const entryOrder = entries.map(({ card }) => card.id).join("\u0000");
  const pendingNewCardId = pendingNewCardIdsRef.current
    ? (config.cards.find((card) => !pendingNewCardIdsRef.current?.has(card.id))?.id ?? null)
    : null;

  useEffect(() => {
    if (!hasClock) {
      return;
    }
    const interval = window.setInterval(() => setNow(new Date()), 1000);
    return () => window.clearInterval(interval);
  }, [hasClock]);

  useEffect(() => {
    if (!menuOpen) {
      return;
    }
    const closeOnOutsideClick = (event: MouseEvent) => {
      if (!menuRootRef.current?.contains(event.target as Node)) {
        setMenuOpen(false);
      }
    };
    document.addEventListener("mousedown", closeOnOutsideClick);
    return () => document.removeEventListener("mousedown", closeOnOutsideClick);
  }, [menuOpen]);

  const positionMenu = useCallback(() => {
    if (!menuRootRef.current) {
      return;
    }
    const trigger = menuRootRef.current.getBoundingClientRect();
    const viewport = {
      width: document.documentElement.clientWidth,
      height: window.innerHeight,
    };
    // `scrollHeight` is the menu's content height whether or not a cap is
    // already clipping it, so this stays correct on every repositioning.
    const contentHeight = menuRef.current?.scrollHeight ?? 0;
    const placement = placeAddMenu(trigger, viewport, contentHeight);
    setMenuAlignEnd(placement.alignEnd);
    setMenuStyle({
      top: placement.top,
      maxHeight: placement.maxHeight,
      ...(placement.alignEnd ? { right: viewport.width - trigger.right } : { left: trigger.left }),
    });
  }, []);

  useLayoutEffect(() => {
    if (menuOpen) {
      positionMenu();
    }
  }, [menuOpen, positionMenu]);

  // A fixed menu keeps the coordinates it was given, so it would drift away from
  // its trigger when the column scrolls or the window resizes underneath it.
  // It is repositioned rather than closed: closing looks right until you notice
  // that opening the menu focuses its first item, which scrolls that item into
  // view — a scroll the menu itself caused, which would then close it again the
  // instant it opened.
  useEffect(() => {
    if (!menuOpen) {
      return;
    }
    const reposition = () => positionMenu();
    window.addEventListener("resize", reposition);
    window.addEventListener("scroll", reposition, true);
    return () => {
      window.removeEventListener("resize", reposition);
      window.removeEventListener("scroll", reposition, true);
    };
  }, [menuOpen, positionMenu]);

  useLayoutEffect(() => {
    const previousCardIds = pendingNewCardIdsRef.current;
    const cardId = pendingFocusCardIdRef.current ?? pendingNewCardId;
    if (!cardId) {
      if (previousCardIds && !menuOpen) {
        addButtonRef.current?.focus();
        pendingNewCardIdsRef.current = null;
      }
      return;
    }
    const tileBody = entryOrder ? tileBodyRefs.current.get(cardId) : undefined;
    if (tileBody) {
      tileBody.focus();
      pendingFocusCardIdRef.current = null;
      pendingNewCardIdsRef.current = null;
    }
  }, [entryOrder, pendingNewCardId, menuOpen]);

  const closeMenu = (restoreFocus: boolean) => {
    setMenuOpen(false);
    if (restoreFocus) {
      queueMicrotask(() => addButtonRef.current?.focus());
    }
  };

  const openMenu = () => {
    if (atCapacity) {
      return;
    }
    menuItemRefs.current = [];
    setMenuOpen(true);
    queueMicrotask(() => menuItemRefs.current[0]?.focus());
  };

  const chooseBuiltIn = (kind: AddableCardKind) => {
    pendingNewCardIdsRef.current = new Set(config.cards.map((card) => card.id));
    onAdd(kind);
    closeMenu(false);
  };

  const choosePicture = () => {
    pendingNewCardIdsRef.current = new Set(config.cards.map((card) => card.id));
    onAddPicture();
    closeMenu(false);
  };

  const onMenuKeyDown = (
    event: KeyboardEvent<HTMLButtonElement>,
    index: number,
    choose: () => void,
  ) => {
    if (event.key === "Escape") {
      event.preventDefault();
      closeMenu(true);
      return;
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const direction = event.key === "ArrowDown" ? 1 : -1;
      const count = addableKinds.length + 1;
      menuItemRefs.current[(index + direction + count) % count]?.focus();
      return;
    }
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      choose();
    }
  };

  /** Resolve both indexes from the current draft at the moment an action fires. */
  const moveTo = (cardId: string, targetCardId: string) => {
    const from = config.cards.findIndex((card) => card.id === cardId);
    const to = config.cards.findIndex((card) => card.id === targetCardId);
    if (from >= 0 && to >= 0) {
      const next = moveEntry(config, from, to);
      if (next !== config) {
        pendingFocusCardIdRef.current = cardId;
        onChange(next);
      }
    }
  };

  const moveBy = (cardId: string, delta: -1 | 1) => {
    const from = config.cards.findIndex((card) => card.id === cardId);
    const target = from >= 0 ? config.cards[from + delta] : undefined;
    if (target) {
      moveTo(cardId, target.id);
    }
  };

  const onTileKeyDown = (event: KeyboardEvent<HTMLButtonElement>, cardId: string) => {
    const delta = cardMoveFromKey(event.key, event.altKey);
    if (delta === 0) {
      return;
    }
    event.preventDefault();
    moveBy(cardId, delta);
  };

  const onDragStart = (event: DragEvent<HTMLLIElement>, cardId: string) => {
    setDraggedCardId(cardId);
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData("text/plain", cardId);
  };

  const onDrop = (event: DragEvent<HTMLLIElement>, targetCardId: string) => {
    event.preventDefault();
    const sourceCardId = draggedCardId ?? event.dataTransfer.getData("text/plain");
    if (sourceCardId) {
      moveTo(sourceCardId, targetCardId);
    }
    setDraggedCardId(null);
  };

  // Every card is in the loop since schema v10, so there is no longer a tile
  // state for one that is not, and a card's own issues already cover its dwell.
  const renderCardTile = (card: CardSettings, index: number) => {
    const tileIssues = issuesForCard(issues, config, card.id);
    const hasAlert = card.alert.kind !== "none";
    const pomodoro = pomodoros.find((candidate) => candidate.card_id === card.id);
    const label = controlLabel(card);
    const pictureFlag = card.kind === "picture" && ownershipTier === "local" ? "needs the server" : null;
    return (
      <li
        key={`loop:${index}:${card.id}`}
        draggable
        className={`card-tile${
          selectedCardId === card.id ? " is-selected" : ""
        }${tileIssues.length > 0 ? " has-issue" : ""}${
          draggedCardId === card.id ? " is-dragging" : ""
        }`}
        onDragStart={(event) => onDragStart(event, card.id)}
        onDragEnd={() => setDraggedCardId(null)}
        onDragOver={(event) => event.preventDefault()}
        onDrop={(event) => onDrop(event, card.id)}
      >
        <button
          ref={(element) => {
            if (element) {
              tileBodyRefs.current.set(card.id, element);
            } else {
              tileBodyRefs.current.delete(card.id);
            }
          }}
          type="button"
          className="card-tile__body"
          aria-pressed={selectedCardId === card.id}
          onClick={() => onSelect(card.id)}
          onKeyDown={(event) => onTileKeyDown(event, card.id)}
        >
          <span className="tile-label">{cardLabel(card)}</span>
          <strong className="card-tile__value numeral">
            {tileValue(card, pomodoro, now, config.preferences.timezone)}
          </strong>
          {cardTitle(card) && <span className="card-tile__name">{cardTitle(card)}</span>}
        </button>
        <span className="card-tile__flags">
          {pictureFlag && <span className="flag">{pictureFlag}</span>}
          {hasAlert && <span className="flag flag--alert">alerts</span>}
        </span>
        <button
          type="button"
          className="card-tile__remove"
          aria-label={
            config.cards.length === 1
              ? `Remove ${label} (keep at least one card)`
              : `Remove ${label}`
          }
          title={config.cards.length === 1 ? "This is your only card." : "Remove"}
          disabled={config.cards.length === 1}
          onClick={() => onRemove(card.id)}
        >
          <Icon name="close" />
        </button>
        <span className="card-tile__moves">
            <button
              type="button"
              aria-label={`Move ${label} earlier`}
              disabled={index === 0}
              onClick={() => moveBy(card.id, -1)}
            >
              <Icon name="left" />
            </button>
            <button
              type="button"
              aria-label={`Move ${label} later`}
              disabled={index === entries.length - 1}
              onClick={() => moveBy(card.id, 1)}
            >
              <Icon name="right" />
            </button>
          </span>
        <FieldIssues issues={tileIssues} className="card-tile__issues" />
      </li>
    );
  };

  return (
    <section className="panel library" aria-labelledby="card-list-heading">
      <div className="panel-heading">
        <div>
          <h2 id="card-list-heading">The loop</h2>
        </div>
        <div className="panel-heading__right">
          <span className="keyboard-hint">Drag to reorder · ⌥ ← →</span>
          <span className="count-badge numeral">
            {config.cards.length}/{MAX_CARDS}
          </span>
        </div>
      </div>

      <FieldIssues issues={containerIssues} />

      <ul className="card-grid" aria-label="Cards, loop order first">
        {entries.map(({ index, card }) => renderCardTile(card, index))}
        <li
          className="card-tile card-tile--add"
          ref={menuRootRef}
          onBlur={(event) => {
            // Only a blur that lands somewhere outside closes the menu. WebKit
            // does not move focus to a `<button>` on mousedown -- a macOS
            // convention Chrome does not share -- so pressing the mouse on a
            // menu item blurs the focused item with a null `relatedTarget`.
            // Treating that as focus leaving unmounted the menu between
            // mousedown and click, so the click never landed on the item and no
            // card of any kind could be added. Focus going nowhere is not focus
            // leaving; a click genuinely outside is caught by the document
            // mousedown listener above.
            if (event.relatedTarget && !menuRootRef.current?.contains(event.relatedTarget)) {
              closeMenu(false);
            }
          }}
        >
          <button
            ref={addButtonRef}
            type="button"
            className="card-tile__add"
            aria-haspopup="menu"
            aria-expanded={menuOpen}
            aria-controls="add-card-menu"
            aria-describedby={capacityDescription ? "add-card-capacity" : undefined}
            disabled={atCapacity}
            onClick={() => (menuOpen ? closeMenu(false) : openMenu())}
            onKeyDown={(event) => {
              if (event.key === "Escape" && menuOpen) {
                event.preventDefault();
                closeMenu(true);
              }
            }}
          >
            <span className="card-tile__plus">
              <Icon name="plus" />
            </span>
            <span>
              <strong>Add a card</strong>
              <small>Built in or picture</small>
            </span>
          </button>
          {capacityDescription && (
            <span className="sr-only" id="add-card-capacity">
              {capacityDescription}
            </span>
          )}
          {menuOpen && (
            <div
              ref={menuRef}
              className={`menu${menuAlignEnd ? " menu--end" : ""}`}
              id="add-card-menu"
              role="menu"
              aria-label="Add a card"
              style={menuStyle}
            >
              <fieldset className="menu__group">
                <legend className="tile-label menu__label">Built in</legend>
                {addableKinds.map(({ kind, description }, index) => {
                  const choose = () => chooseBuiltIn(kind);
                  return (
                    <button
                      ref={(element) => {
                        if (element) {
                          menuItemRefs.current[index] = element;
                        }
                      }}
                      type="button"
                      className="menu__item"
                      role="menuitem"
                      key={kind}
                      onClick={choose}
                      onKeyDown={(event) => onMenuKeyDown(event, index, choose)}
                    >
                      <span>
                        <strong>{cardKindName(kind)}</strong>
                        <small>{description}</small>
                      </span>
                    </button>
                  );
                })}
              </fieldset>
              <fieldset className="menu__group">
                <legend className="tile-label menu__label">Pictures</legend>
                <button
                  ref={(element) => {
                    if (element) {
                      menuItemRefs.current[addableKinds.length] = element;
                    }
                  }}
                  type="button"
                  className="menu__item"
                  role="menuitem"
                  onClick={choosePicture}
                  onKeyDown={(event) => onMenuKeyDown(event, addableKinds.length, choosePicture)}
                >
                  <span>
                    <strong>Picture</strong>
                    <small>A PNG pushed from anywhere</small>
                  </span>
                </button>
              </fieldset>
            </div>
          )}
        </li>
      </ul>
    </section>
  );
}
