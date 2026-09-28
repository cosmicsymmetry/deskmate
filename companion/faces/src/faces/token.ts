// The token face: one price, its direction, and where it has been.
//
// # The anatomy is the weather face's
//
// A rounded hero module over a SURFACE chart module, on black. The hero is coloured BY
// the direction -- a deep green when the day is up, a deep red when it is down -- the way
// the weather hero is coloured by its condition: the glance answer ("up or down?") is
// the colour of the face, and the sign and the arrow say it again for anyone who cannot
// use the colour. The first design set everything loose on black, and read as a list of
// parts rather than as a face.
//
// # The price
//
// Currency mark and fraction at half the integer's size, all on one baseline: the
// ticker treatment, which lets the digits that change be as large as the module allows.
// The decimal point travels WITH the fraction. At half of an 84px numeral it is five
// pixels of ink and perfectly visible; set at the integer's size, as it first was, it
// was a bullet the size of a digit's counter floating between two numbers.
//
// # The chart is the owner's choice
//
// A smoothed line, or candles. Both are drawn at full resolution, which is the clearest
// thing rastering buys: a scene-native line is capped at 8 points. Candles are also the
// cheaper frame on the wire -- axis-aligned rectangles run-length encode almost for
// free, where an anti-aliased curve is a run of one at every edge pixel.

import {
  ConfigurationError,
  type FaceDefinition,
  type RenderContext,
  type Settings,
  type ViewId,
  text,
  TransientError,
  truncateUtf8,
} from "../face";
import { type FetchText, fetchText } from "../kit/http";
import { Canvas, fitTracked, fixed, normalizeWhitespace, textInk, textWidth } from "../kit/svg";
import {
  BAD,
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
  RADIUS_MODULE,
  SIZE_CAPTION,
  SIZE_EYEBROW,
  SIZE_SUBHEAD,
  SURFACE,
  TRACKING_EYEBROW,
  WEIGHT_REGULAR,
  WEIGHT_SEMIBOLD,
} from "../kit/theme";

// ---------------------------------------------------------------------------
// The view model. The face does no arithmetic on prices beyond layout: the
// direction and the percentage arrive decided.
// ---------------------------------------------------------------------------

/** How the price history is drawn, or that it is not drawn at all. */
export type ChartStyle = "line" | "candles" | "none";

export interface TokenFace {
  /** The ticker, e.g. "SOL". */
  symbol: string;
  /** The long name, e.g. "Solana". */
  name: string;
  /** The quote currency's ISO code, e.g. "USD". */
  currency: string;
  /** The currency's mark, e.g. "$". Empty falls back to the code alone. */
  currencyMark: string;
  price: number;
  /** Change over the window, in percent. Sign carries the direction. */
  changePercent: number;
  low: number;
  high: number;
  /** Oldest to newest. Fewer than two points draws no line. */
  series: number[];
  /** Oldest to newest. Drawn instead of the line when `chart` is "candles". */
  candles: Candle[];
  chart: ChartStyle;
  /** CoinGecko's id for the coin the ticker resolved to: what the history requests need. */
  coinId: string;
}

export interface Candle {
  open: number;
  high: number;
  low: number;
  close: number;
}

// ---------------------------------------------------------------------------
// Data: CoinGecko. Two requests per refresh, and only the first is required. A
// rate-limited `market_chart` leaves a face with a price and no sparkline, which
// is a worse face but a true one; refusing the whole refresh would replace a
// correct price with a stale one because a decoration was unavailable.
// ---------------------------------------------------------------------------

const MARKETS_ENDPOINT = "https://api.coingecko.com/api/v3/coins/markets";
const CHART_ENDPOINT_PREFIX = "https://api.coingecko.com/api/v3/coins/";
const MAX_SERIES_SAMPLES = 96;
const MAX_CANDLES = 48;
const MAX_PRICE = 1e12;

function currencyMark(code: string): string {
  switch (code.toUpperCase()) {
    case "USD":
      return "$";
    case "EUR":
      return "€";
    case "GBP":
      return "£";
    case "JPY":
    case "CNY":
      return "¥";
    case "AED":
      return "د.إ";
    case "BTC":
      return "₿";
    default:
      return "";
  }
}

function endpoint(base: string, pairs: [string, string][], apiKey: string): string {
  const url = new URL(base);
  for (const [key, value] of pairs) {
    url.searchParams.append(key, value);
  }
  if (apiKey !== "") {
    url.searchParams.append("x_cg_demo_api_key", apiKey);
  }
  return url.toString();
}

function boundedPrice(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 && value <= MAX_PRICE
    ? value
    : undefined;
}

/** `undefined` when CoinGecko has no coin with that id: it answers `[]`, not an error. */
export function parseMarkets(body: string, currency: string): TokenFace | undefined {
  let document: unknown;
  try {
    document = JSON.parse(body);
  } catch {
    throw new TransientError("the token market JSON is invalid");
  }
  const entry: unknown = Array.isArray(document) ? document[0] : undefined;
  if (typeof entry !== "object" || entry === null) {
    return undefined;
  }
  const fields = entry as Record<string, unknown>;
  const price = boundedPrice(fields.current_price);
  if (price === undefined) {
    throw new TransientError("the token price is missing or outside supported bounds");
  }
  const high = boundedPrice(fields.high_24h) ?? price;
  const low = boundedPrice(fields.low_24h) ?? price;
  const rawChange =
    fields.price_change_percentage_24h_in_currency ?? fields.price_change_percentage_24h;
  const change = typeof rawChange === "number" && Number.isFinite(rawChange) ? rawChange : 0;
  const code = currency.toUpperCase();
  return {
    symbol: truncateUtf8(typeof fields.symbol === "string" ? fields.symbol : "", 12),
    name: truncateUtf8(typeof fields.name === "string" ? fields.name : "", 32),
    currency: code,
    currencyMark: currencyMark(code),
    price,
    changePercent: Math.min(100_000, Math.max(-100_000, change)),
    high: Math.max(high, low),
    low: Math.min(high, low),
    series: [],
    candles: [],
    chart: "line",
    coinId: typeof fields.id === "string" && /^[a-z0-9_.-]+$/i.test(fields.id) ? fields.id : "",
  };
}

export function parseChart(body: string): number[] {
  const document: unknown = JSON.parse(body);
  const points =
    typeof document === "object" && document !== null
      ? (document as Record<string, unknown>).prices
      : undefined;
  if (!Array.isArray(points)) {
    throw new TransientError("the token chart has no prices");
  }
  const prices = points
    .map((point: unknown) => (Array.isArray(point) ? boundedPrice(point[1]) : undefined))
    .filter((price): price is number => price !== undefined);
  return downsample(prices, MAX_SERIES_SAMPLES);
}

/** CoinGecko's `ohlc` rows are `[time, open, high, low, close]`; one day is 48 half-hours. */
export function parseCandles(body: string): Candle[] {
  const rows: unknown = JSON.parse(body);
  if (!Array.isArray(rows)) {
    throw new TransientError("the token candles are not a list");
  }
  return rows
    .map((row: unknown): Candle | undefined => {
      const [, open, high, low, close] = (Array.isArray(row) ? row : []).map(boundedPrice);
      return open === undefined || high === undefined || low === undefined || close === undefined
        ? undefined
        : // A feed that inverts a pair must not draw a wick inside out.
          {
            open,
            close,
            high: Math.max(open, high, low, close),
            low: Math.min(open, high, low, close),
          };
    })
    .filter((candle): candle is Candle => candle !== undefined)
    .slice(-MAX_CANDLES);
}

function downsample(prices: number[], target: number): number[] {
  if (prices.length <= target || target < 2) {
    return prices;
  }
  const last = prices.length - 1;
  return Array.from({ length: target }, (_, index) => {
    const position = Math.floor((index * last) / (target - 1));
    return prices[position] ?? 0;
  });
}

/**
 * The field takes a TICKER and nothing else, on the owner's direction (2026-09-21): SOL,
 * not "Solana" and not CoinGecko's id. One vocabulary, the one printed on the face.
 *
 * `symbols=` on the markets endpoint answers with the best-ranked coin carrying that
 * ticker -- SOL is Solana, never a wrapped or bridged namesake -- in the same single
 * request that returns the price, so a ticker costs nothing over an id.
 */
export async function fetchToken(settings: Settings, get: FetchText): Promise<TokenFace> {
  const ticker = text(settings, "coin_id");
  const currency = (text(settings, "currency") || "usd").toLowerCase();
  const apiKey = text(settings, "api_key");
  if (ticker === "") {
    throw new ConfigurationError("the ticker is empty");
  }
  if (!/^[A-Za-z0-9]{1,12}$/.test(ticker)) {
    throw new ConfigurationError(
      `"${ticker.slice(0, 24)}" is not a ticker; use one like SOL or BTC`,
    );
  }
  if (!/^[a-z0-9]+$/.test(currency)) {
    throw new ConfigurationError("the currency is a short code such as usd");
  }

  const face = parseMarkets(
    await get(
      endpoint(
        MARKETS_ENDPOINT,
        [
          ["vs_currency", currency],
          ["symbols", ticker.toLowerCase()],
          ["price_change_percentage", "24h"],
        ],
        apiKey,
      ),
    ),
    currency,
  );
  // An unknown ticker is answered with [], not an error. A NAME lands here too --
  // "solana" is a well-formed ticker that nothing trades under -- so say what to type.
  if (face === undefined || face.symbol.toLowerCase() !== ticker.toLowerCase()) {
    throw new ConfigurationError(
      `no coin trades as ${ticker.toUpperCase()}; enter the ticker, e.g. SOL for Solana`,
    );
  }

  // The chart is a decoration on a true price: a refused second request leaves a face
  // with a price and an empty chart, never a stale price.
  const chosen = text(settings, "chart");
  face.chart = chosen === "candles" || chosen === "none" ? chosen : "line";
  if (face.chart === "none" || face.coinId === "") {
    return face;
  }
  const coinId = face.coinId;
  const history = (path: string): Promise<string> =>
    get(
      endpoint(
        `${CHART_ENDPOINT_PREFIX}${coinId}/${path}`,
        [
          ["vs_currency", currency],
          ["days", "1"],
        ],
        apiKey,
      ),
    );
  try {
    if (face.chart === "candles") {
      face.candles = parseCandles(await history("ohlc"));
    } else {
      face.series = parseChart(await history("market_chart"));
    }
  } catch {
    face.series = [];
    face.candles = [];
  }
  return face;
}

// ---------------------------------------------------------------------------
// Drawing.
// ---------------------------------------------------------------------------

/** Deep, unsaturated grounds: a module of colour on an emissive panel, never a lamp. */
const GROUND_RISING = "#0c2a1b";
const GROUND_FALLING = "#2f1413";
/** The label ink, warmed or cooled to sit on its ground instead of reading as grey fog. */
const MUTED_RISING = "#8fb8a1";
const MUTED_FALLING = "#c49a96";

const HERO_TOP = MARGIN;
const HERO_HEIGHT = 164;
const HERO_PAD = 2.5 * GRID;
const CHART_TOP = HERO_TOP + HERO_HEIGHT + 2 * GRID;
const CHART_HEIGHT = CANVAS_HEIGHT - MARGIN - CHART_TOP;
const CHART_PAD = 2 * GRID;
const INNER_WIDTH = CONTENT_WIDTH - 2 * HERO_PAD;

/** The fraction and the currency mark, relative to the integer part. */
const FRACTION_RATIO = 0.5;
const PRICE_STEPS = [84, 72, 64, 56, 48, 40] as const;
const LINE_STROKE = 2.5;
const AREA_OPACITY = 0.14;
/** A day of 5-minute closes is noise at this size; the line is drawn through this many. */
const LINE_POINTS = 40;
const CHIP_HEIGHT = 30;

const rising = (face: TokenFace): boolean => face.changePercent >= 0;
const directionColor = (face: TokenFace): string => (rising(face) ? GOOD : BAD);
const f2 = (value: number): string => fixed(value, 2);

export function renderToken(face: TokenFace): string {
  const canvas = new Canvas(CANVAS_WIDTH, CANVAS_HEIGHT);
  canvas.rect(0, 0, CANVAS_WIDTH, CANVAS_HEIGHT, GROUND);
  if (face.chart === "none") {
    drawSimple(canvas, face);
    return canvas.finish();
  }
  drawHero(canvas, face);
  drawChart(canvas, face);
  return canvas.finish();
}

function drawHero(canvas: Canvas, face: TokenFace): void {
  canvas.roundedRect(
    MARGIN,
    HERO_TOP,
    CONTENT_WIDTH,
    HERO_HEIGHT,
    RADIUS_MODULE,
    rising(face) ? GROUND_RISING : GROUND_FALLING,
  );
  const left = MARGIN + HERO_PAD;
  const right = MARGIN + CONTENT_WIDTH - HERO_PAD;
  const muted = rising(face) ? MUTED_RISING : MUTED_FALLING;

  // Row one: what it is, and how the day went. The chip sits on the row's centre line.
  const rowCenter = HERO_TOP + HERO_PAD + CHIP_HEIGHT / 2;
  const chipWidth = drawChangeChip(canvas, face, right, rowCenter);
  const symbol = face.symbol.toUpperCase();
  const baseline = baselineFromCenter(rowCenter, SIZE_SUBHEAD);
  canvas.text({
    x: left,
    baseline,
    content: symbol,
    size: SIZE_SUBHEAD,
    fill: INK,
    weight: WEIGHT_SEMIBOLD,
  });
  const nameX = left + textWidth(symbol, SIZE_SUBHEAD, WEIGHT_SEMIBOLD) + 1.25 * GRID;
  const nameRoom = right - chipWidth - 1.5 * GRID - nameX;
  const name = normalizeWhitespace(face.name).toUpperCase();
  if (name !== "" && name !== symbol && nameRoom > 4 * GRID) {
    canvas.text({
      x: nameX,
      // On the symbol's own baseline, so the two runs sit on one line despite the sizes.
      baseline,
      content: fitTracked(name, SIZE_EYEBROW, WEIGHT_SEMIBOLD, TRACKING_EYEBROW, nameRoom),
      size: SIZE_EYEBROW,
      fill: muted,
      weight: WEIGHT_SEMIBOLD,
      tracking: TRACKING_EYEBROW,
    });
  }

  drawPrice(
    canvas,
    face,
    {
      anchor: left,
      centred: false,
      bottom: HERO_TOP + HERO_HEIGHT - HERO_PAD - 2,
      room: INNER_WIDTH,
    },
    PRICE_STEPS,
    { mark: muted, numerals: INK, fraction: INK_2 },
  );
}

/** The pill at the right of row one. Returns its width so the name can be fitted beside it. */
function drawChangeChip(canvas: Canvas, face: TokenFace, right: number, center: number): number {
  const color = directionColor(face);
  // The sign is printed as well as coloured: a red number with no minus is
  // indistinguishable from a green one in a greyscale screenshot.
  const label = `${rising(face) ? "+" : "\u2212"}${f2(Math.abs(face.changePercent))}%`;
  const triangleWidth = 9;
  const width =
    1.5 * GRID +
    triangleWidth +
    GRID +
    textWidth(label, SIZE_CAPTION, WEIGHT_SEMIBOLD) +
    1.5 * GRID;
  const x = right - width;
  canvas.rectOpacity(x, center - CHIP_HEIGHT / 2, width, CHIP_HEIGHT, CHIP_HEIGHT / 2, color, 0.2);
  canvas.path(triangle(x + 1.5 * GRID, center, triangleWidth, rising(face)), color);
  canvas.text({
    x: right - 1.5 * GRID,
    baseline: baselineFromCenter(center, SIZE_CAPTION),
    content: label,
    size: SIZE_CAPTION,
    fill: color,
    weight: WEIGHT_SEMIBOLD,
    anchor: "end",
  });
  return width;
}

/** An equilateral-ish triangle pointing up or down, centred vertically on `cy`. */
function triangle(x: number, cy: number, width: number, up: boolean): string {
  const half = width / 2;
  const height = width * 0.86;
  const base = up ? cy + height / 2 : cy - height / 2;
  const apex = up ? cy - height / 2 : cy + height / 2;
  return `M${f2(x)} ${f2(base)}L${f2(x + half)} ${f2(apex)}L${f2(x + width)} ${f2(base)}Z`;
}

/** The three inks of a price: its currency mark, its numerals, and its fraction. */
interface PriceTones {
  mark: string;
  numerals: string;
  fraction: string;
}

interface PlacedRun {
  content: string;
  size: number;
  fill: string;
  /** Pen position relative to the assembly's left INK edge, and baseline relative to the price's. */
  x: number;
  dy: number;
}

/**
 * The price as placed runs, composed by INK rather than by advance, plus its inked width.
 *
 * Three things an advance-based layout gets wrong, all of them visible at 84px:
 * - The assembly's left edge. A pen at the margin puts a glyph's ink a bearing inside
 *   it, so the price sat visibly right of the symbol above it. The first run's ink
 *   starts AT the margin here.
 * - The gaps. Every glyph carries its own side bearings, so two runs set a fixed
 *   advance apart are a different distance apart for every price, and at 84px the
 *   numeral's flag came within a pixel of the mark. Gaps are ink to ink.
 * - The mark's height. A "$" is taller than its own S -- the stroke overshoots the cap
 *   line and the baseline -- so hanging its top from the numerals' cap line set the S
 *   itself low and the stroke proud of everything. The S is what the eye aligns,
 *   so the S's top meets the numerals' top and the stroke is left to overshoot, the way
 *   it does in running text.
 */
function composePrice(
  face: TokenFace,
  size: number,
  tones: PriceTones,
): { runs: PlacedRun[]; width: number } {
  const [integer, fraction] = splitPrice(face.price);
  // Below a whole unit the integer is the single character "0" and every digit that
  // matters is in the fraction; shrinking it would set the one meaningless glyph large.
  const uniform = Math.abs(face.price) < 1 && fraction !== "";
  const small = size * FRACTION_RATIO;
  const numerals = uniform ? `${integer}${fraction}` : integer;
  const numeralInk = textInk(numerals, size, WEIGHT_SEMIBOLD);

  const runs: PlacedRun[] = [];
  let cursor = 0; // the right INK edge of what has been placed so far
  if (face.currencyMark !== "") {
    const mark = face.currencyMark;
    const markInk = textInk(mark, small, WEIGHT_SEMIBOLD);
    // The body the eye aligns: the S of a dollar sign, the mark itself otherwise.
    const body = textInk(mark === "$" ? "S" : mark, small, WEIGHT_SEMIBOLD);
    runs.push({
      content: mark,
      size: small,
      fill: tones.mark,
      x: -markInk.left,
      dy: textInk("0", size, WEIGHT_SEMIBOLD).top - body.top,
    });
    cursor = markInk.right - markInk.left + size * 0.055;
  }
  runs.push({ content: numerals, size, fill: tones.numerals, x: cursor - numeralInk.left, dy: 0 });
  cursor += numeralInk.right - numeralInk.left;
  if (!uniform && fraction !== "") {
    const fractionInk = textInk(fraction, small, WEIGHT_SEMIBOLD);
    cursor += size * 0.035;
    runs.push({
      content: fraction,
      size: small,
      fill: tones.fraction,
      x: cursor - fractionInk.left,
      dy: 0,
    });
    cursor += fractionInk.right - fractionInk.left;
  }
  return { runs, width: cursor };
}

/**
 * Draws the price with its baseline `bottom`, at the largest of `steps` that fits
 * `room`. `anchor` is the assembly's left ink edge, or its centre.
 */
function drawPrice(
  canvas: Canvas,
  face: TokenFace,
  place: { anchor: number; centred: boolean; bottom: number; room: number },
  steps: readonly number[],
  tones: PriceTones,
): number {
  // Fit the whole assembly, not just the integer: the mark and the fraction are what
  // push a five-figure price over the edge.
  const size =
    steps.find((step) => composePrice(face, step, tones).width <= place.room) ?? steps.at(-1) ?? 40;
  const { runs, width } = composePrice(face, size, tones);
  const left = place.centred ? place.anchor - width / 2 : place.anchor;
  for (const run of runs) {
    canvas.text({
      x: left + run.x,
      baseline: place.bottom + run.dy,
      content: run.content,
      size: run.size,
      fill: run.fill,
      weight: WEIGHT_SEMIBOLD,
    });
  }
  return size;
}

/** The simple face keeps the hero's price steps and adds the ones a whole canvas has room for. */
const SIMPLE_STEPS = [120, 104, 92, ...PRICE_STEPS] as const;
const SIMPLE_TICKER_SIZE = 34;

/** The direction colours taken down for the small runs, flat: a dimmer ink, not an opacity. */
const GOOD_DIM = "#1f8f3e";
const BAD_DIM = "#b3342c";

/**
 * The chart-free face: the price in the dead centre of a black panel, the ticker
 * slightly above it, and nothing else to read.
 *
 * The owner's direction, 2026-09-21: a dark ground, the NUMBER green or red by the
 * day, centred. So the colour moved from the ground to the figure -- on an emissive
 * panel a dark ground is unlit pixels, and the price is then the only lit thing on
 * the desk. It is the price's own ink that is centred on the canvas, in both axes: a
 * pair centred as a block put the figure visibly below the middle. The ticker hangs
 * above it and is deliberately outside that balance. The small arrow stays beside
 * the ticker because colour may not be the only carrier of the direction.
 */
function drawSimple(canvas: Canvas, face: TokenFace): void {
  const centerX = CANVAS_WIDTH / 2;
  const centerY = CANVAS_HEIGHT / 2;
  const tones: PriceTones = rising(face)
    ? { mark: GOOD_DIM, numerals: GOOD, fraction: GOOD_DIM }
    : { mark: BAD_DIM, numerals: BAD, fraction: BAD_DIM };

  const size =
    SIMPLE_STEPS.find((step) => composePrice(face, step, tones).width <= CONTENT_WIDTH) ??
    SIMPLE_STEPS.at(-1) ??
    40;
  // Centre the digits' INK, not their em box: the box carries descender room the
  // numerals never use, which is exactly the few pixels of "slightly off".
  const digits = textInk("0", size, WEIGHT_SEMIBOLD);
  const baseline = centerY - (digits.top + digits.bottom) / 2;
  drawPrice(
    canvas,
    face,
    { anchor: centerX, centred: true, bottom: baseline, room: CONTENT_WIDTH },
    [size],
    tones,
  );

  const symbol = face.symbol.toUpperCase();
  const arrow = 13;
  const symbolInk = textInk(symbol, SIMPLE_TICKER_SIZE, WEIGHT_SEMIBOLD);
  const symbolWidth = symbolInk.right - symbolInk.left;
  const rowLeft = centerX - (arrow + 1.25 * GRID + symbolWidth) / 2;
  const tickerBaseline = baseline + digits.top - 3 * GRID;
  const tickerCap = SIMPLE_TICKER_SIZE * CAP_HEIGHT;
  canvas.path(
    triangle(rowLeft, tickerBaseline - tickerCap / 2, arrow, rising(face)),
    directionColor(face),
  );
  canvas.text({
    x: rowLeft + arrow + 1.25 * GRID - symbolInk.left,
    baseline: tickerBaseline,
    content: symbol,
    size: SIMPLE_TICKER_SIZE,
    fill: INK_2,
    weight: WEIGHT_SEMIBOLD,
  });
}

function drawChart(canvas: Canvas, face: TokenFace): void {
  canvas.roundedRect(MARGIN, CHART_TOP, CONTENT_WIDTH, CHART_HEIGHT, RADIUS_MODULE, SURFACE);
  const left = MARGIN + CHART_PAD;
  const right = MARGIN + CONTENT_WIDTH - CHART_PAD;

  // The footer: the window, and the range the price moved in.
  const footer = CHART_TOP + CHART_HEIGHT - CHART_PAD;
  canvas.text({
    x: left,
    baseline: footer,
    content: "24H",
    size: SIZE_EYEBROW,
    fill: INK_3,
    weight: WEIGHT_SEMIBOLD,
    tracking: TRACKING_EYEBROW,
  });
  const mark = face.currencyMark;
  const range = [
    { label: "H", value: `${mark}${compactPrice(face.high)}` },
    { label: "L", value: `${mark}${compactPrice(face.low)}` },
  ];
  let x = right;
  for (const { label, value } of range) {
    canvas.text({
      x,
      baseline: footer,
      content: value,
      size: SIZE_CAPTION,
      fill: INK_2,
      anchor: "end",
    });
    x -= textWidth(value, SIZE_CAPTION, WEIGHT_REGULAR) + 0.75 * GRID;
    canvas.text({
      x,
      baseline: footer,
      content: label,
      size: SIZE_EYEBROW,
      fill: INK_3,
      weight: WEIGHT_SEMIBOLD,
      anchor: "end",
    });
    x -= textWidth(label, SIZE_EYEBROW, WEIGHT_SEMIBOLD) + 2 * GRID;
  }

  const plot = {
    left,
    right,
    top: CHART_TOP + CHART_PAD,
    bottom: footer - SIZE_CAPTION * CAP_HEIGHT - 1.75 * GRID,
  };
  if (face.chart === "candles" && face.candles.length >= 2) {
    drawCandles(canvas, face.candles, plot);
  } else if (face.chart === "line" && face.series.length >= 2) {
    drawLine(canvas, face, plot);
  } else {
    // Not an error: a freshly configured card has no history yet, and a refused chart
    // request leaves a true price. A hairline reads as "no series", not as a flat one.
    canvas.rect(left, (plot.top + plot.bottom) / 2, right - left, 1, HAIRLINE);
  }
}

interface Plot {
  left: number;
  right: number;
  top: number;
  bottom: number;
}

/** Maps a price onto the plot, centring a perfectly flat series instead of pinning it. */
function scale(plot: Plot, minimum: number, maximum: number): (value: number) => number {
  const span = maximum - minimum;
  return (value) =>
    span <= Number.EPSILON
      ? (plot.top + plot.bottom) / 2
      : plot.bottom - ((value - minimum) / span) * (plot.bottom - plot.top);
}

function drawLine(canvas: Canvas, face: TokenFace, plot: Plot): void {
  // Averaged into buckets rather than sampled: a sampled line keeps every spike it
  // happens to land on and drops the ones it does not, which is noise by another name.
  const buckets = Math.min(LINE_POINTS, face.series.length);
  const values = Array.from({ length: buckets }, (_, index) => {
    const from = Math.floor((index * face.series.length) / buckets);
    const to = Math.max(from + 1, Math.floor(((index + 1) * face.series.length) / buckets));
    const slice = face.series.slice(from, to);
    return slice.reduce((total, value) => total + value, 0) / slice.length;
  });
  // The newest point is the quoted price, not an average that lags it.
  values[values.length - 1] = face.series.at(-1) ?? face.price;

  const y = scale(plot, Math.min(...values), Math.max(...values));
  const step = (plot.right - plot.left) / (values.length - 1);
  const points = values.map((value, index): [number, number] => [
    plot.left + index * step,
    y(value),
  ]);

  // Catmull-Rom through the points, as cubic Beziers: the curve passes through every
  // value, so nothing is invented, and the corners a polyline makes are gone. The
  // control points are clamped to the plot so an overshoot cannot leave the module.
  const clamp = (value: number): number => Math.min(plot.bottom, Math.max(plot.top, value));
  let stroke = `M${f2(points[0]?.[0] ?? 0)} ${f2(points[0]?.[1] ?? 0)}`;
  for (let index = 0; index < points.length - 1; index += 1) {
    const [p0, p1, p2, p3] = [index - 1, index, index + 1, index + 2].map(
      (at) => points[Math.min(points.length - 1, Math.max(0, at))] ?? [0, 0],
    ) as [[number, number], [number, number], [number, number], [number, number]];
    stroke +=
      `C${f2(p1[0] + (p2[0] - p0[0]) / 6)} ${f2(clamp(p1[1] + (p2[1] - p0[1]) / 6))} ` +
      `${f2(p2[0] - (p3[0] - p1[0]) / 6)} ${f2(clamp(p2[1] - (p3[1] - p1[1]) / 6))} ` +
      `${f2(p2[0])} ${f2(p2[1])}`;
  }

  const color = directionColor(face);
  canvas.filledPathOpacity(
    `${stroke}L${f2(plot.right)} ${f2(plot.bottom)}L${f2(plot.left)} ${f2(plot.bottom)}Z`,
    color,
    AREA_OPACITY,
  );
  canvas.strokedPath(stroke, color, LINE_STROKE);

  // The newest point gets a marker, so the eye lands on "now" rather than on whichever
  // peak happens to be tallest.
  const [lastX, lastY] = points.at(-1) ?? [plot.right, plot.bottom];
  canvas.circle(lastX, lastY, LINE_STROKE + 2.5, SURFACE);
  canvas.circle(lastX, lastY, LINE_STROKE + 0.5, color);
}

function drawCandles(canvas: Canvas, candles: Candle[], plot: Plot): void {
  const y = scale(
    plot,
    Math.min(...candles.map((candle) => candle.low)),
    Math.max(...candles.map((candle) => candle.high)),
  );
  const step = (plot.right - plot.left) / candles.length;
  // Whole pixels: a body on a half-pixel boundary is two grey columns instead of one
  // crisp one, on the panel and on the wire.
  const body = Math.max(2, Math.round(step * 0.62));
  candles.forEach((candle, index) => {
    // Each candle is coloured by ITS half hour, not by the day: that is what a candle is.
    const up = candle.close >= candle.open;
    const color = up ? GOOD : BAD;
    const center = Math.round(plot.left + (index + 0.5) * step);
    const wickTop = y(candle.high);
    canvas.rect(center - 0.75, wickTop, 1.5, Math.max(1, y(candle.low) - wickTop), color);
    const top = y(Math.max(candle.open, candle.close));
    // A doji still gets a visible body: two pixels of "it went nowhere".
    const height = Math.max(2, y(Math.min(candle.open, candle.close)) - top);
    canvas.roundedRect(center - body / 2, top, body, height, 1, color);
  });
}

/**
 * Splits a price into its integer part (with thousands separators) and its
 * fractional part (decimal point included), choosing the decimals from the
 * magnitude. A token at 0.00004182 and one at 142.37 both have to be readable on
 * the same face, and two decimals would render the first as "0.00".
 */
export function splitPrice(price: number): [string, string] {
  const magnitude = Math.abs(price);
  const decimals = magnitude >= 1 ? 2 : magnitude >= 0.01 ? 4 : magnitude > 0 ? 8 : 2;

  const rendered = fixed(magnitude, decimals);
  const [integer = "", rawFraction = ""] = rendered.split(".");
  // "0.00003710" states a precision the quote does not have. Only the
  // high-precision branches are trimmed: $142.50 as $142.5 looks like a mistake.
  const fraction = decimals > 2 ? rawFraction.replace(/0+$/, "") : rawFraction;
  const grouped = `${price < 0 ? "-" : ""}${integer.replace(/\B(?=(\d{3})+(?!\d))/g, ",")}`;
  return [grouped, fraction === "" ? "" : `.${fraction}`];
}

/** The same number on one line, for the range labels. */
function compactPrice(price: number): string {
  return splitPrice(price).join("");
}

/**
 * What a tap remembers: which chart the owner asked for by touching the glass,
 * rather than by editing the card.
 *
 * Only the choice is stored, never the series. The points are the bulk of a
 * token face and would crowd the 16 KB the server allows per source, so a tap
 * pays its fetch again. That still skips the two costs that actually hurt --
 * the flash write and the compaction behind it.
 */
export interface TokenState {
  chart: ChartStyle;
  tappedAt: string | null;
}

/** The chart returns to the card's own setting after this long untouched. */
const TEMPORARY_CHART_MS = 10 * 60 * 1_000;

/**
 * The styles a tap moves between.
 *
 * `none` is deliberately not in the cycle: a face configured for no chart never
 * requests the series, so there would be nothing to draw on arrival.
 */
const TAPPABLE_CHARTS = ["line", "candles"] as const;

function configuredChart(settings: Settings): ChartStyle {
  const chosen = text(settings, "chart");
  return chosen === "candles" || chosen === "none" ? chosen : "line";
}

function orderedCharts(settings: Settings): ChartStyle[] {
  const configured = configuredChart(settings);
  return configured === "none"
    ? [configured]
    : [configured, ...TAPPABLE_CHARTS.filter((chart) => chart !== configured)];
}

function chartViews(settings: Settings): ViewId[] {
  return orderedCharts(settings).map((chart, index) => (index === 0 ? "" : chart));
}

function chartForView(settings: Settings, view: ViewId | undefined): ChartStyle {
  const charts = orderedCharts(settings);
  if (view === undefined || view === "") {
    return charts[0] ?? "line";
  }
  return charts.find((chart) => chart === view) ?? charts[0] ?? "line";
}

function viewForChart(settings: Settings, selected: ChartStyle): ViewId {
  const charts = orderedCharts(settings);
  const index = charts.indexOf(selected);
  return index <= 0 ? "" : chartViews(settings)[index] ?? "";
}

function storedState(value: unknown): TokenState | undefined {
  if (typeof value !== "object" || value === null) {
    return undefined;
  }
  const candidate = value as { chart?: unknown; tappedAt?: unknown };
  const chart = TAPPABLE_CHARTS.find((style) => style === candidate.chart);
  if (chart === undefined) {
    return undefined;
  }
  return {
    chart,
    tappedAt: typeof candidate.tappedAt === "string" ? candidate.tappedAt : null,
  };
}

function nextChart(current: ChartStyle, taps: number): ChartStyle {
  const index = TAPPABLE_CHARTS.indexOf(current as (typeof TAPPABLE_CHARTS)[number]);
  const from = index === -1 ? 0 : index;
  const next = (from + taps) % TAPPABLE_CHARTS.length;
  return TAPPABLE_CHARTS[(next + TAPPABLE_CHARTS.length) % TAPPABLE_CHARTS.length] as ChartStyle;
}

function recentTap(tappedAt: string | null, now: Date): boolean {
  if (tappedAt === null) {
    return false;
  }
  const at = Date.parse(tappedAt);
  return Number.isFinite(at) && now.getTime() - at <= TEMPORARY_CHART_MS;
}

export async function renderTokenRequest(
  settings: Settings,
  now: Date,
  context: RenderContext = {},
  get: FetchText = fetchText,
): Promise<{ svg: string; state: TokenState | null }> {
  // The style has to be settled BEFORE fetching, not after: a line reads
  // `market_chart` and candles read `ohlc`, never both, so overriding the face
  // afterwards would draw one style from the other's data.
  const configured = configuredChart(settings);
  // A card set to "none" requests no series at all, so there is nothing for a
  // tap to switch to. Returning null state keeps a stale override from
  // reappearing if the owner later turns a chart back on.
  if (configured === "none") {
    return { svg: renderToken(await fetchToken(settings, get)), state: null };
  }

  const previous = storedState(context.state);
  let state: TokenState;
  if (
    previous !== undefined &&
    (context.event !== undefined || recentTap(previous.tappedAt, now))
  ) {
    state = previous;
  } else {
    state = { chart: configured, tappedAt: null };
  }
  const face = await fetchToken({ ...settings, chart: chartForView(settings, context.view) }, get);
  return { svg: renderToken(face), state };
}

export const token: FaceDefinition = {
  kind: "token",
  label: "Token price",
  tap: "Tap the panel to switch between the line and the candles.",
  fields: [
    // The key is `coin_id` because that is what spec files already hold. It takes a ticker.
    { type: "text", key: "coin_id", label: "Ticker", placeholder: "SOL" },
    { type: "text", key: "currency", label: "Currency", placeholder: "usd", default: "usd" },
    {
      type: "enum",
      key: "chart",
      label: "Chart",
      default: "line",
      options: [
        { value: "line", label: "Line" },
        { value: "candles", label: "Candles" },
        { value: "none", label: "None" },
      ],
    },
  ],
  views(settings) {
    return chartViews(settings);
  },
  onTap(settings, value, event) {
    const configured = configuredChart(settings);
    if (configured === "none") {
      return { view: "", state: null };
    }
    const previous = storedState(value);
    const chart = nextChart(previous?.chart ?? configured, event.taps);
    const state: TokenState = { chart, tappedAt: new Date().toISOString() };
    return { view: viewForChart(settings, chart), state };
  },
  async render(settings, now, context) {
    return renderTokenRequest(settings, now, context);
  },
};
