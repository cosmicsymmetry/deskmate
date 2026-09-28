// The Hacker News face: the top story as a hero, the next few as a numbered index.
//
// It replaces pointing the generic RSS face at news.ycombinator.com/rss, which had
// two problems a feed cannot fix. On Hacker News the title IS the content, and the
// RSS face truncated it at every level; and the feed carries none of what makes the
// front page the front page -- points, comments, where the link goes.
//
// # The anatomy is the weather face's
//
// A rounded hero module on black with a SURFACE strip underneath. The hero's ground
// is Hacker News orange taken down to an ember, for the reason weather's blue is a
// night sky: colour lives in a module, never full-bleed, and never saturated across
// an area -- an emissive panel showing a field of #ff6600 is a desk lamp. The
// saturated orange appears only as small marks: the Y chip and the vote arrows.
//
// # Nothing is truncated while an arrangement exists that avoids it
//
// The lead's type size and the number of index rows are chosen TOGETHER. Every
// combination is tried -- most rows first, largest type first -- and one is accepted
// only if every title in it fits whole. A long headline therefore costs index rows
// before it costs a word, and a short one is set as large as a hero numeral.

import {
  type FaceDefinition,
  type RenderContext,
  type Settings,
  type ViewId,
  text,
  TransientError,
} from "../face";
import { relativeAge } from "../kit/age";
import { type FetchText, fetchText } from "../kit/http";
import {
  Canvas,
  fit,
  fitTracked,
  fixed,
  normalizeWhitespace,
  textWidth,
  trackedWidth,
  wrap,
} from "../kit/svg";
import {
  baselineFromCapTop,
  baselineFromCenter,
  CANVAS_HEIGHT,
  CANVAS_WIDTH,
  CAP_HEIGHT,
  CONTENT_WIDTH,
  GRID,
  GROUND,
  HAIRLINE,
  INK,
  INK_2,
  INK_3,
  MARGIN,
  RADIUS_MODULE,
  SIZE_BODY,
  SIZE_CAPTION,
  SIZE_EYEBROW,
  SIZE_SUBHEAD,
  SURFACE,
  TRACKING_EYEBROW,
  WEIGHT_REGULAR,
  WEIGHT_SEMIBOLD,
} from "../kit/theme";

// ---------------------------------------------------------------------------
// The view model. Fields arrive decided: the face does layout, never arithmetic.
// ---------------------------------------------------------------------------

export interface Story {
  /** 1-based position in the list. */
  rank: number;
  title: string;
  points: number;
  comments: number;
  /** Bare host, e.g. "blog.cloudflare.com". Empty for Ask HN and other text posts. */
  domain: string;
  /** Already rendered: "14m", "3h", "1d". */
  age: string;
}

export interface HackerNewsFace {
  stories: Story[];
  /** Zero-based position and total, omitted by legacy/static render callers. */
  page?: { index: number; count: number };
}

export interface HackerNewsState {
  stories: Story[];
  page: number;
  tappedAt: string | null;
}

// ---------------------------------------------------------------------------
// Data: the official Firebase API. One request for the ranked ids, then one per
// story -- the ranked list is the only source that is in front-page order.
// ---------------------------------------------------------------------------

const API = "https://hacker-news.firebaseio.com/v0";
const LISTS: Record<string, string> = {
  top: "topstories",
  best: "beststories",
  new: "newstories",
  ask: "askstories",
  show: "showstories",
};
/** One lead plus three index rows is the most the face ever shows. */
const MAX_STORIES = 4;
/** A couple of spares, because a ranked id can be dead or deleted. */
const FETCHED = MAX_STORIES * 5 + 2;
const STORED_STORIES = MAX_STORIES * 5;
const TEMPORARY_VIEW_MS = 10 * 60 * 1000;

const ENTITIES: Record<string, string> = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'" };

function decodeEntities(value: string): string {
  return value.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (whole, body: string) => {
    if (body.startsWith("#")) {
      const code =
        body[1]?.toLowerCase() === "x" ? Number.parseInt(body.slice(2), 16) : Number(body.slice(1));
      return Number.isInteger(code) && code > 0 && code <= 0x10ffff
        ? String.fromCodePoint(code)
        : whole;
    }
    return ENTITIES[body.toLowerCase()] ?? whole;
  });
}

function domainOf(url: unknown): string {
  if (typeof url !== "string") {
    return "";
  }
  try {
    return new URL(url).hostname.replace(/^www\./, "");
  } catch {
    return "";
  }
}

const count = (value: unknown): number =>
  typeof value === "number" && Number.isFinite(value) ? Math.max(0, Math.trunc(value)) : 0;

export function storyFromItem(raw: unknown, rank: number, now: Date): Story | undefined {
  if (typeof raw !== "object" || raw === null) {
    return undefined;
  }
  const item = raw as Record<string, unknown>;
  if (item.deleted === true || item.dead === true || typeof item.title !== "string") {
    return undefined;
  }
  const title = normalizeWhitespace(decodeEntities(item.title));
  if (title === "") {
    return undefined;
  }
  return {
    rank,
    title,
    points: count(item.score),
    comments: count(item.descendants),
    domain: domainOf(item.url),
    age: typeof item.time === "number" ? relativeAge(new Date(item.time * 1000), now) : "",
  };
}

export async function fetchHackerNews(
  settings: Settings,
  now: Date,
  get: FetchText,
): Promise<HackerNewsFace> {
  const list = LISTS[text(settings, "list")] ?? "topstories";
  let ids: unknown;
  try {
    ids = JSON.parse(await get(`${API}/${list}.json`));
  } catch (error) {
    if (error instanceof SyntaxError) {
      throw new TransientError("the Hacker News story list is not JSON");
    }
    throw error;
  }
  if (!Array.isArray(ids)) {
    throw new TransientError("the Hacker News story list is not a list");
  }
  const wanted = ids.filter((id): id is number => Number.isInteger(id)).slice(0, FETCHED);
  // One failed story must not blank the face: it is simply skipped.
  const items = await Promise.all(
    wanted.map((id) =>
      get(`${API}/item/${id}.json`)
        .then((body) => JSON.parse(body) as unknown)
        .catch(() => undefined),
    ),
  );
  const stories: Story[] = [];
  for (const item of items) {
    const story = storyFromItem(item, stories.length + 1, now);
    if (story !== undefined && stories.length < STORED_STORIES) {
      stories.push(story);
    }
  }
  if (stories.length === 0 && wanted.length > 0) {
    throw new TransientError("no Hacker News story could be read");
  }
  return { stories };
}

// ---------------------------------------------------------------------------
// Drawing.
// ---------------------------------------------------------------------------

/** Hacker News orange. Only ever a small mark: the Y chip and the vote arrows. */
const HN_ORANGE = "#ff6600";
/** The hero ground: the orange taken down to an ember. */
const HERO_GROUND = "#2b1508";
/** Secondary inks warmed for the ember ground, where the neutral greys read cold. */
const HERO_INK_2 = "#c9b3a4";
const HERO_INK_3 = "#a68f80";

const TOP = MARGIN;
const HEIGHT = CANVAS_HEIGHT - 2 * MARGIN;
const MODULE_GAP = 2 * GRID;

// Hero anatomy, top to bottom: source row, headline, signal row.
const HERO_PAD_X = 2.5 * GRID;
const HERO_PAD_TOP = 2 * GRID;
const CHIP = 22;
const CHIP_TEXT = 15;
const HERO_GAP_TITLE = 14;
const HERO_GAP_SIGNALS = 2 * GRID;
const SIGNAL_SIZE = SIZE_SUBHEAD;
const HERO_PAD_BOTTOM = 18;
const HERO_INNER = CONTENT_WIDTH - 2 * HERO_PAD_X;

// Index anatomy.
const STRIP_PAD_X = 2 * GRID;
const STRIP_PAD_Y = 4;
const ROW_SIZE = SIZE_BODY;
const ROW_LEADING = 26;
const ROW_PAD = 12;
const ROW_PAD_MAX = 17;
const RANK_COLUMN = 26;
/**
 * The gap between a rank numeral and its title. Chosen to match the rhythm the
 * fixed 26px column already gave a single digit (26 - 11.3 for a "9"), and kept
 * just under it so every one-digit page stays byte-identical to its golden.
 */
const RANK_GUTTER = 14;
const POINTS_SIZE = SIZE_CAPTION;
const POINTS_GUTTER = 14;
const ARROW_SMALL = 9;
const MAX_ROWS = 3;

/** Lead sizes tried when an index is shown beneath, and when the lead stands alone. */
const LEAD_STEPS_WITH_INDEX = [40, 36, 31, 28, 26] as const;
const LEAD_STEPS_ALONE = [56, 48, 40, 36, 31, 28, 26, 23, 21] as const;
const LEAD_FLOOR = 21;

const capHeight = (size: number): number => size * CAP_HEIGHT;
const leadLeading = (size: number): number => Math.round(size * 1.17);
const f2 = (value: number): string => fixed(value, 2);

const blockHeight = (lines: number, size: number, leading: number): number =>
  (lines - 1) * leading + capHeight(size);

const heroMinimum = (lines: number, size: number): number =>
  HERO_PAD_TOP +
  CHIP +
  HERO_GAP_TITLE +
  blockHeight(lines, size, leadLeading(size)) +
  HERO_GAP_SIGNALS +
  capHeight(SIGNAL_SIZE) +
  HERO_PAD_BOTTOM;

const rowHeight = (lines: number, pad: number): number =>
  2 * pad + blockHeight(lines, ROW_SIZE, ROW_LEADING);

/** The untruncated lines, or undefined when a single word would have to be broken. */
function wholeLines(
  title: string,
  size: number,
  weight: number,
  width: number,
): string[] | undefined {
  const words = normalizeWhitespace(title).split(" ");
  if (words.some((word) => textWidth(word, size, weight) > width)) {
    return undefined;
  }
  return wrap(title, size, weight, width, 99);
}

/**
 * The same line count at the narrowest measure that keeps it, so a headline does
 * not end on one orphaned word under two full lines.
 */
function balanced(
  title: string,
  size: number,
  weight: number,
  width: number,
  lineCount: number,
): string[] {
  let low = width * 0.5;
  let high = width;
  let best = wrap(title, size, weight, width, lineCount);
  for (let step = 0; step < 12; step += 1) {
    const middle = (low + high) / 2;
    const lines = wholeLines(title, size, weight, middle);
    if (lines !== undefined && lines.length <= lineCount) {
      best = lines;
      high = middle;
    } else {
      low = middle;
    }
  }
  return best;
}

interface Row {
  story: Story;
  lines: string[];
}

interface Plan {
  leadSize: number;
  leadLines: string[];
  rows: Row[];
  heroHeight: number;
  rowPad: number;
  rankColumn: number;
}

/**
 * How much room the index gives its rank numerals.
 *
 * Paging made ranks reach two digits for the first time -- the front page is 1..4,
 * page three is 9..12 -- and a fixed 26px column left "10" all but touching the
 * title beside it. The column is therefore measured, and a page of single digits
 * keeps exactly the width it always had.
 */
function rankColumn(stories: Story[]): number {
  if (stories.length === 0) {
    return RANK_COLUMN;
  }
  const widest = Math.max(
    ...stories.map((story) => textWidth(String(story.rank), ROW_SIZE, WEIGHT_SEMIBOLD)),
  );
  return Math.max(RANK_COLUMN, widest + RANK_GUTTER);
}

function pointsColumn(stories: Story[]): number {
  const widest = Math.max(
    ...stories.map((story) => textWidth(String(story.points), POINTS_SIZE, WEIGHT_SEMIBOLD)),
  );
  return ARROW_SMALL * 1.12 + 5 + widest;
}

function stripHeight(rows: Row[], pad: number): number {
  return 2 * STRIP_PAD_Y + rows.reduce((total, row) => total + rowHeight(row.lines.length, pad), 0);
}

function plan(lead: Story, rest: Story[]): Plan {
  let best: (Plan & { score: number }) | undefined;

  for (let rowCount = Math.min(MAX_ROWS, rest.length); rowCount >= 0; rowCount -= 1) {
    const shown = rest.slice(0, rowCount);
    const shownRankColumn = rankColumn(shown);
    const titleWidth =
      CONTENT_WIDTH -
      2 * STRIP_PAD_X -
      shownRankColumn -
      (rowCount > 0 ? pointsColumn(shown) : 0) -
      POINTS_GUTTER;
    const natural = shown.map((story) =>
      wholeLines(story.title, ROW_SIZE, WEIGHT_REGULAR, titleWidth),
    );
    if (natural.some((lines) => lines === undefined)) {
      continue;
    }
    const rows = shown.map((story, index) => ({
      story,
      lines: balanced(
        story.title,
        ROW_SIZE,
        WEIGHT_REGULAR,
        titleWidth,
        natural[index]?.length ?? 1,
      ),
    }));
    const heroBudget = HEIGHT - (rowCount === 0 ? 0 : stripHeight(rows, ROW_PAD) + MODULE_GAP);

    for (const size of rowCount === 0 ? LEAD_STEPS_ALONE : LEAD_STEPS_WITH_INDEX) {
      const lines = wholeLines(lead.title, size, WEIGHT_SEMIBOLD, HERO_INNER);
      if (lines === undefined || heroMinimum(lines.length, size) > heroBudget) {
        continue;
      }
      // A row of index is worth more than a step of lead size, up to a point: three
      // rows under a 26px lead loses to two rows under a 40px one. The lone lead is
      // scored below every arrangement that has an index -- its larger steps would
      // otherwise outbid one row, and a front page of one story is not a front page.
      const score = rowCount === 0 ? 0 : 12 * rowCount + size;
      if (best === undefined || score > best.score) {
        // Spare height loosens the rows first, to a cap, and the hero takes the rest.
        const slack = heroBudget - heroMinimum(lines.length, size);
        const rowPad =
          rowCount === 0 ? ROW_PAD : Math.min(ROW_PAD_MAX, ROW_PAD + slack / (2 * rowCount));
        best = {
          score,
          leadSize: size,
          leadLines: balanced(lead.title, size, WEIGHT_SEMIBOLD, HERO_INNER, lines.length),
          rows,
          heroHeight: HEIGHT - (rowCount === 0 ? 0 : stripHeight(rows, rowPad) + MODULE_GAP),
          rowPad,
          rankColumn: shownRankColumn,
        };
      }
      break; // The largest size that fits this row count; smaller ones only score lower.
    }
  }
  if (best !== undefined) {
    return best;
  }
  // Nothing fits whole -- a pathological title. The lead alone, at the floor, cut to
  // the module: the one case in which this face truncates.
  const room = HEIGHT - heroMinimum(1, LEAD_FLOOR);
  const maxLines = 1 + Math.floor(room / leadLeading(LEAD_FLOOR));
  return {
    leadSize: LEAD_FLOOR,
    leadLines: wrap(lead.title, LEAD_FLOOR, WEIGHT_SEMIBOLD, HERO_INNER, maxLines),
    rows: [],
    heroHeight: HEIGHT,
    rowPad: ROW_PAD,
    rankColumn: RANK_COLUMN,
  };
}

/** A vote arrow `height` tall standing on `base`. Returns its width. */
function arrow(canvas: Canvas, x: number, base: number, height: number, fill: string): number {
  const width = height * 1.12;
  canvas.path(
    `M${f2(x)} ${f2(base)}L${f2(x + width)} ${f2(base)}L${f2(x + width / 2)} ${f2(base - height)}Z`,
    fill,
  );
  return width;
}

/** A speech bubble `height` tall overall, standing on `base`. Returns its width. */
function bubble(canvas: Canvas, x: number, base: number, height: number, fill: string): number {
  const width = height * 1.2;
  const body = height * 0.78;
  const top = base - height;
  canvas.roundedRect(x, top, width, body, height * 0.24, fill);
  const tail = x + width * 0.2;
  const under = top + body - 1;
  canvas.path(
    `M${f2(tail)} ${f2(under)}L${f2(tail)} ${f2(base)}L${f2(tail + height * 0.36)} ${f2(under)}Z`,
    fill,
  );
  return width;
}

function sourceRow(
  canvas: Canvas,
  source: string,
  age: string,
  page?: HackerNewsFace["page"],
): void {
  const x = MARGIN + HERO_PAD_X;
  const right = MARGIN + CONTENT_WIDTH - HERO_PAD_X;
  const chipTop = TOP + HERO_PAD_TOP;
  const center = chipTop + CHIP / 2;

  canvas.roundedRect(x, chipTop, CHIP, CHIP, 5, HN_ORANGE);
  canvas.text({
    x: x + CHIP / 2,
    baseline: baselineFromCenter(center, CHIP_TEXT),
    content: "Y",
    size: CHIP_TEXT,
    fill: "#ffffff",
    weight: WEIGHT_SEMIBOLD,
    anchor: "middle",
  });

  const ageWidth = age === "" ? 0 : textWidth(age, SIZE_CAPTION, WEIGHT_REGULAR) + 1.5 * GRID;
  if (age !== "") {
    canvas.text({
      x: right,
      baseline: baselineFromCenter(center, SIZE_CAPTION),
      content: age,
      size: SIZE_CAPTION,
      fill: HERO_INK_3,
      anchor: "end",
    });
  }
  const sourceX = x + CHIP + 10;
  let sourceRight = right - ageWidth;
  if (page !== undefined) {
    const pageText = fitTracked(
      `${page.index + 1} / ${page.count}`,
      SIZE_EYEBROW,
      WEIGHT_SEMIBOLD,
      TRACKING_EYEBROW,
      10 * GRID,
    );
    const pageWidth = trackedWidth(pageText, SIZE_EYEBROW, WEIGHT_SEMIBOLD, TRACKING_EYEBROW);
    const pageRight = right - ageWidth;
    canvas.text({
      x: pageRight,
      baseline: baselineFromCenter(center, SIZE_EYEBROW),
      content: pageText,
      size: SIZE_EYEBROW,
      fill: HERO_INK_3,
      weight: WEIGHT_SEMIBOLD,
      anchor: "end",
      tracking: TRACKING_EYEBROW,
    });
    sourceRight = pageRight - pageWidth - 2 * GRID;
  }
  canvas.text({
    x: sourceX,
    baseline: baselineFromCenter(center, SIZE_CAPTION),
    content: fit(source, SIZE_CAPTION, WEIGHT_REGULAR, sourceRight - sourceX),
    size: SIZE_CAPTION,
    fill: HERO_INK_2,
  });
}

function drawHero(canvas: Canvas, lead: Story, layout: Plan, page?: HackerNewsFace["page"]): void {
  canvas.roundedRect(MARGIN, TOP, CONTENT_WIDTH, layout.heroHeight, RADIUS_MODULE, HERO_GROUND);
  sourceRow(canvas, lead.domain === "" ? "Hacker News" : lead.domain, lead.age, page);

  // The two numerals that make this Hacker News. The word beside neither is needed:
  // the arrow and the bubble say which is which, so colour is not carrying it alone.
  const x = MARGIN + HERO_PAD_X;
  const signalBase = TOP + layout.heroHeight - HERO_PAD_BOTTOM;
  const glyph = capHeight(SIGNAL_SIZE);
  let cursor = x;
  cursor += arrow(canvas, cursor, signalBase - 1, glyph - 2, HN_ORANGE) + 7;
  const points = String(lead.points);
  canvas.text({
    x: cursor,
    baseline: signalBase,
    content: points,
    size: SIGNAL_SIZE,
    fill: INK,
    weight: WEIGHT_SEMIBOLD,
  });
  cursor += textWidth(points, SIGNAL_SIZE, WEIGHT_SEMIBOLD) + 22;
  cursor += bubble(canvas, cursor, signalBase + 1, glyph, HERO_INK_3) + 7;
  canvas.text({
    x: cursor,
    baseline: signalBase,
    content: String(lead.comments),
    size: SIGNAL_SIZE,
    fill: INK,
    weight: WEIGHT_SEMIBOLD,
  });

  // The headline, centred in what is left between the two rows.
  const leading = leadLeading(layout.leadSize);
  const block = blockHeight(layout.leadLines.length, layout.leadSize, leading);
  const zoneTop = TOP + HERO_PAD_TOP + CHIP + HERO_GAP_TITLE;
  const zoneBottom = signalBase - glyph - HERO_GAP_SIGNALS;
  const capTop = zoneTop + Math.max(0, (zoneBottom - zoneTop - block) / 2);
  layout.leadLines.forEach((line, index) => {
    canvas.text({
      x,
      baseline: baselineFromCapTop(capTop + index * leading, layout.leadSize),
      content: line,
      size: layout.leadSize,
      fill: INK,
      weight: WEIGHT_SEMIBOLD,
    });
  });
}

function drawIndex(canvas: Canvas, layout: Plan): void {
  if (layout.rows.length === 0) {
    return;
  }
  const top = TOP + layout.heroHeight + MODULE_GAP;
  canvas.roundedRect(MARGIN, top, CONTENT_WIDTH, TOP + HEIGHT - top, RADIUS_MODULE, SURFACE);
  const x = MARGIN + STRIP_PAD_X;
  const right = MARGIN + CONTENT_WIDTH - STRIP_PAD_X;

  let rowTop = top + STRIP_PAD_Y;
  layout.rows.forEach((row, index) => {
    if (index > 0) {
      canvas.rect(x, rowTop, right - x, 1, HAIRLINE);
    }
    const baseline = baselineFromCapTop(rowTop + layout.rowPad, ROW_SIZE);
    canvas.text({
      x,
      baseline,
      content: String(row.story.rank),
      size: ROW_SIZE,
      fill: INK_3,
      weight: WEIGHT_SEMIBOLD,
    });
    row.lines.forEach((line, lineIndex) => {
      canvas.text({
        x: x + layout.rankColumn,
        baseline: baseline + lineIndex * ROW_LEADING,
        content: line,
        size: ROW_SIZE,
        fill: INK,
      });
    });
    const points = String(row.story.points);
    canvas.text({
      x: right,
      baseline,
      content: points,
      size: POINTS_SIZE,
      fill: INK_2,
      weight: WEIGHT_SEMIBOLD,
      anchor: "end",
    });
    const arrowX = right - textWidth(points, POINTS_SIZE, WEIGHT_SEMIBOLD) - 5 - ARROW_SMALL * 1.12;
    arrow(canvas, arrowX, baseline - 1.5, ARROW_SMALL, HN_ORANGE);
    rowTop += rowHeight(row.lines.length, layout.rowPad);
  });
}

/** No stories is a real state for `new` on a quiet minute, so it is composed. */
function drawEmpty(canvas: Canvas): void {
  canvas.roundedRect(MARGIN, TOP, CONTENT_WIDTH, HEIGHT, RADIUS_MODULE, HERO_GROUND);
  sourceRow(canvas, "Hacker News", "");
  const x = MARGIN + HERO_PAD_X;
  const middle = TOP + HEIGHT / 2;
  canvas.text({
    x,
    baseline: middle,
    content: "No stories yet",
    size: 40,
    fill: INK,
    weight: WEIGHT_SEMIBOLD,
  });
  canvas.text({
    x,
    baseline: middle + 34,
    content: "The list is reachable and empty",
    size: SIZE_BODY,
    fill: HERO_INK_2,
  });
}

export function renderHackerNews(face: HackerNewsFace): string {
  const canvas = new Canvas(CANVAS_WIDTH, CANVAS_HEIGHT);
  canvas.rect(0, 0, CANVAS_WIDTH, CANVAS_HEIGHT, GROUND);
  const [lead, ...rest] = face.stories;
  if (lead === undefined) {
    drawEmpty(canvas);
    return canvas.finish();
  }
  const layout = plan(lead, rest);
  drawHero(canvas, lead, layout, face.page);
  drawIndex(canvas, layout);
  return canvas.finish();
}

function isStoredStory(value: unknown): value is Story {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const story = value as Record<string, unknown>;
  return (
    Number.isInteger(story.rank) &&
    typeof story.title === "string" &&
    typeof story.points === "number" &&
    Number.isFinite(story.points) &&
    typeof story.comments === "number" &&
    Number.isFinite(story.comments) &&
    typeof story.domain === "string" &&
    typeof story.age === "string"
  );
}

function storedState(value: unknown): HackerNewsState | undefined {
  if (typeof value !== "object" || value === null) {
    return undefined;
  }
  const state = value as Record<string, unknown>;
  if (
    !Array.isArray(state.stories) ||
    !state.stories.every(isStoredStory) ||
    !Number.isInteger(state.page) ||
    (state.tappedAt !== null && typeof state.tappedAt !== "string")
  ) {
    return undefined;
  }
  return {
    stories: state.stories.slice(0, STORED_STORIES),
    page: Math.max(0, state.page as number),
    tappedAt: state.tappedAt as string | null,
  };
}

const pageCount = (stories: Story[]): number =>
  Math.max(1, Math.ceil(stories.length / MAX_STORIES));

function normalizedPage(page: number, stories: Story[]): number {
  return page % pageCount(stories);
}

function pageViews(stories: Story[]): ViewId[] {
  return Array.from({ length: pageCount(stories) }, (_, page) =>
    page === 0 ? "" : `page-${page + 1}`,
  );
}

function pageForView(view: ViewId | undefined, stories: Story[]): number {
  const page = pageViews(stories).indexOf(view ?? "");
  return page === -1 ? 0 : page;
}

function recentTap(tappedAt: string | null, now: Date): boolean {
  if (tappedAt === null) {
    return false;
  }
  const instant = Date.parse(tappedAt);
  return Number.isFinite(instant) && now.getTime() - instant <= TEMPORARY_VIEW_MS;
}

function drawState(state: HackerNewsState): string {
  const count = pageCount(state.stories);
  const page = normalizedPage(state.page, state.stories);
  return renderHackerNews({
    stories: state.stories.slice(page * MAX_STORIES, (page + 1) * MAX_STORIES),
    page: { index: page, count },
  });
}

export async function renderHackerNewsRequest(
  settings: Settings,
  now: Date,
  context: RenderContext = {},
  get: FetchText = fetchText,
): Promise<{ svg: string; state: HackerNewsState }> {
  const previous = storedState(context.state);
  // A render that is TOLD which view to draw must not fetch: answering a tap
  // from what is already in hand is the entire point of the views split. The
  // event is still honoured beside it, because the server and this package
  // deploy independently and a server that sends a tap without a view is the
  // one running right now.
  if ((context.view !== undefined || context.event !== undefined) && previous !== undefined) {
    return {
      svg: drawState({ ...previous, page: pageForView(context.view, previous.stories) }),
      state: previous,
    };
  }

  const fresh = await fetchHackerNews(settings, now, get);
  const keepPage = previous !== undefined && recentTap(previous.tappedAt, now);
  const state: HackerNewsState = {
    stories: fresh.stories,
    page: keepPage ? normalizedPage(previous.page, fresh.stories) : 0,
    tappedAt: keepPage ? previous.tappedAt : null,
  };
  // A scheduled refresh names no view, and must draw the page the state above
  // just decided -- that is where "a tap holds its page for ten minutes" lives.
  // Reading an absent view as page zero threw that decision away.
  const page = context.view === undefined ? state.page : pageForView(context.view, fresh.stories);
  return { svg: drawState({ ...state, page }), state };
}

export const hackernews: FaceDefinition = {
  kind: "hackernews",
  label: "Hacker News",
  tap: "Tap the panel for the next stories.",
  fields: [
    {
      type: "enum",
      key: "list",
      label: "Stories",
      default: "top",
      options: [
        { value: "top", label: "Front page" },
        { value: "best", label: "Best" },
        { value: "new", label: "Newest" },
        { value: "ask", label: "Ask HN" },
        { value: "show", label: "Show HN" },
      ],
    },
  ],
  views(_settings, value) {
    const state = storedState(value);
    return state === undefined ? [""] : pageViews(state.stories);
  },
  onTap(_settings, value, event, now) {
    const previous = storedState(value);
    if (previous === undefined) {
      return { view: "" };
    }
    const page = normalizedPage(previous.page + event.taps, previous.stories);
    const state: HackerNewsState = {
      ...previous,
      page,
      tappedAt: now.toISOString(),
    };
    return { view: pageViews(state.stories)[page] ?? "", state };
  },
  render(settings, now, context) {
    return renderHackerNewsRequest(settings, now, context);
  },
};
