import { useEffect, useState, type DragEvent, type FormEvent, type KeyboardEvent } from "react";

import {
  addEntry,
  addPlaylist,
  cardLabel,
  cardMoveFromKey,
  cardTitle,
  cardsOutsidePlaylist,
  issuesForPath,
  moveEntry,
  numberValue,
  removeEntry,
  removePlaylist,
  renamePlaylist,
  setActivePlaylist,
  setEntryDwell,
  setPlaylistAdvance,
} from "../lib/configDraft";
import { Icon } from "./Icon";
import {
  MAX_PLAYLIST_ENTRIES,
  MAX_PLAYLIST_NAME_LEN,
  MAX_PLAYLISTS,
  type AppConfig,
  type ValidationIssue,
} from "../lib/types";

interface PlaylistPanelProps {
  config: AppConfig;
  issues: ValidationIssue[];
  onChange: (config: AppConfig) => void;
  onSelectCard: (cardId: string) => void;
}

function FieldIssues({
  issues,
  className = "",
}: {
  issues: ValidationIssue[];
  className?: string;
}) {
  if (issues.length === 0) {
    return null;
  }
  return (
    <ul className={`field-errors${className ? ` ${className}` : ""}`} role="alert">
      {issues.map((issue) => (
        <li key={`${issue.path}:${issue.code}`}>{issue.message}</li>
      ))}
    </ul>
  );
}

const DEFAULT_DWELL_SECONDS = 20;

export function PlaylistPanel({ config, issues, onChange, onSelectCard }: PlaylistPanelProps) {
  const [selectedPlaylistId, setSelectedPlaylistId] = useState(config.active_playlist_id);
  const [newPlaylistName, setNewPlaylistName] = useState("");
  const [renameValue, setRenameValue] = useState("");
  const [cardToAdd, setCardToAdd] = useState("");
  const [draggedIndex, setDraggedIndex] = useState<number | null>(null);

  const selectedIndex = config.playlists.findIndex(
    (playlist) => playlist.id === selectedPlaylistId,
  );
  const selectedPlaylist = selectedIndex >= 0 ? config.playlists[selectedIndex] : null;

  useEffect(() => {
    if (selectedIndex >= 0) {
      return;
    }
    setSelectedPlaylistId(
      config.playlists.some((playlist) => playlist.id === config.active_playlist_id)
        ? config.active_playlist_id
        : (config.playlists[0]?.id ?? ""),
    );
  }, [config.active_playlist_id, config.playlists, selectedIndex]);

  useEffect(() => {
    setRenameValue(selectedPlaylist?.name ?? "");
    setCardToAdd("");
  }, [selectedPlaylist?.name]);

  const apply = (next: AppConfig) => {
    if (next !== config) {
      onChange(next);
    }
  };

  const handleAddPlaylist = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const previousIds = new Set(config.playlists.map((playlist) => playlist.id));
    const next = addPlaylist(config, newPlaylistName);
    if (next === config) {
      return;
    }
    const added = next.playlists.find((playlist) => !previousIds.has(playlist.id));
    apply(next);
    setNewPlaylistName("");
    if (added) {
      setSelectedPlaylistId(added.id);
    }
  };

  const rootIssues = issues.filter(
    (issue) => issue.path === "playlists" || issue.path === "active_playlist_id",
  );
  const outsideCards = selectedPlaylist ? cardsOutsidePlaylist(config, selectedPlaylist.id) : [];
  const outsideIds = new Set(outsideCards.map((card) => card.id));
  const atEntryCapacity = (selectedPlaylist?.entries.length ?? 0) >= MAX_PLAYLIST_ENTRIES;
  const trimmedNewName = newPlaylistName.trim();
  const newNameExists = config.playlists.some((playlist) => playlist.name === trimmedNewName);
  const trimmedRename = renameValue.trim();
  const renameExists = config.playlists.some(
    (playlist) => playlist.id !== selectedPlaylist?.id && playlist.name === trimmedRename,
  );

  const moveTo = (from: number, to: number) => {
    if (!selectedPlaylist) {
      return;
    }
    apply(moveEntry(config, selectedPlaylist.id, from, to));
  };

  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    const delta = cardMoveFromKey(event.key, event.altKey);
    if (delta === 0 || !selectedPlaylist?.entries[index + delta]) {
      return;
    }
    event.preventDefault();
    moveTo(index, index + delta);
  };

  const onDragStart = (event: DragEvent<HTMLLIElement>, index: number) => {
    setDraggedIndex(index);
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData("text/plain", String(index));
  };

  const onDrop = (event: DragEvent<HTMLLIElement>, targetIndex: number) => {
    event.preventDefault();
    const transferred = Number(event.dataTransfer.getData("text/plain"));
    const from = draggedIndex ?? transferred;
    if (Number.isInteger(from)) {
      moveTo(from, targetIndex);
    }
    setDraggedIndex(null);
  };

  return (
    <section className="panel playlist-panel" aria-labelledby="playlist-heading">
      <div className="panel-heading">
        <div>
          <h2 id="playlist-heading">What plays, and when</h2>
        </div>
        <span className="count-badge numeral" id="playlist-capacity">
          {config.playlists.length}/{MAX_PLAYLISTS}
        </span>
      </div>

      <FieldIssues issues={rootIssues} className="playlist-panel-issues" />

      <ul className="playlist-tabs" aria-label="Playlists">
        {config.playlists.map((playlist, index) => {
          const isActive = playlist.id === config.active_playlist_id;
          const isSelected = playlist.id === selectedPlaylistId;
          const playlistIssues = issuesForPath(issues, `playlists[${index}]`);
          return (
            <li key={playlist.id} className={isSelected ? "is-selected" : undefined}>
              <button
                type="button"
                aria-pressed={isSelected}
                onClick={() => setSelectedPlaylistId(playlist.id)}
              >
                <span
                  className={`playlist-tab__dot${isActive ? " is-active" : ""}`}
                  aria-hidden="true"
                />
                <strong>{playlist.name}</strong>
                {isActive && <small>active</small>}
                {!isSelected && playlistIssues.length > 0 && (
                  <span className="playlist-tab__issue-count numeral">{playlistIssues.length}</span>
                )}
              </button>
              {!isSelected && (
                <FieldIssues issues={playlistIssues} className="playlist-tab-issues" />
              )}
            </li>
          );
        })}
      </ul>

      <form className="playlist-add-form" onSubmit={handleAddPlaylist}>
        <label className="field">
          <span>New playlist</span>
          <input
            value={newPlaylistName}
            maxLength={MAX_PLAYLIST_NAME_LEN}
            placeholder="Evening"
            onChange={(event) => setNewPlaylistName(event.currentTarget.value)}
            aria-describedby="playlist-capacity"
          />
        </label>
        <button
          className="button button--quiet"
          type="submit"
          disabled={!trimmedNewName || newNameExists || config.playlists.length >= MAX_PLAYLISTS}
        >
          Add playlist
        </button>
      </form>

      {selectedPlaylist && (
        <div className="playlist-detail">
          <div className="playlist-actions">
            <label className="field">
              <span>Playlist name</span>
              <input
                value={renameValue}
                maxLength={MAX_PLAYLIST_NAME_LEN}
                onChange={(event) => setRenameValue(event.currentTarget.value)}
                aria-invalid={issuesForPath(issues, `playlists[${selectedIndex}].name`).length > 0}
              />
              <FieldIssues
                issues={[
                  ...issues.filter(
                    (issue) =>
                      issue.path === `playlists[${selectedIndex}]` ||
                      issue.path === `playlists[${selectedIndex}].id`,
                  ),
                  ...issuesForPath(issues, `playlists[${selectedIndex}].name`),
                ]}
              />
            </label>
            <div className="playlist-action-buttons">
              <button
                type="button"
                className="button button--quiet"
                disabled={!trimmedRename || trimmedRename === selectedPlaylist.name || renameExists}
                onClick={() => apply(renamePlaylist(config, selectedPlaylist.id, trimmedRename))}
              >
                Rename
              </button>
              <button
                type="button"
                className="button button--secondary"
                disabled={selectedPlaylist.id === config.active_playlist_id}
                onClick={() => apply(setActivePlaylist(config, selectedPlaylist.id))}
              >
                {selectedPlaylist.id === config.active_playlist_id
                  ? "Active playlist"
                  : "Make active"}
              </button>
              <button
                type="button"
                className="text-button text-button--danger"
                disabled={config.playlists.length === 1}
                onClick={() => {
                  const next = removePlaylist(config, selectedPlaylist.id);
                  apply(next);
                  if (next !== config) {
                    setSelectedPlaylistId(next.active_playlist_id);
                  }
                }}
              >
                Delete playlist
              </button>
            </div>
          </div>

          <fieldset className="advance-fieldset">
            <legend className="section-label">Advance</legend>
            <label className="advance-option">
              <input
                type="radio"
                name={`advance-${selectedPlaylist.id}`}
                checked={selectedPlaylist.advance.kind === "manual"}
                onChange={() =>
                  apply(setPlaylistAdvance(config, selectedPlaylist.id, { kind: "manual" }))
                }
              />
              <span>
                <strong>Manual</strong>
                <small>The display stays put until you swipe.</small>
              </span>
            </label>
            <label className="advance-option">
              <input
                type="radio"
                name={`advance-${selectedPlaylist.id}`}
                checked={selectedPlaylist.advance.kind === "timed"}
                onChange={() =>
                  apply(
                    setPlaylistAdvance(config, selectedPlaylist.id, {
                      kind: "timed",
                      default_dwell_seconds:
                        selectedPlaylist.advance.kind === "timed"
                          ? selectedPlaylist.advance.default_dwell_seconds
                          : DEFAULT_DWELL_SECONDS,
                    }),
                  )
                }
              />
              <span>
                <strong>Timed</strong>
                <small>Move through entries automatically.</small>
              </span>
            </label>
            {selectedPlaylist.advance.kind === "timed" && (
              <label className="field advance-dwell-field">
                <span>Default dwell in seconds</span>
                <input
                  type="number"
                  className="numeral"
                  min={5}
                  max={3600}
                  step={1}
                  value={selectedPlaylist.advance.default_dwell_seconds}
                  onChange={(event) =>
                    apply(
                      setPlaylistAdvance(config, selectedPlaylist.id, {
                        kind: "timed",
                        default_dwell_seconds: numberValue(event.currentTarget.value),
                      }),
                    )
                  }
                  aria-invalid={
                    issuesForPath(
                      issues,
                      `playlists[${selectedIndex}].advance.default_dwell_seconds`,
                    ).length > 0
                  }
                />
              </label>
            )}
            <FieldIssues issues={issuesForPath(issues, `playlists[${selectedIndex}].advance`)} />
          </fieldset>

          <div className="playlist-entries-heading">
            <div>
              <p className="section-label">Ordered entries</p>
              <small>Blank dwell inherits the playlist default.</small>
            </div>
            <span className="keyboard-hint">⌥ ↑ ↓ to move</span>
          </div>
          <FieldIssues
            issues={issues.filter((issue) => issue.path === `playlists[${selectedIndex}].entries`)}
            className="playlist-entry-container-issues"
          />

          {selectedPlaylist.entries.length === 0 ? (
            <div className="empty-state">
              <strong>This playlist is empty</strong>
              <span>Add a card from the library below.</span>
            </div>
          ) : (
            <ol className="playlist-entry-list" aria-label={`${selectedPlaylist.name} entries`}>
              {selectedPlaylist.entries.map((entry, index) => {
                const card = config.cards.find((candidate) => candidate.id === entry.card_id);
                const name = card ? cardLabel(card) : "Missing card";
                const title = card ? cardTitle(card) : null;
                // Two entries can share a template, so a control that acts on one of
                // them has to say which: the owner's title is the only thing that
                // distinguishes a second "ICS calendar" from the first.
                const controlName = title ? `${name} — ${title}` : name;
                const entryIssues = issuesForPath(
                  issues,
                  `playlists[${selectedIndex}].entries[${index}]`,
                );
                return (
                  <li
                    key={entry.card_id}
                    draggable
                    className={`playlist-entry${draggedIndex === index ? " is-dragging" : ""}`}
                    onDragStart={(event) => onDragStart(event, index)}
                    onDragEnd={() => setDraggedIndex(null)}
                    onDragOver={(event) => event.preventDefault()}
                    onDrop={(event) => onDrop(event, index)}
                  >
                    <span className="card-row__handle">
                      <Icon name="grip" />
                    </span>
                    <span className="card-row__index numeral">{index + 1}</span>
                    <button
                      type="button"
                      className="card-row__body"
                      onClick={() => card && onSelectCard(card.id)}
                      onKeyDown={(event) => onKeyDown(event, index)}
                    >
                      <strong>{name}</strong>
                      {card ? (
                        title && <small>{title}</small>
                      ) : (
                        <small>Reference needs attention</small>
                      )}
                    </button>
                    <label className="entry-dwell-field">
                      <span className="sr-only">Dwell time for {controlName} in seconds</span>
                      <input
                        type="number"
                        className="numeral"
                        min={5}
                        max={3600}
                        step={1}
                        placeholder="inherit"
                        value={entry.dwell_seconds ?? ""}
                        onChange={(event) =>
                          apply(
                            setEntryDwell(
                              config,
                              selectedPlaylist.id,
                              index,
                              event.currentTarget.value === ""
                                ? null
                                : numberValue(event.currentTarget.value),
                            ),
                          )
                        }
                        aria-invalid={entryIssues.some((issue) =>
                          issue.path.endsWith(".dwell_seconds"),
                        )}
                      />
                    </label>
                    <span className="card-row__moves">
                      <button
                        type="button"
                        aria-label={`Move ${controlName} up`}
                        disabled={index === 0}
                        onClick={() => moveTo(index, index - 1)}
                      >
                        <Icon name="up" />
                      </button>
                      <button
                        type="button"
                        aria-label={`Move ${controlName} down`}
                        disabled={index === selectedPlaylist.entries.length - 1}
                        onClick={() => moveTo(index, index + 1)}
                      >
                        <Icon name="down" />
                      </button>
                    </span>
                    <button
                      type="button"
                      className="text-button text-button--danger"
                      aria-label={`Remove ${name} from ${selectedPlaylist.name}`}
                      onClick={() => apply(removeEntry(config, selectedPlaylist.id, index))}
                    >
                      Remove
                    </button>
                    <FieldIssues issues={entryIssues} className="playlist-entry-issues" />
                  </li>
                );
              })}
            </ol>
          )}

          <div className="playlist-add-entry">
            <label className="field">
              <span>Add from library</span>
              <select
                value={cardToAdd}
                onChange={(event) => setCardToAdd(event.currentTarget.value)}
                disabled={atEntryCapacity || outsideCards.length === 0}
              >
                <option value="">Choose a card…</option>
                {config.cards.map((card) => (
                  <option key={card.id} value={card.id} disabled={!outsideIds.has(card.id)}>
                    {cardLabel(card)}
                    {cardTitle(card) ? ` — ${cardTitle(card)}` : ""}
                    {!outsideIds.has(card.id) ? " — already added" : ""}
                  </option>
                ))}
              </select>
              {outsideCards.length === 0 && (
                <small>Every library card is already in this playlist.</small>
              )}
            </label>
            <button
              type="button"
              className="button button--quiet"
              disabled={!cardToAdd || !outsideIds.has(cardToAdd) || atEntryCapacity}
              onClick={() => {
                apply(addEntry(config, selectedPlaylist.id, cardToAdd));
                setCardToAdd("");
              }}
            >
              Add card
            </button>
          </div>
        </div>
      )}
    </section>
  );
}
