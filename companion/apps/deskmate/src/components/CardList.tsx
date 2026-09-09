import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type DragEvent,
  type KeyboardEvent,
} from "react";

import { placeAddMenu } from "../lib/menuPlacement";

import {
  activePlaylist,
  addEntry,
  cardKindName,
  cardLabel,
  cardMoveFromKey,
  cardTitle,
  cardsContainerIssues,
  cardsOutsideLoop,
  issuesForCard,
  issuesForPath,
  loopEntries,
  MAX_CARDS,
  moveEntry,
  pluginCardFlag,
  removeEntry,
} from "../lib/configDraft";
import { providerTrouble } from "../lib/providers";
import {
  MAX_PLAYLIST_ENTRIES,
  type AddableCardKind,
  type AppConfig,
  type CardDataSnapshot,
  type CardSettings,
  type DeviceTier,
  type PluginCatalog,
  type PomodoroSnapshot,
  type ProviderSnapshot,
  type ServerCardState,
  type ValidationIssue,
} from "../lib/types";
import { FieldIssues } from "./FieldIssues";
import { Icon } from "./Icon";

/** One row in the menu's server group, built by `App` from the plugin catalog. */
export interface PluginKindOption {
  id: string;
  version: string;
  /** The manifest's `display_name`, or null when it declared none. */
  displayName: string | null;
  /** The manifest's `description`, or null when it declared none. */
  description: string | null;
  onAdd: () => void;
}

interface CardListProps {
  config: AppConfig;
  issues: ValidationIssue[];
  cardData: CardDataSnapshot[];
  pomodoros: PomodoroSnapshot[];
  providers: ProviderSnapshot[];
  pluginKinds: PluginKindOption[];
  /** The server's registry, or null in local tier and before the first read. */
  catalog: PluginCatalog | null;
  /** The server's own rows for this device's plugin cards. Empty in local tier. */
  serverCardState: ServerCardState[];
  ownershipTier: DeviceTier | null;
  selectedCardId: string | null;
  onSelect: (cardId: string) => void;
  onAdd: (kind: AddableCardKind) => void;
  onChange: (config: AppConfig) => void;
  onRemove: (cardId: string) => void;
}

const addableKinds: { kind: AddableCardKind; description: string }[] = [
  { kind: "clock", description: "Time and date" },
  { kind: "pomodoro", description: "Focus timer" },
  { kind: "calendar", description: "Upcoming events" },
  { kind: "weather", description: "Local conditions" },
  { kind: "json-feed", description: "Custom JSON data" },
  { kind: "rss", description: "Headlines feed" },
];

function fieldText(data: CardDataSnapshot | undefined, key: string): string | null {
  const field = data?.fields.find((candidate) => candidate.key === key);
  if (!field) {
    return null;
  }
  return field.value.kind === "text" ? field.value.value : String(field.value.value);
}

/** The live fact that makes a tile a complication rather than a list row. */
function tileValue(
  card: CardSettings,
  data: CardDataSnapshot | undefined,
  pomodoro: PomodoroSnapshot | undefined,
  now: Date,
  timezone: string,
  pluginHero: string | null,
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
    case "weather":
    case "json-feed":
      return fieldText(data, "hero") ?? "—";
    case "calendar":
    case "rss": {
      const rowCount = (data?.fields ?? []).filter(
        (field) =>
          /^row\d+_title$/.test(field.key) &&
          field.value.kind === "text" &&
          field.value.value.trim() !== "",
      ).length;
      return rowCount > 0 ? String(rowCount) : "—";
    }
    case "plugin":
      // The server's evaluated `summary`, or the same em dash a weather card shows
      // before its first fetch. A tile owns one fact; it does not narrate.
      return pluginHero ?? "—";
  }
}

/** Every control that acts on one card names template and typed title together. */
function controlLabel(card: CardSettings, catalog: PluginCatalog | null): string {
  const title = cardTitle(card);
  return title ? `${cardLabel(card, catalog)} — ${title}` : cardLabel(card, catalog);
}

export function CardList({
  config,
  issues,
  cardData,
  pomodoros,
  providers,
  pluginKinds,
  catalog,
  serverCardState,
  ownershipTier,
  selectedCardId,
  onSelect,
  onAdd,
  onChange,
  onRemove,
}: CardListProps) {
  const playlist = activePlaylist(config);
  const entries = loopEntries(config);
  const outsideCards = cardsOutsideLoop(config);
  const playlistIndex = config.playlists.findIndex(
    (candidate) => candidate.id === config.active_playlist_id,
  );
  const cardsFull = config.cards.length >= MAX_CARDS;
  const loopFull = (playlist?.entries.length ?? 0) >= MAX_PLAYLIST_ENTRIES;
  const atCapacity = cardsFull || loopFull || !playlist;
  const capacityDescription = cardsFull
    ? `The limit is ${MAX_CARDS} cards.`
    : loopFull
      ? `The limit is ${MAX_PLAYLIST_ENTRIES} in the loop.`
      : !playlist
        ? "An active loop is required."
        : null;
  const containerIssues = [
    ...cardsContainerIssues(issues),
    ...(playlistIndex < 0
      ? []
      : issues.filter((issue) => issue.path === `playlists[${playlistIndex}].entries`)),
  ];
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
  const entryOrder = entries.map(({ entry }) => entry.card_id).join("\u0000");
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

  const choosePlugin = (plugin: PluginKindOption) => {
    pendingNewCardIdsRef.current = new Set(config.cards.map((card) => card.id));
    plugin.onAdd();
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
      const count = addableKinds.length + pluginKinds.length;
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
    const current = activePlaylist(config);
    if (!current) {
      return;
    }
    const from = current.entries.findIndex((entry) => entry.card_id === cardId);
    const to = current.entries.findIndex((entry) => entry.card_id === targetCardId);
    if (from >= 0 && to >= 0) {
      const next = moveEntry(config, current.id, from, to);
      if (next !== config) {
        pendingFocusCardIdRef.current = cardId;
        onChange(next);
      }
    }
  };

  const moveBy = (cardId: string, delta: -1 | 1) => {
    const current = activePlaylist(config);
    const from = current?.entries.findIndex((entry) => entry.card_id === cardId) ?? -1;
    const target = current?.entries[from + delta];
    if (target) {
      moveTo(cardId, target.card_id);
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

  const renderCardTile = (card: CardSettings, inLoop: boolean, entryIndex?: number) => {
    const cardIssues = issuesForCard(issues, config, card.id);
    const entryIssues =
      inLoop && playlistIndex >= 0 && entryIndex !== undefined
        ? issuesForPath(issues, `playlists[${playlistIndex}].entries[${entryIndex}]`)
        : [];
    const tileIssues = [...cardIssues, ...entryIssues];
    const hasAlert = card.alert.kind !== "none";
    const data = cardData.find((candidate) => candidate.card_id === card.id);
    const pomodoro = pomodoros.find((candidate) => candidate.widget_id === card.id);
    const stale =
      providerTrouble(providers.find((candidate) => candidate.widget_id === card.id)) !== null;
    const index = inLoop ? (entryIndex ?? -1) : -1;
    const label = controlLabel(card, catalog);
    // The server's row for this card, when it has one, and the word that explains a
    // bare plugin id when the catalog or the tier cannot resolve it.
    const serverState = serverCardState.find((row) => row.card_id === card.id) ?? null;
    const pluginFlag = pluginCardFlag(card, catalog, ownershipTier);
    return (
      <li
        key={inLoop ? `loop:${entryIndex}:${card.id}` : `outside:${card.id}`}
        draggable={inLoop}
        className={`card-tile${inLoop ? "" : " card-tile--outside"}${
          selectedCardId === card.id ? " is-selected" : ""
        }${tileIssues.length > 0 ? " has-issue" : ""}${
          draggedCardId === card.id ? " is-dragging" : ""
        }`}
        onDragStart={inLoop ? (event) => onDragStart(event, card.id) : undefined}
        onDragEnd={inLoop ? () => setDraggedCardId(null) : undefined}
        onDragOver={inLoop ? (event) => event.preventDefault() : undefined}
        onDrop={inLoop ? (event) => onDrop(event, card.id) : undefined}
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
          onKeyDown={inLoop ? (event) => onTileKeyDown(event, card.id) : undefined}
        >
          <span className="tile-label">{cardLabel(card, catalog)}</span>
          <strong className="card-tile__value numeral">
            {tileValue(
              card,
              data,
              pomodoro,
              now,
              config.preferences.timezone,
              serverState?.hero ?? null,
            )}
          </strong>
          {cardTitle(card) && <span className="card-tile__name">{cardTitle(card)}</span>}
        </button>
        <span className="card-tile__flags">
          {!inLoop && <span className="flag">not in loop</span>}
          {pluginFlag && <span className="flag">{pluginFlag}</span>}
          {stale && <span className="flag flag--stale">stale</span>}
          {hasAlert && <span className="flag flag--alert">alerts</span>}
          {!inLoop && (
            <button
              type="button"
              className="text-button card-tile__join"
              aria-label={`Add ${label} to the loop`}
              disabled={loopFull || !playlist}
              onClick={() => {
                if (playlist) {
                  pendingFocusCardIdRef.current = card.id;
                  onChange(addEntry(config, playlist.id, card.id));
                }
              }}
            >
              Add to loop
            </button>
          )}
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
        {inLoop && (
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
        )}
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
        {entries.map(({ index, entry, card }) => {
          if (card) {
            return renderCardTile(card, true, index);
          }
          const entryIssues =
            playlistIndex < 0
              ? []
              : issuesForPath(issues, `playlists[${playlistIndex}].entries[${index}]`);
          return (
            <li
              key={`missing:${entry.card_id}:${index}`}
              className={`card-tile${entryIssues.length > 0 ? " has-issue" : ""}`}
            >
              <div className="card-tile__body">
                <span className="tile-label">Missing card</span>
                <strong className="card-tile__value numeral">—</strong>
              </div>
              <button
                type="button"
                className="card-tile__remove"
                aria-label="Remove Missing card"
                title="Remove"
                onClick={() => playlist && onChange(removeEntry(config, playlist.id, index))}
              >
                <Icon name="close" />
              </button>
              <FieldIssues issues={entryIssues} className="card-tile__issues" />
            </li>
          );
        })}
        {outsideCards.map((card) => renderCardTile(card, false))}
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
              <small>Built in, or a plugin</small>
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
              {pluginKinds.length > 0 && (
                <fieldset className="menu__group">
                  <legend className="tile-label menu__label">Plugins on the server</legend>
                  {pluginKinds.map((plugin, pluginIndex) => {
                    const index = addableKinds.length + pluginIndex;
                    const choose = () => choosePlugin(plugin);
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
                        key={plugin.id}
                        onClick={choose}
                        onKeyDown={(event) => onMenuKeyDown(event, index, choose)}
                      >
                        <span>
                          <strong>{plugin.displayName ?? plugin.id}</strong>
                          <small>{plugin.description ?? `Plugin · ${plugin.version}`}</small>
                        </span>
                      </button>
                    );
                  })}
                </fieldset>
              )}
            </div>
          )}
        </li>
      </ul>
    </section>
  );
}
