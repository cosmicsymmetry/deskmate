import { useEffect, useState } from "react";

import {
  cardKindName,
  cardLabel,
  cardTitle,
  cardsContainerIssues,
  issuesForCard,
  libraryCards,
} from "../lib/configDraft";
import { providerTrouble } from "../lib/providers";
import { Icon } from "./Icon";
import type {
  AppConfig,
  CardDataSnapshot,
  CardKind,
  CardSettings,
  PomodoroSnapshot,
  ProviderSnapshot,
  ValidationIssue,
} from "../lib/types";

const MAX_CARDS = 8;

interface CardListProps {
  config: AppConfig;
  issues: ValidationIssue[];
  cardData: CardDataSnapshot[];
  pomodoros: PomodoroSnapshot[];
  providers: ProviderSnapshot[];
  selectedCardId: string | null;
  onSelect: (cardId: string) => void;
  onAdd: (kind: CardKind) => void;
  onRemove: (cardId: string) => void;
}

const addableKinds: { kind: CardKind; description: string }[] = [
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

/**
 * The live face of a card, which is what makes these tiles complications rather
 * than a list: each shows the thing its card is currently for. A card with no data
 * yet says so with an em dash rather than borrowing a plausible-looking number.
 */
function tileValue(
  card: CardSettings,
  data: CardDataSnapshot | undefined,
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
    case "weather":
    case "json-feed":
      return fieldText(data, "hero") ?? "—";
    case "calendar":
    case "rss": {
      const rows = (data?.fields ?? []).filter((field) => field.key.startsWith("row"));
      return rows.length > 0 ? String(rows.length) : "—";
    }
  }
}

/** Two cards can share a template, so a remove control names the owner's title too
 *  when there is one — otherwise a screen reader hears "Remove Weather" twice. */
function removeLabel(card: CardSettings): string {
  const title = cardTitle(card);
  return title ? `${cardLabel(card)} — ${title}` : cardLabel(card);
}

function FieldIssues({ issues }: { issues: ValidationIssue[] }) {
  if (issues.length === 0) {
    return null;
  }
  return (
    <ul className="field-errors card-tile__issues" role="alert">
      {issues.map((issue) => (
        <li key={`${issue.path}:${issue.code}`}>{issue.message}</li>
      ))}
    </ul>
  );
}

export function CardList({
  config,
  issues,
  cardData,
  pomodoros,
  providers,
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

  // A face whose clock does not move is a picture of a face. One tick a second is
  // enough, and it is the only timer this component owns.
  const [now, setNow] = useState(() => new Date());
  const hasClock = cards.some((card) => card.kind === "clock");
  useEffect(() => {
    if (!hasClock) {
      return;
    }
    const interval = window.setInterval(() => setNow(new Date()), 1000);
    return () => window.clearInterval(interval);
  }, [hasClock]);

  return (
    <section className="panel library" aria-labelledby="card-list-heading">
      <div className="panel-heading">
        <div>
          <h2 id="card-list-heading">Card library</h2>
        </div>
        <span className="count-badge numeral" id="card-capacity">
          {cards.length}/{MAX_CARDS}
        </span>
      </div>

      {containerIssues.length > 0 && (
        <ul className="field-errors" role="alert">
          {containerIssues.map((issue) => (
            <li key={`${issue.path}:${issue.code}`}>{issue.message}</li>
          ))}
        </ul>
      )}

      {cards.length === 0 ? (
        <div className="empty-state">
          <strong>No cards yet</strong>
          <span>Add one below, then place it in a playlist.</span>
        </div>
      ) : (
        <ul className="card-grid" aria-label="Card library">
          {cards.map((card) => {
            const cardIssues = issuesForCard(issues, config, card.id);
            const hasAlert = card.alert.kind !== "none";
            const isUnused = !usedCardIds.has(card.id);
            const data = cardData.find((candidate) => candidate.card_id === card.id);
            const pomodoro = pomodoros.find((candidate) => candidate.widget_id === card.id);
            // The single fact the old data-sources panel carried that was worth
            // keeping: which card's number you should not trust right now.
            const stale =
              providerTrouble(providers.find((candidate) => candidate.widget_id === card.id)) !==
              null;
            return (
              <li
                key={card.id}
                className={`card-tile${selectedCardId === card.id ? " is-selected" : ""}${
                  cardIssues.length > 0 ? " has-issue" : ""
                }`}
              >
                <button
                  type="button"
                  className="card-tile__body"
                  aria-pressed={selectedCardId === card.id}
                  onClick={() => onSelect(card.id)}
                >
                  {/* The template names the card here and everywhere else; the
                      owner's own words sit under it, and are omitted rather than
                      repeated when they were never typed. */}
                  <span className="tile-label">{cardLabel(card)}</span>
                  <strong className="card-tile__value numeral">
                    {tileValue(card, data, pomodoro, now, config.preferences.timezone)}
                  </strong>
                  {cardTitle(card) && <span className="card-tile__name">{cardTitle(card)}</span>}
                </button>
                <span className="card-tile__flags">
                  {stale && <span className="flag flag--stale">stale</span>}
                  {hasAlert && <span className="flag flag--alert">alerts</span>}
                  {isUnused && <span className="flag">unused</span>}
                </span>
                <button
                  type="button"
                  className="card-tile__remove"
                  aria-label={
                    cards.length === 1
                      ? `Remove ${removeLabel(card)} (keep at least one card)`
                      : `Remove ${removeLabel(card)}`
                  }
                  title={cards.length === 1 ? "This is your only card." : "Remove"}
                  disabled={cards.length === 1}
                  onClick={() => onRemove(card.id)}
                >
                  <Icon name="close" />
                </button>
                <FieldIssues issues={cardIssues} />
              </li>
            );
          })}
        </ul>
      )}

      <fieldset className="card-add">
        <legend className="tile-label">Add a card</legend>
        {addableKinds.map(({ kind, description }) => (
          <button
            className="add-card"
            type="button"
            key={kind}
            onClick={() => onAdd(kind)}
            disabled={atCapacity}
            aria-describedby={atCapacity ? "card-capacity" : undefined}
          >
            <span className="add-card__plus">
              <Icon name="plus" />
            </span>
            <span>
              <strong>{cardKindName(kind)}</strong>
              <small>{description}</small>
            </span>
          </button>
        ))}
      </fieldset>
    </section>
  );
}
