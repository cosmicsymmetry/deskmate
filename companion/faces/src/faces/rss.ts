// The RSS face: one lead story set large, then the stories behind it.
//
// A panel is glanced at, not read. Five equal headlines at a size that fits five of
// them is a face you have to stop and work through, so the newest item gets the hero
// treatment and the rest become a quiet index underneath.
//
// The layout is computed rather than fixed: a lead headline is one, two or three
// lines depending on the feed, and a fixed divider would leave a hole under the
// short ones. The lead block is measured first and everything below is placed
// against that result.

import { XMLParser } from "fast-xml-parser";
import {
  ConfigurationError,
  type FaceDefinition,
  type RenderContext,
  type Settings,
  type ViewId,
  text,
  TransientError,
} from "../face";
import { relativeAge } from "../kit/age";
import { type FetchText, fetchText } from "../kit/http";
import { Canvas, fit, fitTracked, normalizeWhitespace, textWidth, wrap } from "../kit/svg";
import {
  baselineFromCapTop,
  baselineFromCenter,
  CANVAS_HEIGHT,
  CANVAS_WIDTH,
  CAP_HEIGHT,
  CONTENT_WIDTH,
  GOOD,
  GRID,
  GROUND,
  HAIRLINE,
  INK,
  INK_2,
  INK_3,
  MARGIN,
  SIZE_BODY,
  SIZE_CAPTION,
  SIZE_EYEBROW,
  SIZE_TITLE,
  TRACKING_EYEBROW,
  WEIGHT_REGULAR,
  WEIGHT_SEMIBOLD,
} from "../kit/theme";

export interface FeedEntry {
  title: string;
  /** A rendered relative age such as "14m". Empty when the item carried no usable date. */
  age: string;
}

export interface RssFace {
  /** The owner's name for the feed, set as the eyebrow. */
  feedTitle: string;
  entries: FeedEntry[];
}

// ---------------------------------------------------------------------------
// Data. RSS 2.0, Atom and RDF all parse. A title reaches the document only through
// `escapeXml`, so markup inside one is ugly text at worst -- never an element.
// ---------------------------------------------------------------------------

/** One lead plus three followers is what the face can show at once. */
const MAX_ITEMS = 4;

/**
 * How many entries are kept so a tap has somewhere to go.
 *
 * Four pages is plenty for a desk feed, and sixteen entries of a title plus a
 * short age string sit comfortably inside the 16 KB the server allows per
 * source's face state.
 */
const STORED_ITEMS = MAX_ITEMS * 4;

const parser = new XMLParser({
  ignoreAttributes: true,
  removeNSPrefix: true,
  processEntities: true,
  htmlEntities: true,
  trimValues: true,
  parseTagValue: false,
});

function asArray(value: unknown): unknown[] {
  if (value === undefined || value === null) {
    return [];
  }
  return Array.isArray(value) ? value : [value];
}

/** An element's text, whether the parser gave a string or a node with `#text`. */
function nodeText(value: unknown): string {
  if (typeof value === "string") {
    return value;
  }
  if (typeof value === "number") {
    return String(value);
  }
  if (typeof value === "object" && value !== null && "#text" in value) {
    return nodeText((value as Record<string, unknown>)["#text"]);
  }
  return "";
}

function plainTitle(value: unknown): string {
  // Atom allows `type="html"` titles; tags are dropped rather than shown.
  return normalizeWhitespace(nodeText(value).replace(/<[^>]*>/g, " "));
}

export function parseFeed(body: string, now: Date): FeedEntry[] {
  let document: Record<string, unknown>;
  try {
    document = parser.parse(body) as Record<string, unknown>;
  } catch {
    throw new TransientError("the feed is not well-formed XML");
  }
  const rss = document.rss as Record<string, unknown> | undefined;
  const rdf = document.RDF as Record<string, unknown> | undefined;
  const atom = document.feed as Record<string, unknown> | undefined;
  if (rss === undefined && rdf === undefined && atom === undefined) {
    throw new ConfigurationError("that URL did not return an RSS or Atom feed");
  }
  const channel = rss?.channel as Record<string, unknown> | undefined;
  const items = [...asArray(channel?.item), ...asArray(rdf?.item), ...asArray(atom?.entry)];

  return items.slice(0, STORED_ITEMS).map((raw) => {
    const item = (typeof raw === "object" && raw !== null ? raw : {}) as Record<string, unknown>;
    const dated = nodeText(item.pubDate ?? item.published ?? item.updated ?? item.date).trim();
    const published = dated === "" ? Number.NaN : Date.parse(dated);
    return {
      title: plainTitle(item.title) || "(Untitled)",
      age: Number.isNaN(published) ? "" : relativeAge(new Date(published), now),
    };
  });
}

export async function fetchRss(settings: Settings, now: Date, get: FetchText): Promise<RssFace> {
  const url = text(settings, "url");
  if (url === "") {
    throw new ConfigurationError("the feed URL is empty");
  }
  return { feedTitle: text(settings, "title"), entries: parseFeed(await get(url), now) };
}

// ---------------------------------------------------------------------------
// Drawing.
// ---------------------------------------------------------------------------

/** Tighter than a paragraph's, because a multi-line headline reads as one object. */
const LEAD_LEADING = 1.16;
/** The accent bar that marks the freshest item, in `--good`: the "fresh" role. */
const ACCENT_WIDTH = 4;
const ACCENT_GUTTER = 2 * GRID;
/** A follower row needs this much height to hold a headline without crowding. */
const MIN_FOLLOWER_HEIGHT = 40;
const MAX_FOLLOWERS = 3;
const MAX_LEAD_LINES = 3;
const RULE_Y = 52;

export function renderRss(face: RssFace): string {
  const canvas = new Canvas(CANVAS_WIDTH, CANVAS_HEIGHT);
  canvas.rect(0, 0, CANVAS_WIDTH, CANVAS_HEIGHT, GROUND);
  drawEyebrow(canvas, face.feedTitle);

  const [lead, ...followers] = face.entries;
  if (lead === undefined) {
    drawEmptyState(canvas);
    return canvas.finish();
  }
  drawFollowers(canvas, followers, drawLead(canvas, lead));
  return canvas.finish();
}

function drawEyebrow(canvas: Canvas, feedTitle: string): void {
  // Uppercased THEN measured: uppercasing widens text, so a title that fitted in
  // its original case can stop fitting. It is tracked, so measured with tracking.
  const upper = normalizeWhitespace(feedTitle).toUpperCase();
  canvas.text({
    x: MARGIN,
    baseline: baselineFromCapTop(MARGIN, SIZE_EYEBROW),
    content: fitTracked(
      upper === "" ? "FEED" : upper,
      SIZE_EYEBROW,
      WEIGHT_SEMIBOLD,
      TRACKING_EYEBROW,
      CONTENT_WIDTH,
    ),
    size: SIZE_EYEBROW,
    fill: INK_3,
    weight: WEIGHT_SEMIBOLD,
    tracking: TRACKING_EYEBROW,
  });
  canvas.rect(MARGIN, RULE_Y, CONTENT_WIDTH, 1, HAIRLINE);
}

/** Draws the lead story and returns the y the next element may start at. */
function drawLead(canvas: Canvas, lead: FeedEntry): number {
  const headlineX = MARGIN + ACCENT_WIDTH + ACCENT_GUTTER;
  const headlineWidth = CANVAS_WIDTH - MARGIN - headlineX;
  const lines = wrap(lead.title, SIZE_TITLE, WEIGHT_SEMIBOLD, headlineWidth, MAX_LEAD_LINES);

  const capTop = RULE_Y + 2.5 * GRID;
  const leading = SIZE_TITLE * LEAD_LEADING;
  const capHeight = SIZE_TITLE * CAP_HEIGHT;
  if (lines.length === 0) {
    return capTop;
  }

  // The accent spans the headline's own cap height, not the whole line box, so it
  // reads as aligned with the type rather than floating around it.
  const accentHeight = (lines.length - 1) * leading + capHeight;
  canvas.roundedRect(MARGIN, capTop, ACCENT_WIDTH, accentHeight, ACCENT_WIDTH / 2, GOOD);

  lines.forEach((line, index) => {
    canvas.text({
      x: headlineX,
      baseline: baselineFromCapTop(capTop + index * leading, SIZE_TITLE),
      content: line,
      size: SIZE_TITLE,
      fill: INK,
      weight: WEIGHT_SEMIBOLD,
    });
  });

  let next = capTop + accentHeight;
  if (lead.age !== "") {
    const metaTop = next + 1.5 * GRID;
    canvas.text({
      x: headlineX,
      baseline: baselineFromCapTop(metaTop, SIZE_CAPTION),
      content: lead.age,
      size: SIZE_CAPTION,
      fill: INK_3,
      weight: WEIGHT_SEMIBOLD,
      tracking: 0.4,
    });
    next = metaTop + SIZE_CAPTION * CAP_HEIGHT;
  }
  return next + 2 * GRID;
}

function drawFollowers(canvas: Canvas, followers: FeedEntry[], top: number): void {
  const available = CANVAS_HEIGHT - MARGIN - top;
  if (available < MIN_FOLLOWER_HEIGHT || followers.length === 0) {
    return;
  }
  // Only as many rows as fit at a readable height: squeezing a third row into the
  // space for two is how a face stops being glanceable.
  const capacity = Math.floor(available / MIN_FOLLOWER_HEIGHT);
  const rows = Math.min(
    Math.max(Math.min(followers.length, MAX_FOLLOWERS, capacity), 1),
    followers.length,
  );
  const rowHeight = available / rows;

  followers.slice(0, rows).forEach((entry, index) => {
    const rowTop = top + index * rowHeight;
    // Every row gets a rule above it, the first included: it is what separates the
    // lead block from the index below it.
    canvas.rect(MARGIN, rowTop, CONTENT_WIDTH, 1, HAIRLINE);

    const center = rowTop + rowHeight / 2;
    const ageWidth =
      entry.age === "" ? 0 : textWidth(entry.age, SIZE_CAPTION, WEIGHT_SEMIBOLD) + 1.5 * GRID;
    canvas.text({
      x: MARGIN,
      baseline: baselineFromCenter(center, SIZE_BODY),
      content: fit(
        normalizeWhitespace(entry.title),
        SIZE_BODY,
        WEIGHT_REGULAR,
        CONTENT_WIDTH - ageWidth,
      ),
      size: SIZE_BODY,
      fill: INK_2,
    });
    if (entry.age !== "") {
      canvas.text({
        x: CANVAS_WIDTH - MARGIN,
        baseline: baselineFromCenter(center, SIZE_CAPTION),
        content: entry.age,
        size: SIZE_CAPTION,
        fill: INK_3,
        weight: WEIGHT_SEMIBOLD,
        anchor: "end",
      });
    }
  });
}

/**
 * A feed that parsed but carried no items. A real steady state for a quiet feed, so
 * it gets a composed face rather than looking broken.
 */
function drawEmptyState(canvas: Canvas): void {
  const center = CANVAS_HEIGHT / 2;
  canvas.text({
    x: CANVAS_WIDTH / 2,
    baseline: baselineFromCenter(center, SIZE_TITLE),
    content: "No stories yet",
    size: SIZE_TITLE,
    fill: INK_2,
    weight: WEIGHT_SEMIBOLD,
    anchor: "middle",
  });
  canvas.text({
    x: CANVAS_WIDTH / 2,
    baseline: baselineFromCenter(center + 4 * GRID, SIZE_CAPTION),
    content: "The feed is reachable and empty",
    size: SIZE_CAPTION,
    fill: INK_3,
    anchor: "middle",
  });
}

/** What the face carries between renders: the entries it fetched, and where in them it is. */
export interface RssState {
  feedTitle: string;
  entries: FeedEntry[];
  page: number;
  tappedAt: string | null;
}

/** A page returns to the top after this long untouched, so the card rests on its lead story. */
const TEMPORARY_PAGE_MS = 10 * 60 * 1_000;

function storedState(value: unknown): RssState | undefined {
  if (typeof value !== "object" || value === null) {
    return undefined;
  }
  const candidate = value as Partial<RssState>;
  if (!Array.isArray(candidate.entries)) {
    return undefined;
  }
  return {
    feedTitle: typeof candidate.feedTitle === "string" ? candidate.feedTitle : "",
    entries: candidate.entries as FeedEntry[],
    page:
      typeof candidate.page === "number" && Number.isFinite(candidate.page) ? candidate.page : 0,
    tappedAt: typeof candidate.tappedAt === "string" ? candidate.tappedAt : null,
  };
}

function pageCount(entries: FeedEntry[]): number {
  return Math.max(1, Math.ceil(entries.length / MAX_ITEMS));
}

/** Wraps, so the last page taps back to the first rather than stopping dead. */
function normalizedPage(page: number, entries: FeedEntry[]): number {
  const pages = pageCount(entries);
  return ((page % pages) + pages) % pages;
}

/** Shared by staged selection and the server's render-only tap fallback. */
function tappedState(previous: RssState, taps: number, now: Date): RssState {
  return {
    ...previous,
    page: normalizedPage(previous.page + taps, previous.entries),
    tappedAt: now.toISOString(),
  };
}

function pageViews(entries: FeedEntry[]): ViewId[] {
  return Array.from({ length: pageCount(entries) }, (_, page) =>
    page === 0 ? "" : `page-${page + 1}`,
  );
}

function pageForView(view: ViewId | undefined, entries: FeedEntry[]): number {
  const page = pageViews(entries).indexOf(view ?? "");
  return page === -1 ? 0 : page;
}

function recentTap(tappedAt: string | null, now: Date): boolean {
  if (tappedAt === null) {
    return false;
  }
  const at = Date.parse(tappedAt);
  return Number.isFinite(at) && now.getTime() - at <= TEMPORARY_PAGE_MS;
}

function faceForPage(state: RssState): RssFace {
  const start = state.page * MAX_ITEMS;
  return {
    feedTitle: state.feedTitle,
    entries: state.entries.slice(start, start + MAX_ITEMS),
  };
}

/**
 * A tap turns the page **without fetching**: the entries are already in state,
 * so answering costs a draw and nothing else. The upstream feed is only read on
 * a scheduled refresh.
 */
export async function renderRssRequest(
  settings: Settings,
  now: Date,
  context: RenderContext = {},
  get: FetchText = fetchText,
): Promise<{ svg: string; state: RssState }> {
  const previous = storedState(context.state);
  // Explicit views only draw; an unstaged tap arrives without a view and must
  // advance the stored state, using the same transition as onTap.
  if (context.view === undefined && context.event !== undefined && previous !== undefined) {
    const state = tappedState(previous, context.event.taps, now);
    return { svg: renderRss(faceForPage(state)), state };
  }
  if (context.view !== undefined && previous !== undefined) {
    return {
      svg: renderRss(
        faceForPage({ ...previous, page: pageForView(context.view, previous.entries) }),
      ),
      state: previous,
    };
  }

  const fresh = await fetchRss(settings, now, get);
  const keepPage = previous !== undefined && recentTap(previous.tappedAt, now);
  const state: RssState = {
    feedTitle: fresh.feedTitle,
    entries: fresh.entries,
    page: keepPage ? normalizedPage(previous.page, fresh.entries) : 0,
    tappedAt: keepPage ? previous.tappedAt : null,
  };
  return {
    svg: renderRss(
      faceForPage({
        ...state,
        // As in hackernews: an absent view is a scheduled refresh, which draws
        // the page the state decided rather than resetting to the first one.
        page: context.view === undefined ? state.page : pageForView(context.view, fresh.entries),
      }),
    ),
    state,
  };
}

export const rss: FaceDefinition = {
  kind: "rss",
  label: "RSS feed",
  tap: "Tap the panel for the next headlines.",
  fields: [
    { type: "url", key: "url", label: "Feed URL", placeholder: "https://example.com/feed.xml" },
    { type: "text", key: "title", label: "Title", placeholder: "News" },
  ],
  views(_settings, value) {
    const state = storedState(value);
    return state === undefined ? [""] : pageViews(state.entries);
  },
  onTap(_settings, value, event, now) {
    const previous = storedState(value);
    if (previous === undefined) {
      return { view: "" };
    }
    const state = tappedState(previous, event.taps, now);
    return { view: pageViews(state.entries)[state.page] ?? "", state };
  },
  async render(settings, now, context) {
    return renderRssRequest(settings, now, context);
  },
};
