// The token face: one price, its direction, and where it has been.
//
// The price is set with its currency mark and its fractional part at about 42% of
// the integer part's size, all on one baseline. That is the conventional ticker
// treatment and it is not decoration: it lets the digits that actually change size
// be as large as the canvas allows while keeping the cents legible.
//
// The sparkline is drawn at full resolution, which is the clearest thing rastering
// buys: a scene-native line is capped at 8 points, so a day of prices would arrive
// as seven straight segments.

import {
  ConfigurationError,
  type FaceDefinition,
  type Settings,
  text,
  TransientError,
  truncateUtf8,
} from "../face";
import { type FetchText, fetchText } from "../kit/http";
import {
  Canvas,
  fitTracked,
  fixed,
  normalizeWhitespace,
  textWidth,
  trackedWidth,
} from "../kit/svg";
import {
  BAD,
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
  HERO_STEPS,
  INK,
  INK_2,
  INK_3,
  MARGIN,
  RADIUS_CHIP,
  SIZE_CAPTION,
  SIZE_EYEBROW,
  SIZE_SUBHEAD,
  TRACKING_EYEBROW,
  WEIGHT_REGULAR,
  WEIGHT_SEMIBOLD,
} from "../kit/theme";

// ---------------------------------------------------------------------------
// The view model. The face does no arithmetic on prices beyond layout: the
// direction and the percentage arrive decided.
// ---------------------------------------------------------------------------

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
  /** Oldest to newest. Fewer than two points draws no sparkline. */
  series: number[];
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

export function parseMarkets(body: string, currency: string): TokenFace {
  let document: unknown;
  try {
    document = JSON.parse(body);
  } catch {
    throw new TransientError("the token market JSON is invalid");
  }
  const entry: unknown = Array.isArray(document) ? document[0] : undefined;
  if (typeof entry !== "object" || entry === null) {
    // CoinGecko answers an unknown id with an empty array, so this is the owner's
    // typo far more often than it is an outage.
    throw new ConfigurationError("the token was not found; check the coin ID");
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

export async function fetchToken(settings: Settings, get: FetchText): Promise<TokenFace> {
  const coinId = text(settings, "coin_id");
  const currency = (text(settings, "currency") || "usd").toLowerCase();
  const apiKey = text(settings, "api_key");
  if (coinId === "") {
    throw new ConfigurationError("the coin ID is empty");
  }
  if (!/^[a-z0-9-]+$/.test(coinId)) {
    throw new ConfigurationError(
      "the coin ID may use only lowercase letters, digits and hyphens (CoinGecko's id, e.g. solana -- not the ticker)",
    );
  }
  if (!/^[a-z0-9]+$/.test(currency)) {
    throw new ConfigurationError("the currency is a short code such as usd");
  }

  const markets = await get(
    endpoint(
      MARKETS_ENDPOINT,
      [
        ["vs_currency", currency],
        ["ids", coinId],
        ["price_change_percentage", "24h"],
      ],
      apiKey,
    ),
  );
  const face = parseMarkets(markets, currency);

  try {
    const chart = await get(
      endpoint(
        `${CHART_ENDPOINT_PREFIX}${coinId}/market_chart`,
        [
          ["vs_currency", currency],
          ["days", "1"],
        ],
        apiKey,
      ),
    );
    face.series = parseChart(chart);
  } catch {
    face.series = [];
  }
  return face;
}

// ---------------------------------------------------------------------------
// Drawing.
// ---------------------------------------------------------------------------

/** The fractional part and currency mark, relative to the integer part. */
const FRACTION_RATIO = 0.42;
const SPARKLINE_TOP = 190;
const SPARKLINE_HEIGHT = 96;
/** Headroom so a flat line is not drawn on the boundary and a peak is not clipped. */
const SPARKLINE_PADDING = 8;
const SPARKLINE_STROKE = 2.5;
const AREA_OPACITY = 0.16;
const CHIP_HEIGHT = 32;

const rising = (face: TokenFace): boolean => face.changePercent >= 0;
const directionColor = (face: TokenFace): string => (rising(face) ? GOOD : BAD);
const f2 = (value: number): string => fixed(value, 2);

export function renderToken(face: TokenFace): string {
  const canvas = new Canvas(CANVAS_WIDTH, CANVAS_HEIGHT);
  canvas.rect(0, 0, CANVAS_WIDTH, CANVAS_HEIGHT, GROUND);

  drawIdentity(canvas, face);
  const priceBottom = drawPrice(canvas, face);
  drawChangeChip(canvas, face, priceBottom + 1.75 * GRID);
  drawSparkline(canvas, face);
  drawRange(canvas, face);

  return canvas.finish();
}

function drawIdentity(canvas: Canvas, face: TokenFace): void {
  const baseline = baselineFromCapTop(MARGIN, SIZE_SUBHEAD);
  const symbol = face.symbol.toUpperCase();
  canvas.text({
    x: MARGIN,
    baseline,
    content: symbol,
    size: SIZE_SUBHEAD,
    fill: INK,
    weight: WEIGHT_SEMIBOLD,
  });

  const symbolWidth = textWidth(symbol, SIZE_SUBHEAD, WEIGHT_SEMIBOLD);
  const nameX = MARGIN + symbolWidth + 1.5 * GRID;
  const currency = face.currency.toUpperCase();
  // Both of these runs are tracked, so both are measured with tracking.
  const currencyWidth = trackedWidth(currency, SIZE_EYEBROW, WEIGHT_SEMIBOLD, TRACKING_EYEBROW);
  const nameRoom = CANVAS_WIDTH - MARGIN - currencyWidth - 2 * GRID - nameX;

  const name = normalizeWhitespace(face.name).toUpperCase();
  if (name !== "" && nameRoom > 0) {
    canvas.text({
      x: nameX,
      // On the symbol's own baseline, not its box, so the two runs sit on one line
      // despite the size difference.
      baseline,
      content: fitTracked(name, SIZE_EYEBROW, WEIGHT_SEMIBOLD, TRACKING_EYEBROW, nameRoom),
      size: SIZE_EYEBROW,
      fill: INK_3,
      weight: WEIGHT_SEMIBOLD,
      tracking: TRACKING_EYEBROW,
    });
  }

  canvas.text({
    x: CANVAS_WIDTH - MARGIN,
    baseline,
    content: currency,
    size: SIZE_EYEBROW,
    fill: INK_3,
    weight: WEIGHT_SEMIBOLD,
    tracking: TRACKING_EYEBROW,
    anchor: "end",
  });
}

/**
 * The integer run and the fraction digits as they will be drawn.
 *
 * The decimal separator is set at the INTEGER's size, not the fraction's. A period
 * is a few pixels of ink: at 42% beside a 112px numeral it disappears, and "$101"
 * followed by a small "96" reads as ten thousand one hundred and ninety-six.
 */
function priceRuns(integer: string, fraction: string, uniform: boolean): [string, string] {
  if (!uniform && fraction.startsWith(".")) {
    return [`${integer}.`, fraction.slice(1)];
  }
  return [integer, fraction];
}

/** Draws the price and returns the y of its cap bottom. */
function drawPrice(canvas: Canvas, face: TokenFace): number {
  const [integer, fraction] = splitPrice(face.price);
  const mark = face.currencyMark;
  // Below a whole unit the integer part is the single character "0" and every digit
  // that matters lives in the fraction. Shrinking the fraction there sets the one
  // meaningless glyph large and the significant digits small, so a sub-unit price
  // is set at one size.
  const uniform = Math.abs(face.price) < 1 && fraction !== "";

  // Fit the whole assembly, not just the integer part: the mark and the fraction
  // are what push a four-digit price over the edge.
  const size = fitPriceSize(mark, integer, fraction, uniform);
  const fractionSize = uniform ? size : size * FRACTION_RATIO;
  const markSize = size * FRACTION_RATIO;
  const capTop = 62;
  const baseline = baselineFromCapTop(capTop, size);
  const [integerRun, fractionDigits] = priceRuns(integer, fraction, uniform);

  let x = MARGIN;
  if (mark !== "") {
    // Cap-top aligned, not baseline aligned: a mark on the baseline of a 112px
    // numeral hangs off the bottom-left corner looking detached.
    canvas.text({
      x,
      baseline: baselineFromCapTop(capTop, markSize),
      content: mark,
      size: markSize,
      fill: INK_2,
      weight: WEIGHT_SEMIBOLD,
    });
    x += textWidth(mark, markSize, WEIGHT_SEMIBOLD) + 2;
  }
  canvas.text({
    x,
    baseline,
    content: integerRun,
    size,
    fill: INK,
    weight: WEIGHT_SEMIBOLD,
  });
  x += textWidth(integerRun, size, WEIGHT_SEMIBOLD);
  if (fractionDigits !== "") {
    // The fraction stays on the main baseline: raised cents read as a footnote mark.
    canvas.text({
      x,
      baseline,
      content: fractionDigits,
      size: fractionSize,
      fill: uniform ? INK : INK_2,
      weight: WEIGHT_SEMIBOLD,
    });
  }

  return capTop + size * CAP_HEIGHT;
}

/**
 * The largest step at which the whole assembly fits. It measures exactly what
 * `drawPrice` will draw -- otherwise the fit is computed for a different string
 * than the one that appears, which is how a price ends up one glyph past the margin.
 */
function fitPriceSize(mark: string, integer: string, fraction: string, uniform: boolean): number {
  const [integerRun, fractionDigits] = priceRuns(integer, fraction, uniform);
  for (const candidate of HERO_STEPS) {
    const fractionSize = uniform ? candidate : candidate * FRACTION_RATIO;
    const markSize = candidate * FRACTION_RATIO;
    const width =
      textWidth(mark, markSize, WEIGHT_SEMIBOLD) +
      textWidth(integerRun, candidate, WEIGHT_SEMIBOLD) +
      textWidth(fractionDigits, fractionSize, WEIGHT_SEMIBOLD);
    if (width <= CONTENT_WIDTH) {
      return candidate;
    }
  }
  return HERO_STEPS[HERO_STEPS.length - 1] ?? 60;
}

function drawChangeChip(canvas: Canvas, face: TokenFace, top: number): void {
  const color = directionColor(face);
  // The sign is printed as well as coloured: a red number with no minus is
  // indistinguishable from a green one in a greyscale screenshot.
  const label = `${rising(face) ? "+" : "-"}${f2(Math.abs(face.changePercent))}%`;
  const labelWidth = textWidth(label, SIZE_CAPTION, WEIGHT_SEMIBOLD);
  const triangleWidth = 9;
  const chipWidth = triangleWidth + 1 * GRID + labelWidth + 3 * GRID;
  const center = top + CHIP_HEIGHT / 2;

  canvas.rectOpacity(MARGIN, top, chipWidth, CHIP_HEIGHT, RADIUS_CHIP, color, AREA_OPACITY);

  const triangleX = MARGIN + 1.5 * GRID;
  canvas.path(triangle(triangleX, center, triangleWidth, rising(face)), color);

  canvas.text({
    x: triangleX + triangleWidth + 1 * GRID,
    baseline: baselineFromCenter(center, SIZE_CAPTION),
    content: label,
    size: SIZE_CAPTION,
    fill: color,
    weight: WEIGHT_SEMIBOLD,
  });

  canvas.text({
    x: MARGIN + chipWidth + 1.5 * GRID,
    baseline: baselineFromCenter(center, SIZE_EYEBROW),
    content: "24H",
    size: SIZE_EYEBROW,
    fill: INK_3,
    weight: WEIGHT_SEMIBOLD,
    tracking: TRACKING_EYEBROW,
  });
}

/** An equilateral-ish triangle pointing up or down, centred vertically on `cy`. */
function triangle(x: number, cy: number, width: number, up: boolean): string {
  const half = width / 2;
  const height = width * 0.86;
  const base = up ? cy + height / 2 : cy - height / 2;
  const apex = up ? cy - height / 2 : cy + height / 2;
  return `M${f2(x)} ${f2(base)}L${f2(x + half)} ${f2(apex)}L${f2(x + width)} ${f2(base)}Z`;
}

function drawSparkline(canvas: Canvas, face: TokenFace): void {
  if (face.series.length < 2) {
    // Not an error: a freshly configured card has no history yet. A hairline reads
    // as "no series" without pretending to be a flat price.
    canvas.rect(MARGIN, SPARKLINE_TOP + SPARKLINE_HEIGHT - 1, CONTENT_WIDTH, 1, HAIRLINE);
    return;
  }

  let minimum = Number.MAX_VALUE;
  let maximum = -Number.MAX_VALUE;
  for (const value of face.series) {
    minimum = Math.min(minimum, value);
    maximum = Math.max(maximum, value);
  }
  const span = Math.max(maximum - minimum, Number.EPSILON);
  const plotHeight = SPARKLINE_HEIGHT - 2 * SPARKLINE_PADDING;
  const step = CONTENT_WIDTH / (face.series.length - 1);

  const point = (index: number, value: number): [number, number] => {
    // A perfectly flat series would otherwise sit on the top edge, because every
    // value equals the maximum; centre it instead.
    const normalized = maximum - minimum <= Number.EPSILON ? 0.5 : (value - minimum) / span;
    return [
      MARGIN + index * step,
      SPARKLINE_TOP + SPARKLINE_PADDING + (1 - normalized) * plotHeight,
    ];
  };

  let stroke = "";
  face.series.forEach((value, index) => {
    const [x, y] = point(index, value);
    stroke += `${index === 0 ? "M" : "L"}${f2(x)} ${f2(y)}`;
  });

  const color = directionColor(face);
  const bottom = SPARKLINE_TOP + SPARKLINE_HEIGHT;
  const area = `${stroke}L${f2(MARGIN + CONTENT_WIDTH)} ${f2(bottom)}L${f2(MARGIN)} ${f2(bottom)}Z`;
  canvas.filledPathOpacity(area, color, AREA_OPACITY);
  canvas.strokedPath(stroke, color, SPARKLINE_STROKE);

  // The newest point gets a marker, so the eye lands on "now" rather than on
  // whichever peak happens to be tallest.
  const lastIndex = face.series.length - 1;
  const [x, y] = point(lastIndex, face.series[lastIndex] ?? 0);
  canvas.circle(x, y, SPARKLINE_STROKE + 2, GROUND);
  canvas.circle(x, y, SPARKLINE_STROKE + 0.5, color);
}

function drawRange(canvas: Canvas, face: TokenFace): void {
  const labelTop = 300;
  const mark = face.currencyMark;
  const baseline = baselineFromCapTop(labelTop, SIZE_CAPTION);

  canvas.text({
    x: MARGIN,
    baseline,
    content: `L ${mark}${compactPrice(face.low)}`,
    size: SIZE_CAPTION,
    fill: INK_3,
    weight: WEIGHT_REGULAR,
  });
  canvas.text({
    x: CANVAS_WIDTH - MARGIN,
    baseline,
    content: `H ${mark}${compactPrice(face.high)}`,
    size: SIZE_CAPTION,
    fill: INK_3,
    weight: WEIGHT_REGULAR,
    anchor: "end",
  });

  // Where the current price sits inside the window's range: the one fact the
  // sparkline does not state plainly.
  const trackY = labelTop + SIZE_CAPTION * CAP_HEIGHT + 1.5 * GRID;
  canvas.roundedRect(MARGIN, trackY, CONTENT_WIDTH, 3, 1.5, HAIRLINE);
  const span = face.high - face.low;
  if (span > Number.EPSILON) {
    const fraction = Math.min(1, Math.max(0, (face.price - face.low) / span));
    canvas.circle(MARGIN + fraction * CONTENT_WIDTH, trackY + 1.5, 4.5, directionColor(face));
  }
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

export const token: FaceDefinition = {
  kind: "token",
  label: "Token price",
  fields: [
    { type: "text", key: "coin_id", label: "Coin ID", placeholder: "solana" },
    { type: "text", key: "currency", label: "Currency", placeholder: "usd", default: "usd" },
  ],
  async render(settings) {
    return renderToken(await fetchToken(settings, fetchText));
  },
};
