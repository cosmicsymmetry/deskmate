import {
  cardKindName,
  cardName,
  cardsContainerIssues,
  issuesForCard,
  libraryCards,
} from "../lib/configDraft";
import type { AppConfig, CardKind, ValidationIssue } from "../lib/types";

const MAX_CARDS = 8;

interface CardListProps {
  config: AppConfig;
  issues: ValidationIssue[];
  selectedCardId: string | null;
  onSelect: (cardId: string) => void;
  onAdd: (kind: CardKind) => void;
  onRemove: (cardId: string) => void;
}

const addableKinds: { kind: CardKind; glyph: string; description: string }[] = [
  { kind: "clock", glyph: "09:41", description: "Time and date" },
  { kind: "pomodoro", glyph: "25", description: "Focus timer" },
  { kind: "calendar", glyph: "≡", description: "Upcoming events" },
  { kind: "weather", glyph: "☀", description: "Local conditions" },
  { kind: "json-feed", glyph: "{ }", description: "Custom JSON data" },
  { kind: "rss", glyph: "⟢", description: "Headlines feed" },
];

const glyphForKind = new Map(addableKinds.map(({ kind, glyph }) => [kind, glyph]));

function FieldIssues({ issues }: { issues: ValidationIssue[] }) {
  if (issues.length === 0) {
    return null;
  }
  return (
    <ul className="field-errors card-row__issues" role="alert">
      {issues.map((issue) => (
        <li key={`${issue.path}:${issue.code}`}>{issue.message}</li>
      ))}
    </ul>
  );
}

export function CardList({
  config,
  issues,
  selectedCardId,
  onSelect,
  onAdd,
  onRemove,
}: CardListProps) {
  const cards = libraryCards(config);
  const atCapacity = cards.length >= MAX_CARDS;
  const containerIssues = cardsContainerIssues(issues);
  const usedCardIds = new Set(
    config.playlists.flatMap((playlist) => playlist.entries.map((entry) => entry.card_id)),
  );

  return (
    <section className="panel card-list-panel" aria-labelledby="card-list-heading">
      <div className="panel-heading">
        <div>
          <p className="step-label">Library</p>
          <h2 id="card-list-heading">Every card</h2>
        </div>
        <span className="count-badge numeral" id="card-capacity">
          {cards.length}/{MAX_CARDS}
        </span>
      </div>

      {containerIssues.length > 0 && (
        <ul className="field-errors card-list-issues" role="alert">
          {containerIssues.map((issue) => (
            <li key={`${issue.path}:${issue.code}`}>{issue.message}</li>
          ))}
        </ul>
      )}

      {cards.length === 0 ? (
        <div className="empty-state">
          <strong>Your library is empty</strong>
          <span>Add a card below, then place it in a playlist.</span>
        </div>
      ) : (
        <ul className="card-list library-list" aria-label="Card library">
          {cards.map((card) => {
            const cardIssues = issuesForCard(issues, config, card.id);
            const hasAlert = card.alert.kind !== "none";
            const isUnused = !usedCardIds.has(card.id);
            return (
              <li
                key={card.id}
                className={`card-row library-row${selectedCardId === card.id ? " is-selected" : ""}`}
              >
                <span className="library-row__glyph numeral" aria-hidden="true">
                  {glyphForKind.get(card.kind)}
                </span>
                <button
                  type="button"
                  className="card-row__body"
                  aria-pressed={selectedCardId === card.id}
                  onClick={() => onSelect(card.id)}
                >
                  <strong>{cardName(card)}</strong>
                  <small>{cardKindName(card.kind)}</small>
                </button>
                <span className="card-row__badges">
                  {hasAlert && <span className="status-badge">alerts</span>}
                  {isUnused && <span className="status-badge status-badge--quiet">unused</span>}
                </span>
                <button
                  type="button"
                  className="card-row__remove text-button text-button--danger"
                  aria-label={
                    cards.length === 1
                      ? `Remove ${cardName(card)} (keep at least one card)`
                      : `Remove ${cardName(card)}`
                  }
                  title={cards.length === 1 ? "This is your only card." : undefined}
                  disabled={cards.length === 1}
                  onClick={() => onRemove(card.id)}
                >
                  Remove
                </button>
                <FieldIssues issues={cardIssues} />
              </li>
            );
          })}
        </ul>
      )}

      <fieldset className="card-add">
        <legend className="sr-only">Add a card</legend>
        {addableKinds.map(({ kind, glyph, description }) => (
          <button
            className="add-card"
            type="button"
            key={kind}
            onClick={() => onAdd(kind)}
            disabled={atCapacity}
            aria-describedby={atCapacity ? "card-capacity" : undefined}
          >
            <span className="add-card__glyph numeral" aria-hidden="true">
              {glyph}
            </span>
            <span>
              <strong>Add {cardKindName(kind)}</strong>
              <small>{description}</small>
            </span>
            <span aria-hidden="true">+</span>
          </button>
        ))}
      </fieldset>
    </section>
  );
}
